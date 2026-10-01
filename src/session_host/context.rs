//! The host context, the session context and `open` (session plan CS1.4,
//! extended by CS1.9), for the core session contract (gwz-dev
//! `dev-docs/GwzCoreSessionDesign.md`) as the reuse and server designs amend it.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The driver creates a `HostContext` and passes it to every session it opens; they share it, and no static holds one | §2, §5.6 |
//! | It holds one supervisor, gwz-session-host's: its thread starts with its first job and stops once the host context has been shut down or dropped and its jobs have finished, and a job whose poll panics is quarantined, never polled again and dropped when the thread stops | §5.6; reuse design §7; crate map §2, §6 step 4 |
//! | `shutdown` disposes what the members hold within one cleanup bound, 5 s, and reports what remains: the jobs still running at the bound, which the supervisor polls to their end, and the quarantined jobs | reuse design §7 "Host context and server shutdown", and §13's §5.6 bullet (CS:308) |
//! | The report's `peer_cleanup_confirmed` is false: no member has a peer before CS3.7's endpoint registry, so no peer cleanup occurred | §8's `(0, false)` |
//! | A later `shutdown`, from any handle, returns the first one's report and disposes nothing again; a concurrent one waits for the first, so it too returns within the bound | reuse design §7; §8's repeated close |
//! | Once `shutdown` has begun, `supervise` refuses a job, dropping it unpolled, and `open` refuses the host context with `invalid_request`, before any effect | reuse design §7; §9 |
//! | Dropping a host context without `shutdown` disposes the same way, without waiting, and reports nothing; after `shutdown`, the drop disposes nothing more | reuse design §7 and §13 (CS:308) |
//! | `open` validates the limits, refusing with `invalid_request` before any effect, then creates the session context on the calling thread | §1, §5.6, §9 |
//! | The session context holds the endpoint environment, the limits, the off switch's value and the host context; a worker reaches it only through its gate | §5.6, O8, O9 |
//! | `transport_off` comes only from `open`'s options: nothing derives it from the snapshot or from the process's own environment or configuration | §5.6 as server design §8 amends it; server design §5 "The off switch's value" |
//! | `open` returns the client end of the session's in-process channel: gwz-session-channel's pair, with the limits' outstanding calls on the call lane and their control reserve on the control lane; the host's end waits in it for CS2.2's host | §3, §9; session plan CS1.2; crate map §6 step 3 |
//!
//! Later steps add members without changing these signatures. The host
//! context gains the SSH setup supervisor's jobs and budgets (CS3.5), the
//! HTTPS helper slots (CS3.6), the member lock manager (CS3.9), the workspace
//! registry (CS2.9), and the endpoint registry (CS3.7), whose disposal joins
//! `shutdown`'s one bound. The session context gains the transport timeouts
//! (CS3.8), the operation table (CS2.6) and the `diff.output` and `log.output`
//! registries (CS2.11). `SessionOptions` is `non_exhaustive`, so a later
//! field breaks no caller.

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gwz_session_channel::{InProcessEnd, pair};
use gwz_session_contract::{Closed, Frame, FrameSink, FrameSource, Lane, SendError};
use gwz_session_host::{Limits, SuperviseError, SupervisedJob, Supervisor, validate_limits};

use super::environment::EnvironmentSnapshot;
use crate::model::{ErrorCode, ModelError, ModelResult};

/// What the sessions of one driver share (§5.6): the driver creates it and
/// passes it to every session it opens.
///
/// A clone is another handle to the same host context. It is dropped when the
/// driver's handles and every session using it are gone. At its end the
/// driver calls `shutdown`, which disposes what it holds within the cleanup
/// bound and reports what remains; dropping it without that disposes the same
/// way, without waiting or reporting. gwz-cli creates one at startup, and the
/// Python bridge one per process. Core keeps none in a static.
#[derive(Clone)]
pub struct HostContext {
    shared: Arc<HostShared>,
}

struct HostShared {
    /// Its drop is the host context's disposal without `shutdown`: no new
    /// job, and the thread stops once its jobs have finished, without a wait.
    supervisor: Supervisor,
    /// Set as `shutdown` begins; `open` then refuses the host context.
    shut_down: AtomicBool,
    /// The first `shutdown`'s report, which later calls return. That call
    /// holds the lock while it disposes, so a concurrent call waits for it.
    report: Mutex<Option<ShutdownReport>>,
}

