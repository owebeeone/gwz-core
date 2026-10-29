//! The thin adapter that gives `gwz_connection::remote` its lane (plan step 2.2c; gwz-dev
//! `GwzSplitCommandChannel.md` §3.3 and §3.4): a carrier end, seen as the `(request, Envelope)` messages the remote
//! module speaks, plus what each end knows of its session: its clock, the executor its driver runs on and, for the
//! core's end, the live-call registry. The stream carrier frames the messages as the generated `TransportLaneFrame`.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The core end's `CoreBinding` is the session's lane and its live-call registry (`Service::is_live`) | §3.4 "Construction" |
//! | A message that did not arrive on the connection this binding was made for is discarded before the remote module sees it | contract §6 |
//! | The client end's `ClientSession` is the same lane; `serve_bindings` builds the receiver over it | §3.4 walkthrough |
//!
//! The messages are the ones the carrier already moves (`Carrier::try_send`, `register`); the lane takes the transport
//! class only (`try_recv_transport`), opens the core's bindings as their `bind` goes out (`bind_open`), retires them
//! (`bind_retire`) and reports the ones the carrier faulted (`bind_fault`). The client's end admits the core's `bind`
//! (`accept_binds`). These four are the seams step 2.2c added to the carrier trait; `FrameSink`, `FrameRouter` and
//! `FrameKey` are unchanged.
use super::{
    Attachment,
    carrier::{BindingRefused, Carrier, CarrierBinding, SendRefused},
};
use gwz_connection::{
    Clock, HostHandle, HostNamespace, Progress,
    remote::{
        ClientSession, CoreBinding, Invocations, Lane, LaneEnded, LaneSend, Receiver,
        serve_bindings,
    },
};
use gwz_transport::protocol::MessageKind;
use std::{sync::Arc, task::Waker};

/// What the core end knows of its calls, from the served session's registry (`Service::is_live`) and the request each
/// call carried (its `TransportOptions.endpoint_path_base`): supplied by the composition that builds the session.
pub(crate) trait Calls: Send + Sync + 'static {
    fn is_live(&self, request_id: &str) -> bool;
    fn endpoint_path_base(&self, request_id: &str) -> Option<String>;
}

/// One end of a carrier as the remote module's lane.
pub(crate) struct CarrierLane {
    carrier: Arc<dyn Carrier>,
    bound: CarrierBinding,
    clock: Arc<dyn Clock>,
    progress: Arc<dyn Progress>,
    core: Option<(HostNamespace, Arc<dyn Calls>)>,
}
impl CarrierLane {
    /// The core's end of a served session.
    pub(crate) fn core(
        carrier: Arc<dyn Carrier>,
        clock: Arc<dyn Clock>,
        progress: Arc<dyn Progress>,
        namespace: HostNamespace,
        calls: Arc<dyn Calls>,
    ) -> Arc<Self> {
        Arc::new(Self {
            bound: carrier.binding(),
            carrier,
            clock,
            progress,
            core: Some((namespace, calls)),
        })
    }
    /// The client's end. It admits the core's `bind` for a binding it has not opened (GwzSplitCommandChannel 3.3): the
    /// receiver above it refuses a `bind` that names no call of its own, and retires the binding.
    pub(crate) fn client(
        carrier: Arc<dyn Carrier>,
        clock: Arc<dyn Clock>,
        progress: Arc<dyn Progress>,
    ) -> Arc<Self> {
        carrier.accept_binds();
        Arc::new(Self {
            bound: carrier.binding(),
            carrier,
            clock,
            progress,
            core: None,
        })
    }
}
impl Lane for CarrierLane {
    fn send(&self, message: Attachment) -> Result<(), LaneSend> {
        // The core's end opens a binding as its `bind` goes out, so the client's answers are routed to it. A retry of a
        // `bind` the lane turned away finds the binding open already.
        if self.core.is_some() && message.1.kind == MessageKind::Bind {
            match self.carrier.bind_open(&message.1.session_id) {
                Ok(()) | Err(BindingRefused::Name) => {}
                Err(BindingRefused::TooMany) => return Err(LaneSend::Full(Box::new(message))),
                Err(BindingRefused::Closed(_)) => return Err(LaneSend::Closed(Box::new(message))),
            }
        }
        self.carrier
            .try_send(message)
            .map_err(|refused| match refused {
                SendRefused::Full(message) => LaneSend::Full(message),
                SendRefused::Closed(message, _) => LaneSend::Closed(message),
            })
    }
    fn recv(&self) -> Result<Option<Attachment>, LaneEnded> {
        loop {
            // The transport take: the registry's frames (tags 1 to 8) are the operation service's, and an end that carries
            // both classes ends if the lane takes one.
            match self.carrier.try_recv_transport() {
                Ok(Some(arrival)) => {
                    // An arrival on another connection is discarded here and makes nothing.
                    if let Some(message) = arrival.into_message_for(&self.bound) {
                        return Ok(Some(message));
                    }
                }
                Ok(None) => return Ok(None),
                Err(_) => return Err(LaneEnded),
            }
        }
    }
    fn register(&self, waker: &Waker) {
        self.carrier.register(waker);
    }
    fn retire(&self, session: &str) {
        self.carrier.bind_retire(session);
    }
    fn faulted(&self, session: &str) -> bool {
        self.carrier.bind_fault(session).is_some()
    }
}
impl CoreBinding for CarrierLane {
    fn namespace(&self) -> HostNamespace {
        self.core
            .as_ref()
            .map_or(HostNamespace::new(0), |(namespace, _)| *namespace)
    }
    fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }
    fn progress(&self) -> Arc<dyn Progress> {
        self.progress.clone()
    }
    fn call_is_live(&self, request_id: &str) -> bool {
        self.core
            .as_ref()
            .is_some_and(|(_, calls)| calls.is_live(request_id))
    }
    fn endpoint_path_base(&self, request_id: &str) -> Option<String> {
        self.core
            .as_ref()
            .and_then(|(_, calls)| calls.endpoint_path_base(request_id))
    }
}
impl ClientSession for CarrierLane {
    fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }
    fn progress(&self) -> Arc<dyn Progress> {
        self.progress.clone()
    }
}

/// The client's receiver over its carrier end (`serve_bindings(&host, &session)` of the walkthrough, §3.4). Its driver
/// runs on the end's executor.
pub(crate) fn serve(
    host: &HostHandle,
    lane: Arc<CarrierLane>,
    invocations: Arc<dyn Invocations>,
) -> Receiver {
    serve_bindings(host, lane, invocations)
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
