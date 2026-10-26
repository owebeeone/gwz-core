//! The pump between a session's port and a carrier end (what `LocalLink` was, now behind the seam).
//!
//! One thread serves every pump it is given. Each pump moves a port's outgoing messages into its carrier end and the
//! end's arrivals into the port, in order, up to a bound per pass, and keeps the message it is moving across passes,
//! so that the change that lets it finish wakes the thread. The thread parks only when no pump moved anything.
//! Arrivals pass through the end's binding first: a message that did not arrive on the connection the pump was bound
//! to is discarded without touching the port.
use super::{
    Carrier, CarrierBinding, CarrierClosed, CarrierLimits, CloseCode, CloseReason, SendRefused,
    in_process,
};
use crate::{
    git::endpoint::ssh_worker::ThreadWake,
    model::ModelResult,
    transport_host::{Attachment, TransportPort, unavailable},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
    thread,
    time::Duration,
};

/// How long the thread parks when a pass moved nothing: the longest it can miss a deadline. Whatever gives a pass
/// work wakes it before that.
const PARK: Duration = Duration::from_millis(5);
/// Messages one pump moves in each direction per pass.
const PER_PASS: usize = 64;

type Take = Pin<Box<dyn Future<Output = ModelResult<Option<Attachment>>>>>;
type Give = Pin<Box<dyn Future<Output = ModelResult<()>>>>;

/// Why a link stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkEnd {
    /// The owner stopped the link, or dropped it.
    Stopped,
    /// A port finished: its session closed in order.
    PortClosed,
    /// A port failed to take or give a message.
    PortFailed,
    /// A carrier end ended, with its own reason.
    Carrier(CarrierClosed),
}

impl LinkEnd {
    /// The code the surviving carrier ends close with: `Normal` for an orderly end and never for a failure.
    pub(crate) fn code(self) -> CloseCode {
        match self {
            LinkEnd::Stopped | LinkEnd::PortClosed => CloseCode::Normal,
            LinkEnd::PortFailed => CloseCode::ProtocolFault,
            LinkEnd::Carrier(closed) => match closed.reason {
                CloseReason::Local(code) => code,
                CloseReason::Peer(CloseCode::Normal) => CloseCode::Normal,
                CloseReason::Peer(CloseCode::Dropped | CloseCode::Cancelled) => {
                    CloseCode::Cancelled
                }
                CloseReason::Peer(CloseCode::ProtocolFault)
                | CloseReason::PeerGone
                | CloseReason::Stream(_)
                | CloseReason::Stalled
                | CloseReason::Protocol(_) => CloseCode::ProtocolFault,
            },
        }
    }
}

/// A port, the carrier end it talks through, and the binding (if the end has bindings) whose arrivals are the port's.
pub(crate) struct Join {
    pub(crate) port: TransportPort,
    pub(crate) end: Arc<dyn Carrier>,
    pub(crate) binding: Option<String>,
}

/// One port joined to one carrier end.
struct PortPump {
    port: TransportPort,
    end: Arc<dyn Carrier>,
    bound: CarrierBinding,
    /// The binding whose arrivals this pump takes; the end's whole stream when `None`.
    scope: Option<String>,
    /// The port's next outgoing message, being awaited.
    take: Option<Take>,
    /// A message taken from the port that the carrier refused as full; sent first on the next pass.
    held: Option<Attachment>,
    /// An arrival being delivered to the port.
    give: Option<Give>,
}

impl PortPump {
    fn new(join: Join) -> Self {
        let Join {
            port,
            end,
            binding: scope,
        } = join;
        let bound = end.binding();
        Self {
            port,
            end,
            bound,
            scope,
            take: None,
            held: None,
            give: None,
        }
    }

    /// Moves what is ready in both directions; `Err` once either side has closed.
    fn pump(&mut self, cx: &mut Context<'_>) -> Result<bool, LinkEnd> {
        let outbound = self.outbound(cx)?;
        let inbound = self.inbound(cx)?;
        Ok(outbound || inbound)
    }

    /// Port to carrier.
    fn outbound(&mut self, cx: &mut Context<'_>) -> Result<bool, LinkEnd> {
        let mut moved = false;
        for _ in 0..PER_PASS {
            if let Some(message) = self.held.take() {
                match self.end.try_send(message) {
                    Ok(()) => moved = true,
                    Err(SendRefused::Full(message)) => {
                        self.held = Some(*message);
                        return Ok(moved);
                    }
                    Err(SendRefused::Closed(_, closed)) => return Err(LinkEnd::Carrier(closed)),
                }
            }
            let port = &self.port;
            let take = self.take.get_or_insert_with(|| {
                let port = port.clone();
                Box::pin(async move { port.next_message().await })
            });
            match take.as_mut().poll(cx) {
                Poll::Ready(Ok(Some(message))) => {
                    self.take = None;
                    self.held = Some(message);
                }
                Poll::Ready(Ok(None)) => return Err(LinkEnd::PortClosed),
                Poll::Ready(Err(_)) => return Err(LinkEnd::PortFailed),
                Poll::Pending => return Ok(moved),
            }
        }
        Ok(moved)
    }