/// The cleanup bound within which `shutdown` disposes the host context's
/// members (reuse design §7: "within the 5 s cleanup bound"). A host process
/// bounds its own waits for its operations with it too: gwz-py's close,
/// cancel and exit (gwz-py `dev-docs/GwzPyPerOperationTransportDesign.md`
/// §2.6).
pub const CLEANUP_BOUND: Duration = Duration::from_secs(5);

/// What a host context's `shutdown` leaves behind (reuse design §7). It
/// carries the two facts of the contract's §13 `CleanupReport`, so a driver
/// can add it to its session's close report (§8), as gwz-cli does at command
/// end (session plan CS6.6).
///
/// `non_exhaustive`: core creates it, and a driver reads its fields.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ShutdownReport {
    /// The local work the host context still owned when `shutdown` returned:
    /// its jobs still running at the cleanup bound, which the supervisor
    /// polls to their end, and its quarantined jobs, whose finish is never
    /// assumed. Zero when everything was disposed within the bound.
    pub pending_local_work: u32,
    /// Whether a peer confirmed its cleanup. False until the endpoint
    /// registry (CS3.7) gives the host context a peer: like the close report
    /// of a session that ran no network operation, `(0, false)` says that no
    /// peer cleanup occurred (§8).
    pub peer_cleanup_confirmed: bool,
}

impl HostContext {
    /// Creates a host context. It starts no thread until it has a job.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(HostShared {
                supervisor: Supervisor::new(),
                shut_down: AtomicBool::new(false),
                report: Mutex::new(None),
            }),
        }
    }

    /// Shuts the host context down at the driver's end (reuse design §7). It
    /// disposes what the host context's members hold within one cleanup
    /// bound of 5 seconds and returns a report of what remains. It returns
    /// within the bound even when a job never finishes: that job stays with
    /// the supervisor, which polls it to its end.
    ///
    /// Once it has begun, `open` refuses the host context and no new job
    /// starts. A later call, from any handle, returns the same report and
    /// disposes nothing again, and a concurrent call waits for the first, so
    /// each call returns within the bound. Dropping the host context without
    /// `shutdown` disposes the same way, without waiting or reporting; after
    /// `shutdown`, the drop disposes nothing more.
    pub fn shutdown(&self) -> ShutdownReport {
        self.shared.shutdown(CLEANUP_BOUND)
    }

    /// Hands `job` to the supervisor, starting its thread with the first job.
    /// If the thread cannot be created, or the host context has been shut
    /// down, the job is dropped unpolled, so a caller registers a job before
    /// starting the work it owns, and no work starts that nothing cleans up.
    #[allow(dead_code, reason = "CS3.5 moves the SSH setup jobs onto it")]
    pub(crate) fn supervise(&self, job: Box<dyn SupervisedJob>) -> io::Result<()> {
        self.shared
            .supervisor
            .supervise(job)
            .map_err(|error| match error {
                SuperviseError::ShutDown => io::Error::other("the host context has been shut down"),
                SuperviseError::Spawn(error) => error,
            })
    }
}

impl HostShared {
    /// `shutdown` within `bound`.
    fn shutdown(&self, bound: Duration) -> ShutdownReport {
        let mut report = self.report.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(done) = report.as_ref() {
            return done.clone();
        }
        self.shut_down.store(true, Ordering::Release);
        // Every member disposes within the one bound. CS3.7 adds the endpoint
        // registry's disposal before the supervisor's, whose jobs then hold
        // what a disposal leaves unfinished, passes the supervisor what
        // remains of the bound, and adds what remains of the registry.
        let remaining = self.supervisor.shutdown(bound);
        let disposed = ShutdownReport {
            pending_local_work: u32::try_from(remaining).unwrap_or(u32::MAX),
            peer_cleanup_confirmed: false,
        };
        *report = Some(disposed.clone());
        disposed
    }
}

impl Default for HostContext {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for HostContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostContext").finish_non_exhaustive()
    }
}

