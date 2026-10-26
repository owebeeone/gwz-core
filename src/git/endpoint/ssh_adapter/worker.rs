//! The SSH adapter's worker thread: it owns the connector and every physical connection, does all their I/O and
//! reports to the adapter's outbox. It parks between passes; commands, the resources' own wakers (setup jobs, idle
//! watches, the channel pumps' streams) and the exchanges' clients unpark it, and a pass is never longer than its
//! bounded work. It ends when it is told to dispose (or its adapter is dropped) and every connection is disposed.
use super::{AdapterSettings, Command, Exchange, Shared};
use crate::git::endpoint::{
    ssh_pool::{Connector, Resource},
    ssh_worker::{ChannelResource, ThreadWake},
};
use gwz_connection::{Completion, ConnectCommand};
use gwz_transport::{
    pool::ConnectionId,
    protocol::{Disposition, Effect, ErrorCode, Failure, Scheme},
};
use std::{
    collections::BTreeMap,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::pin,
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

/// The longest a parked worker waits before a pass of its own (a safety net; every wake source unparks it).
const PARK: Duration = Duration::from_millis(50);
/// Messages moved each way per exchange per pass, so that one exchange cannot starve the others.
const BATCH: usize = 8;

pub(super) fn failure(code: ErrorCode) -> Failure {
    Failure {
        detail: None,
        setup_cause: None,
        facts: None,
        code,
        effect: Effect::None,
    }
}

/// What a disposal reports once it is done.
enum Report {
    Connected(Failure),
    Closed,
    IdleClosed,
}
enum Phase {
    Connecting,
    Idle,
    /// An exchange runs on the connection's channel.
    Active(Box<Running>),
    /// The exchange failed: the connection waits for the pool's close.
    Broken,
    Disposing {
        report: Report,
        force: bool,
        deadline: u64,
    },
}
struct Running {
    exchange: Exchange,
    /// When the member's orderly end was first seen: the reclaim must come within the cleanup bound.
    close_deadline: Option<u64>,
}
struct Entry<R> {
    resource: R,
    phase: Phase,
}

struct Worker<C: Connector> {
    connector: C,
    origin: Instant,
    shared: Arc<Shared>,
    settings: AdapterSettings,
    entries: BTreeMap<ConnectionId, Entry<C::Resource>>,
    disposing: bool,
}

pub(super) fn run<C>(
    connector: C,
    origin: Instant,
    commands: Receiver<Command>,
    shared: Arc<Shared>,
    settings: AdapterSettings,
) where
    C: Connector,
    C::Resource: ChannelResource,
{
    let mut worker = Worker {
        connector,
        origin,
        shared,
        settings,
        entries: BTreeMap::new(),
        disposing: false,
    };
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        loop {
            match commands.try_recv() {
                Ok(command) => worker.command(command),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // The adapter is gone: the last resort, as fast as the resources allow.
                    worker.dispose_all(true);
                    break;
                }
            }
        }
        worker.pass(&mut cx);
        if worker.disposing && worker.entries.is_empty() {
            worker.shared.disposed();
            return;
        }
        thread::park_timeout(PARK);
    }
}

