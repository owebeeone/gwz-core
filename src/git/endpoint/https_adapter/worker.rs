//! The HTTPS adapter's worker thread: it drives a small runtime of its own (Hyper's connection drivers, TLS, the
//! exchanges' tasks and the helper lookups run on it), owns the connector and every physical connection, and reports
//! to the adapter's outbox. Its one future is woken by the adapter's commands, the resources' own wakers (setup jobs,
//! connects, Hyper's driver ends, disposals) and the exchanges' tasks; nothing polls on a timer. It ends when it is
//! told to dispose (or its adapter is dropped), every connection is disposed and every helper process and native
//! conversation it started is confirmed gone.
use super::{
    Binding, Command, HttpsInputs, Scopes,
    exchange::{self, Common, Ended, Work},
    failure,
    pipe::Pipe,
};
use crate::git::endpoint::{
    https_auth::AuthOwner,
    https_connection::HttpConnector,
    https_worker::native::NativeRetained,
    shared_reservation::{ReservedConnector, ReservedResource},
    ssh_adapter::{Settle, Shared},
    ssh_pool::{Connector, Resource},
};
use gwz_connection::{Completion, ConnectCommand, SharedProcessSafety};
use gwz_transport::{
    pool::ConnectionId,
    protocol::{Disposition, ErrorCode, Failure},
};
use std::{
    collections::BTreeMap,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    sync::{Semaphore, mpsc::UnboundedReceiver},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

/// The runtime's worker threads: Hyper's drivers and the exchanges are I/O bound; a TLS handshake's CPU is short.
const RUNTIME_WORKERS: usize = 2;
/// The connector's blocking setup jobs (name resolution, TLS configuration) at once, as the endpoint's pool has.
const SETUP_SLOTS: usize = 8;
/// The adapter's helper lookups at once, as the endpoint's client has.
const HELPER_LOOKUPS: usize = 8;
/// How often a disposal looks again at retained helper processes and native records.
const SETTLE_STEP: Duration = Duration::from_millis(20);

type Http = ReservedConnector<HttpConnector>;
enum Report {
    Connected(Failure),
    Closed,
    IdleClosed,
}
enum Phase {
    Connecting,
    Idle,
    /// An exchange's task runs on the connection.
    Active(Box<Active>),
    /// The exchange ended discarded: the connection waits for the pool's close.
    Broken,
    Disposing(Report),
}
struct Active {
    task: JoinHandle<Ended>,
    settle: Arc<Settle>,
    pipe: Arc<Pipe>,
}
struct Entry {
    resource: ReservedResource<HttpConnector>,
    phase: Phase,
}

struct Worker {
    connector: Http,
    epoch: Instant,
    common: Arc<Common>,
    shared: Arc<Shared>,
    scopes: Scopes,
    entries: BTreeMap<ConnectionId, Entry>,
    /// Exchanges started, which number the scope records.
    exchanges: u64,
    disposing: bool,
    /// The adapter's command channel has closed.
    closed: bool,
    /// The wait for retained helper processes and native records, once disposal has begun.
    settling: Option<JoinHandle<()>>,
}

pub(super) fn run(
    inputs: HttpsInputs,
    safety: SharedProcessSafety,
    mut commands: UnboundedReceiver<Command>,
    shared: Arc<Shared>,
    scopes: Scopes,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(RUNTIME_WORKERS)
        .thread_name("gwz-https-adapter-worker")
        .enable_all()
        .build();
    let Ok(runtime) = runtime else {
        // No runtime: nothing physical can be made. Every command is refused until disposal.
        while let Some(command) = commands.blocking_recv() {
            if matches!(command, Command::Dispose) {
                break;
            }
            command.refuse(&shared);
        }
        shared.disposed();
        return;
    };
    runtime.block_on(async move {
        let HttpsInputs {
            tls,
            helper_slots,
            native,
            authority,
            settings,
        } = inputs;
        let epoch = Instant::now();
        let connector = HttpConnector::new(
            tls,
            epoch,
            Arc::new(Semaphore::new(SETUP_SLOTS)),
            authority.supervisor().clone(),
        );
        let worker = Worker {
            connector: ReservedConnector::new(connector, authority),
            epoch,
            common: Arc::new(Common {
                settings,
                owner: AuthOwner::new(helper_slots),
                helpers: Arc::new(Semaphore::new(HELPER_LOOKUPS)),
                native,
                safety,
                retained: NativeRetained::default(),
                cancel: CancellationToken::new(),
            }),
            shared: shared.clone(),
            scopes,
            entries: BTreeMap::new(),
            exchanges: 0,
            disposing: false,
            closed: false,
            settling: None,
        };
        worker.serve(commands).await;
        shared.disposed();
    });
}

impl Worker {
    async fn serve(mut self, mut commands: UnboundedReceiver<Command>) {
        std::future::poll_fn(|cx| {
            while !self.closed {
                match commands.poll_recv(cx) {
                    Poll::Ready(Some(command)) => self.command(command),
                    Poll::Ready(None) => {
                        // The adapter is gone: the last resort.
                        self.closed = true;
                        self.dispose_all();
                    }
                    Poll::Pending => break,
                }
            }
            self.pass(cx);
            if self.disposing && self.entries.is_empty() {
                self.settled(cx)
            } else {
                Poll::Pending
            }
        })
        .await;
    }
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    fn command(&mut self, command: Command) {
        match command {
            Command::Connect(command) => self.connect(command),
            Command::CancelConnect(connection) => {
                if let Some(entry) = self.entries.get_mut(&connection)
                    && matches!(entry.phase, Phase::Connecting)
                {
                    entry.phase =
                        Phase::Disposing(Report::Connected(failure(ErrorCode::Cancelled)));
                }
            }
            Command::Close(connection) => match self.entries.get_mut(&connection) {
                Some(entry) => {
                    end_exchange(&mut entry.phase);
                    if !matches!(entry.phase, Phase::Disposing(_)) {
                        entry.phase = Phase::Disposing(Report::Closed);
                    }
                }
                // Disposed already (lost while idle, a lease having won the race): the close is confirmed at once.
                None => self.shared.push(Completion::Closed { connection }),
            },
            Command::Start { connection, work } => self.start(connection, *work),
            Command::Dispose => self.dispose_all(),
        }
    }
    fn connect(&mut self, command: ConnectCommand) {
        if self.disposing {
            return self.shared.push(Completion::Connected {
                connection: command.connection,
                result: Err(failure(ErrorCode::Unavailable)),
            });
        }
        let settings = &self.common.settings;
        let deadline = (settings.connect_timeout_ms != 0)
            .then(|| self.now().saturating_add(settings.connect_timeout_ms));
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
            Err(_) => self.shared.push(Completion::Connected {
                connection: command.connection,
                result: Err(failure(ErrorCode::Io)),
            }),
        }
    }
    fn start(&mut self, connection: ConnectionId, mut work: Work) {
        let Some(entry) = self.entries.get_mut(&connection) else {
            return work.fail(ErrorCode::Io);
        };
        if !matches!(entry.phase, Phase::Idle) {
            return work.fail(ErrorCode::Io);
        }
        let live = entry.resource.reusable();
        let Some(resource) = entry.resource.inner_mut().filter(|_| live) else {
            work.fail(ErrorCode::Io);
            entry.phase = Phase::Broken;
            return;
        };
        let Some(physical) = resource.connection.clone() else {
            work.fail(ErrorCode::Io);
            entry.phase = Phase::Broken;
            return;
        };
        // The exchange is given the authentication that last bound this connection, and the repository it was for.
        let scope = self.scopes.of(connection);
        work.prior = scope.map_or(Binding::Anonymous, |scope| scope.binding);
        work.authenticated = scope.and_then(|scope| scope.repository);
        // Leased: its idle watch stands down until a reusable end makes it idle again.
        resource
            .reusable
            .store(false, std::sync::atomic::Ordering::Release);
        let (settle, pipe) = (work.settle.clone(), work.pipe.clone());
        let task = tokio::spawn(exchange::run(physical, work, self.common.clone()));
        entry.phase = Phase::Active(Box::new(Active { task, settle, pipe }));
    }
    fn dispose_all(&mut self) {
        if !self.disposing {
            self.disposing = true;
            // In-flight helper lookups end; their processes are retained by the owner until reaped.
            self.common.cancel.cancel();
            self.common.owner.cancel();
        }
        for entry in self.entries.values_mut() {
            end_exchange(&mut entry.phase);
            if !matches!(entry.phase, Phase::Disposing(_)) {
                entry.phase = Phase::Disposing(if matches!(entry.phase, Phase::Connecting) {
                    Report::Connected(failure(ErrorCode::Cancelled))
                } else {
                    Report::Closed
                });
            }
        }
    }
    /// One pass over every connection. A connection that changes phase is stepped again at once, so that its new
    /// phase's waker is registered in the same pass.
    fn pass(&mut self, cx: &mut Context<'_>) {
        let mut done = Vec::new();
        for (id, entry) in &mut self.entries {
            for _ in 0..3 {
                let next = step(
                    *id,
                    entry,
                    cx,
                    &self.shared,
                    &self.scopes,
                    &mut self.exchanges,
                );
                match next {
                    Step::Same => break,
                    Step::Changed => {}
                    Step::Disposed => {
                        done.push(*id);
                        break;
                    }
                }
            }
        }
        for id in done {
            // Destruction precedes the report, which lets the pool free the capacity.
            let Some(entry) = self.entries.remove(&id) else {
                continue;
            };
            let Phase::Disposing(report) = entry.phase else {
                continue;
            };
            drop(entry.resource);
            self.scopes.forget(id);
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
    /// `Ready` once every helper process the adapter started is reaped and every native record confirmed.
    fn settled(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let task = self.settling.get_or_insert_with(|| {
            let common = self.common.clone();
            tokio::spawn(async move {
                loop {
                    common
                        .owner
                        .reap_pending(tokio::time::Instant::now() + SETTLE_STEP)
                        .await;
                    if common.owner.pending_cleanup_count() == 0 && common.retained.pending() == 0 {
                        return;
                    }
                    tokio::time::sleep(SETTLE_STEP).await;
                }
            })
        });
        Pin::new(task).poll(cx).map(|_| ())
    }
}

enum Step {
    Same,
    Changed,
    Disposed,
}
fn step(
    id: ConnectionId,
    entry: &mut Entry,
    cx: &mut Context<'_>,
    shared: &Shared,
    scopes: &Scopes,
    exchanges: &mut u64,
) -> Step {
    match &mut entry.phase {
        Phase::Connecting => match entry.resource.poll_connected(cx) {
            Poll::Ready(Ok(identity)) => {
                entry.phase = Phase::Idle;
                scopes.connected(id);
                shared.push(Completion::Connected {
                    connection: id,
                    result: Ok(identity),
                });
                Step::Changed
            }
            Poll::Ready(Err(failure)) => {
                entry.phase = Phase::Disposing(Report::Connected(failure));
                Step::Changed
            }
            Poll::Pending => Step::Same,
        },
        // Idle liveness: Hyper's driver ended while no exchange ran (the peer closed or reset it, or sent bytes
        // nothing asked for).
        Phase::Idle => match entry.resource.poll_idle_lost(cx) {
            Poll::Ready(()) => {
                entry.phase = Phase::Disposing(Report::IdleClosed);
                Step::Changed
            }
            Poll::Pending => Step::Same,
        },
        Phase::Active(active) => {
            let ended = match Pin::new(&mut active.task).poll(cx) {
                Poll::Pending => return Step::Same,
                Poll::Ready(ended) => ended.ok(),
            };
            let (settle, pipe) = (active.settle.clone(), active.pipe.clone());
            *exchanges += 1;
            let reusable = match ended {
                Some(Ended {
                    disposition,
                    binding,
                    repository,
                }) => {
                    scopes.bound(id, binding, *exchanges, repository);
                    disposition == Disposition::Reusable
                }
                // The task panicked or was cancelled: nothing about the connection is proven.
                None => false,
            };
            let resource = entry.resource.inner_mut();
            if reusable && let Some(resource) = resource {
                resource
                    .reusable
                    .store(true, std::sync::atomic::Ordering::Release);
                entry.phase = Phase::Idle;
                settle.end(Disposition::Reusable);
            } else {
                // A task that ended without a word (it panicked) leaves its client a failure to read.
                pipe.end(Err(ErrorCode::Io));
                entry.phase = Phase::Broken;
                settle.end(Disposition::Discarded);
            }
            Step::Changed
        }
        Phase::Broken => Step::Same,
        Phase::Disposing(_) => match entry.resource.poll_dispose(cx, true) {
            Poll::Ready(Ok(())) => Step::Disposed,
            Poll::Ready(Err(_)) => {
                // A failed disposal is tried again.
                cx.waker().wake_by_ref();
                Step::Same
            }
            Poll::Pending => Step::Same,
        },
    }
}

/// A running exchange ends because its connection goes: it is discarded, and its task is cancelled.
fn end_exchange(phase: &mut Phase) {
    if let Phase::Active(active) = phase {
        active.task.abort();
        active.pipe.end(Err(ErrorCode::Cancelled));
        active.settle.end(Disposition::Discarded);
        *phase = Phase::Broken;
    }
}