/// A session's context (§5.6), created by `open` on the calling thread and
/// dropped when the session ends. Every operation reaches it only through its
/// gate. Code on the session path reads session-relevant input from here, or
/// from the host context, never from the process (O9).
pub(crate) struct SessionContext {
    host: HostContext,
    environment: EnvironmentSnapshot,
    limits: Limits,
    /// The off switch's resolved value, from `open`'s options only.
    transport_off: bool,
}

#[allow(
    dead_code,
    reason = "reached through the gate by CS1.6, Phase 2 and CS3.10"
)]
impl SessionContext {
    pub(crate) fn host(&self) -> &HostContext {
        &self.host
    }

    pub(crate) fn environment(&self) -> &EnvironmentSnapshot {
        &self.environment
    }

    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The off switch's resolved value, as `open`'s options gave it. With it
    /// on, CS3.10 runs every transport-scope request on the native path.
    pub(crate) fn transport_off(&self) -> bool {
        self.transport_off
    }
}

/// `open`'s options (§9): the host context to use, the endpoint environment
/// the driver captured (§5.6), the limits of §1, and the off switch's
/// resolved value (§5.6 as the server design amends it).
///
/// `non_exhaustive`: build it with `SessionOptions::new`, then change
/// `limits` or `transport_off`.
#[derive(Debug)]
#[non_exhaustive]
pub struct SessionOptions {
    /// The host context the session shares with the driver's other sessions.
    pub host: HostContext,
    /// The endpoint environment the driver captured, once, at its edge (§5.6).
    pub environment: EnvironmentSnapshot,
    /// The session's limits (§1). `open` validates them.
    pub limits: Limits,
    /// The off switch's resolved value (the server design's §5): the driver
    /// resolves the switch's forms into one value before `open`. The snapshot
    /// never carries it, and core never derives it from the snapshot or from
    /// the process's own environment or configuration. False from `new`.
    pub transport_off: bool,
}

impl SessionOptions {
    /// Options with the contract's default limits and the off switch off.
    pub fn new(host: HostContext, environment: EnvironmentSnapshot) -> Self {
        Self {
            host,
            environment,
            limits: Limits::default(),
            transport_off: false,
        }
    }
}

/// Opens a session (§9). It validates `options.limits` (§1) and refuses a
/// host context that has been shut down, each with `invalid_request` before
/// any effect. It then creates the session context on the calling thread
/// (§5.6), and returns the client end of the session's in-process channel.
pub fn open(options: SessionOptions) -> ModelResult<ClientChannel> {
    let channel_limits = validate_limits(&options.limits)?;
    if options.host.shared.shut_down.load(Ordering::Acquire) {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "the host context has been shut down",
        ));
    }
    let SessionOptions {
        host,
        environment,
        limits,
        transport_off,
    } = options;
    let session = Arc::new(SessionContext {
        host,
        environment,
        limits,
        transport_off,
    });
    let (end, host_end) = pair(channel_limits);
    Ok(ClientChannel {
        end,
        held: Mutex::new(Some(Held { host_end, session })),
    })
}

/// The client end of a session's in-process channel (§3, §9): an end of
/// gwz-session-channel's `pair`, whose two bounded queues each hold the
/// outstanding-call limit on the call lane plus the control reserve on the
/// control lane.
///
/// - `send(frame, lane)` never waits. A full lane refuses the frame with
///   `SendError::Full`, which gives it back, and the channel stays open; a
///   driver reports that as `transport_session_full`. A control call,
///   `operation.cancel` or `session.close`, goes on `Lane::Control`, which
///   the outstanding-call limit never refuses, and every other call on
///   `Lane::Call` (crate map §7).
/// - `recv()` waits for the host's next frame, or the session's end, which it
///   reports as `Closed`.
/// - `close()`, like dropping the `ClientChannel`, ends the session, and the
///   session's context drops then, not when the handle does.
/// - A frame whose tag the in-process channel does not carry (it carries 1 to
///   3), or larger than `MAX_FRAME_BYTES`, ends the session.
///
/// The host's end waits here until the session host serves it (session plan
/// CS2.2, in gwz-session-host's `serve`), so until then no host answers, and
/// a `recv` waits for a frame that does not come; no driver calls one before
/// the plan's Phase 4 or 6. This end owns the session: closing it or dropping
/// it ends the session, and the session context and its snapshot drop then,
/// the snapshot overwriting its buffers as it drops (§5.6; §8's channel
/// closure, before any worker exists).
pub struct ClientChannel {
    end: InProcessEnd,
    /// What the session holds until it ends here. `close()` takes it, so the
    /// context drops, and its snapshot is wiped, at the close, as at a drop.
    held: Mutex<Option<Held>>,
}