impl<C> Worker<C>
where
    C: Connector,
    C::Resource: ChannelResource,
{
    fn now(&self) -> u64 {
        self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    fn command(&mut self, command: Command) {
        let now = self.now();
        match command {
            Command::Connect(command) => self.connect(command, now),
            Command::CancelConnect(connection) => {
                if let Some(entry) = self.entries.get_mut(&connection)
                    && matches!(entry.phase, Phase::Connecting)
                {
                    entry.phase = disposing(
                        Report::Connected(failure(ErrorCode::Cancelled)),
                        false,
                        now,
                        &self.settings,
                    );
                }
            }
            Command::Close(connection) => match self.entries.get_mut(&connection) {
                Some(entry) => {
                    let force = end_exchange(&mut entry.phase);
                    if !matches!(entry.phase, Phase::Disposing { .. }) {
                        entry.phase = disposing(Report::Closed, force, now, &self.settings);
                    }
                }
                // Disposed already (lost while idle, a lease having won the race): the close is confirmed at once.
                None => self.shared.push(Completion::Closed { connection }),
            },
            Command::Start {
                connection,
                exchange,
            } => self.start(connection, *exchange),
            Command::Dispose => self.dispose_all(false),
        }
    }
    fn connect(&mut self, command: ConnectCommand, now: u64) {
        let refuse = |code| Completion::Connected {
            connection: command.connection,
            result: Err(failure(code)),
        };
        if self.disposing {
            return self.shared.push(refuse(ErrorCode::Unavailable));
        }
        // Plan step 1.3c serves HTTPS through its own adapter; this one is SSH's only.
        if command.key.scheme != Scheme::Ssh {
            return self.shared.push(refuse(ErrorCode::UnsupportedOperation));
        }
        let deadline = (self.settings.connect_timeout_ms != 0)
            .then(|| now.saturating_add(self.settings.connect_timeout_ms));
        let started = catch_unwind(AssertUnwindSafe(|| {
            self.connector
                .start(&command.key, &command.identity, deadline)
        }));
        match started {
            Ok(Ok(resource)) => {
                self.entries.insert(
                    command.connection,
                    Entry {
                        resource,
                        phase: Phase::Connecting,
                    },
                );
            }
            Ok(Err(failure)) => self.shared.push(Completion::Connected {
                connection: command.connection,
                result: Err(failure),
            }),
            // A start that unwound owns nothing (the Connector contract).
            Err(_) => self.shared.push(refuse(ErrorCode::Io)),
        }
    }
    fn start(&mut self, connection: ConnectionId, mut exchange: Exchange) {
        let now = self.now();
        let (Some(entry), Some((stream, endpoint))) =
            (self.entries.get_mut(&connection), exchange.channel.take())
        else {
            return fail(&exchange);
        };
        if !matches!(entry.phase, Phase::Idle) {
            return fail(&exchange);
        }
        exchange.peer.advance(now);
        endpoint.advance(now);
        // The endpoint side of the streams goes to the channel pump; the worker keeps the client's messages.
        let started =
            entry
                .resource
                .start_exchange(stream, endpoint, exchange.service, &exchange.path);
        match started {
            Ok(()) => {
                entry.phase = Phase::Active(Box::new(Running {
                    exchange,
                    close_deadline: None,
                }))
            }
            Err(_) => {
                fail(&exchange);
                entry.phase = Phase::Broken;
            }
        }
    }
    fn dispose_all(&mut self, force: bool) {
        self.disposing = true;
        let now = self.now();
        for entry in self.entries.values_mut() {
            let ended = end_exchange(&mut entry.phase);
            match &mut entry.phase {
                Phase::Disposing { force: prior, .. } => *prior |= force,
                phase => {
                    let report = if matches!(phase, Phase::Connecting) {
                        Report::Connected(failure(ErrorCode::Cancelled))
                    } else {
                        Report::Closed
                    };
                    *phase = disposing(report, force || ended, now, &self.settings);
                }
            }
        }
    }
    /// One bounded pass over every connection.
    fn pass(&mut self, cx: &mut Context<'_>) {
        let now = self.now();
        let mut done = Vec::new();
        for (id, entry) in &mut self.entries {
            match &mut entry.phase {
                Phase::Connecting => match entry.resource.poll_connected(cx) {
                    Poll::Ready(Ok(identity)) => {
                        entry.phase = Phase::Idle;
                        self.shared.push(Completion::Connected {
                            connection: *id,
                            result: Ok(identity),
                        });
                    }
                    Poll::Ready(Err(failure)) => {
                        entry.phase =
                            disposing(Report::Connected(failure), false, now, &self.settings)
                    }
                    Poll::Pending => {}
                },
                Phase::Idle => {
                    if entry.resource.poll_idle_lost(cx).is_ready() {
                        entry.phase = disposing(Report::IdleClosed, false, now, &self.settings);
                    }
                }
                Phase::Active(running) => {
                    if let Some(next) =
                        transfer(&mut entry.resource, running, cx, now, &self.settings)
                    {
                        entry.phase = next;
                    }
                }
                Phase::Broken => {}
                Phase::Disposing {
                    force, deadline, ..
                } => {
                    if !*force && now >= *deadline {
                        *force = true;
                    }
                    match entry.resource.poll_dispose(cx, *force) {
                        Poll::Ready(Ok(())) => done.push(*id),
                        // A failed graceful disposal is forced on the next pass.
                        Poll::Ready(Err(_)) => *force = true,
                        Poll::Pending => {}
                    }
                }
            }
        }
        for id in done {
            // Destruction precedes the report, which lets the pool free the capacity.
            let Some(entry) = self.entries.remove(&id) else {
                continue;
            };
            let Phase::Disposing { report, .. } = entry.phase else {
                continue;
            };
            drop(entry.resource);
            self.shared.push(match report {
                Report::Connected(failure) => Completion::Connected {
                    connection: id,
                    result: Err(failure),
                },
                Report::Closed => Completion::Closed { connection: id },
                Report::IdleClosed => Completion::IdleClosed { connection: id },
            });
        }
    }
}

fn disposing(report: Report, force: bool, now: u64, settings: &AdapterSettings) -> Phase {
    Phase::Disposing {
        report,
        force,
        deadline: now.saturating_add(settings.cleanup_timeout_ms),
    }
}
/// A running exchange ends because its connection goes: it is discarded. Whether the channel needs forcing.
fn end_exchange(phase: &mut Phase) -> bool {
    match phase {
        Phase::Active(running) => {
            fail(&running.exchange);
            true
        }
        Phase::Broken => true,
        _ => false,
    }
}
/// The exchange cannot run or goes on no further: its client sees the stream fail, and its end is `Discarded`.
fn fail(exchange: &Exchange) {
    exchange.peer.disconnect();
    exchange.settle.end(Disposition::Discarded);
}

/// One pass over a running exchange: the client's messages to the channel pump, the pump's I/O, and its messages to
/// the client. Returns the connection's next phase when the exchange is over: idle after a clean close and reclaim
/// (`Reusable`), broken after any failure (`Discarded`).
fn transfer<R: ChannelResource>(
    resource: &mut R,
    running: &mut Running,
    cx: &mut Context<'_>,
    now: u64,
    settings: &AdapterSettings,
) -> Option<Phase> {
    let Some(pump) = resource.pump() else {
        fail(&running.exchange);
        return Some(Phase::Broken);
    };
    let exchange = &running.exchange;
    let moved = (|| {
        pump.advance(now);
        exchange.peer.advance(now);
        for _ in 0..BATCH {
            match pin!(exchange.peer.next_message()).poll(cx) {
                Poll::Ready(Ok(Some(message))) => pump.deliver(message).map_err(|_| ())?,
                Poll::Ready(Ok(None)) | Poll::Pending => break,
                Poll::Ready(Err(_)) => return Err(()),
            }
        }
        pump.tick(cx, now).map_err(|_| ())?;
        for _ in 0..BATCH {
            match pump.poll_next_message(cx) {
                Poll::Ready(Ok(Some(message))) => exchange.peer.deliver(message).map_err(|_| ())?,
                Poll::Ready(Ok(None)) | Poll::Pending => break,
                Poll::Ready(Err(_)) => return Err(()),
            }
        }
        Ok(())
    })();
    if moved.is_err() || pump.retired() {
        pump.cancel();
        fail(exchange);
        return Some(Phase::Broken);
    }
    let finished = pump.finished();
    if running.exchange.settle.requested() && running.close_deadline.is_none() {
        running.close_deadline = Some(now.saturating_add(settings.cleanup_timeout_ms));
    }
    if finished && resource.reclaim() {
        running.exchange.settle.end(Disposition::Reusable);
        return Some(Phase::Idle);
    }
    if running
        .close_deadline
        .is_some_and(|deadline| now >= deadline)
    {
        fail(&running.exchange);
        return Some(Phase::Broken);
    }
    None
}