    /// Carrier to port.
    fn inbound(&mut self, cx: &mut Context<'_>) -> Result<bool, LinkEnd> {
        let mut moved = false;
        for _ in 0..PER_PASS {
            if let Some(give) = self.give.as_mut() {
                match give.as_mut().poll(cx) {
                    Poll::Ready(Ok(())) => {
                        self.give = None;
                        moved = true;
                    }
                    Poll::Ready(Err(_)) => return Err(LinkEnd::PortFailed),
                    Poll::Pending => return Ok(moved),
                }
            }
            let arrival = match &self.scope {
                Some(binding) => self.end.try_recv_binding(binding),
                None => self.end.try_recv(),
            };
            match arrival {
                Ok(Some(arrival)) => {
                    moved = true;
                    // Only what arrived on the connection this pump is bound to reaches the port.
                    if let Some(message) = arrival.into_message_for(&self.bound) {
                        let port = self.port.clone();
                        self.give = Some(Box::pin(async move { port.deliver(message).await }));
                    }
                }
                Ok(None) => return Ok(moved),
                Err(closed) => return Err(LinkEnd::Carrier(closed)),
            }
        }
        Ok(moved)
    }
}

/// The thread that pumps ports to carrier ends.
pub(crate) struct CarrierLink {
    stop: Arc<AtomicBool>,
    thread: thread::Thread,
    ended: Arc<Mutex<Option<LinkEnd>>>,
}

impl CarrierLink {
    /// The default carrier: the two ports joined through an in-process pair, one thread pumping both.
    pub(crate) fn in_process(core: TransportPort, peer: TransportPort) -> ModelResult<Self> {
        let (core_end, peer_end) = in_process(CarrierLimits::DEFAULT)
            .map_err(|_| unavailable("local forwarding unavailable"))?;
        let ends: [Arc<dyn Carrier>; 2] = [Arc::new(core_end), Arc::new(peer_end)];
        let [core_end, peer_end] = ends;
        Self::start(vec![(core, core_end), (peer, peer_end)])
    }

    /// Starts the pumping thread over `joins`, each a port and the carrier end it talks through.
    pub(crate) fn start(joins: Vec<(TransportPort, Arc<dyn Carrier>)>) -> ModelResult<Self> {
        Self::start_joined(
            joins
                .into_iter()
                .map(|(port, end)| Join {
                    port,
                    end,
                    binding: None,
                })
                .collect(),
        )
    }

    /// Starts the pumping thread over `joins`, each a port, a carrier end and, where the end has bindings, the binding
    /// whose arrivals are the port's.
    pub(crate) fn start_joined(joins: Vec<Join>) -> ModelResult<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let ended = Arc::new(Mutex::new(None));
        let record = ended.clone();
        let link = thread::Builder::new()
            .name("gwz-placement-local".into())
            .spawn(move || {
                // Each pump's waiting futures wake this thread when a mux changes, and each carrier end wakes it when a
                // message arrives or room appears; the timeout only covers a wake that never comes.
                let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
                let mut cx = Context::from_waker(&waker);
                let mut pumps: Vec<PortPump> = joins.into_iter().map(PortPump::new).collect();
                for pump in &pumps {
                    pump.end.register(&waker);
                    if let Some(binding) = &pump.scope {
                        pump.end.register_binding(binding, &waker);
                    }
                }
                let mut why = LinkEnd::Stopped;
                while !stopped.load(Ordering::Acquire) {
                    let mut moved = false;
                    for pump in &mut pumps {
                        match pump.pump(&mut cx) {
                            Ok(progress) => moved |= progress,
                            Err(end) => {
                                why = end;
                                stopped.store(true, Ordering::Release);
                                break;
                            }
                        }
                    }
                    if !moved && !stopped.load(Ordering::Acquire) {
                        thread::park_timeout(PARK);
                    }
                }
                *record.lock().unwrap_or_else(PoisonError::into_inner) = Some(why);
                for pump in pumps {
                    pump.end.close(why.code());
                    let port = pump.port.clone();
                    drop(pump);
                    port.disconnect();
                }
            })
            .map_err(|_| unavailable("local forwarding unavailable"))?;
        Ok(Self {
            stop,
            thread: link.thread().clone(),
            ended,
        })
    }

    /// Why the link stopped, once it has.
    pub(crate) fn ended(&self) -> Option<LinkEnd> {
        *self.ended.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

impl Drop for CarrierLink {
    fn drop(&mut self) {
        self.stop();
    }
}