/// What a session holds beside its client end, while it lasts.
struct Held {
    /// The host's end, until CS2.2's host takes it. It never goes into the
    /// session context: that is the gates' per-session data, and a crossing's
    /// closure must not reach a channel end (O6).
    #[allow(dead_code, reason = "held for its drop until CS2.2's host serves it")]
    host_end: InProcessEnd,
    #[allow(dead_code, reason = "held for its drop until CS2.2")]
    session: Arc<SessionContext>,
}

impl ClientChannel {
    /// Sends `frame` on `lane` (§3). It never waits.
    ///
    /// # Errors
    ///
    /// `SendError::Full` when the lane is full, leaving the channel open, and
    /// `SendError::Closed` once the session has ended; the frame comes back
    /// either way.
    pub fn send(&self, frame: Frame, lane: Lane) -> Result<(), SendError> {
        self.end.send(frame, lane)
    }

    /// Takes the host's next frame, waiting until one arrives or the session
    /// has ended (§3).
    ///
    /// # Errors
    ///
    /// Why the session ended; every later call returns the same reason.
    pub fn recv(&self) -> Result<Frame, Closed> {
        self.end.recv()
    }

    /// Ends the session at this end, as dropping it does (§8's channel
    /// closure): the channel closes, then the host's end and the session
    /// context drop, and with the context its snapshot, which is wiped as it
    /// drops (§5.6). Closing again changes nothing.
    pub fn close(&self) {
        self.end.close();
        let held = self.held().take();
        drop(held);
    }

    fn held(&self) -> MutexGuard<'_, Option<Held>> {
        // Only `close` and the tests' accessors take this lock. Nothing done
        // under it leaves the `Option` half-changed, even if a test's closure
        // panics, so a poisoned lock is recovered.
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl FrameSink for ClientChannel {
    fn send(&self, frame: Frame, lane: Lane) -> Result<(), SendError> {
        self.end.send(frame, lane)
    }

    fn close(&self) {
        ClientChannel::close(self);
    }
}

impl FrameSource for ClientChannel {
    fn recv(&self) -> Result<Frame, Closed> {
        self.end.recv()
    }
}

impl fmt::Debug for ClientChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientChannel").finish_non_exhaustive()
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl ClientChannel {
            /// The session context, while the session lasts.
            pub(crate) fn session(&self) -> Arc<SessionContext> {
                // The lock is released at the end of the statement, before
                // the `expect`.
                let session = self.held().as_ref().map(|held| Arc::clone(&held.session));
                session.expect("the session has not ended")
            }

            /// Runs `f` on the host's end, which CS2.2's host will serve,
            /// while the session lasts; `None` once it has ended here.
            pub(crate) fn with_host_end<R>(&self, f: impl FnOnce(&InProcessEnd) -> R) -> Option<R> {
                self.held().as_ref().map(|held| f(&held.host_end))
            }
        }

        impl HostContext {
            pub(crate) fn same_as(&self, other: &HostContext) -> bool {
                Arc::ptr_eq(&self.shared, &other.shared)
            }

            /// A view of the host context's supervisor thread, through
            /// gwz-session-host's `test-support` feature.
            pub(crate) fn supervisor_watch(
                &self,
            ) -> gwz_session_host::test_support::SupervisorWatch {
                gwz_session_host::test_support::watch(&self.shared.supervisor)
            }

            /// `shutdown` within a short bound, so no test waits 5 seconds.
            pub(crate) fn shutdown_within(&self, bound: Duration) -> ShutdownReport {
                self.shared.shutdown(bound)
            }
        }

        /// A session with the default limits and an empty snapshot.
        pub(crate) fn test_session() -> ClientChannel {
            let environment = EnvironmentSnapshot::from_byte_pairs(Vec::new()).unwrap();
            open(SessionOptions::new(HostContext::new(), environment)).unwrap()
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
