//! Operation gates and the handler's context (session plan CS1.4), for the
//! core session contract (gwz-dev `dev-docs/GwzCoreSessionDesign.md`), moved
//! from gwz-core's `session_host/gate.rs` with the core session crate map's
//! nesting design (`GwzCoreSessionCrateMap.md` §2) in place of CS1.4's
//! thread-local rule. The call's token is the `token` module's.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | A call's token, its record's controls and its gate are created together, before admission, any reply or any worker ([`CallControls::new`], called when the host reads the call's frame) | O7, §5.1 |
//! | Only the record's [`CallControls`] cancels and revokes, and only the worker's [`OperationGate`] crosses. Neither is `Clone`, so each capability has one holder | §5.3 "The token is the only authority"; map §2 |
//! | Nothing else cancels: no crossing, failure or drop, and no controls discarded without a cancel | §5.3 "Nothing else does" |
//! | Live: every crossing goes through | O8, §5.6 |
//! | Cancelled: effectful requests and log appends are refused; reports still land | O8, §5.3, §5.6 |
//! | Revoked: reports are ignored as well; once `revoke` returns no crossing is in flight, so none touches session state again | O8, §5.6, §8 step 5 |
//! | A gate holds its session weakly, so it keeps no session state alive; one whose session has ended acts revoked | O8, §5.6 |
//! | A crossing's closure receives only a [`GateScope`], which exposes the session's data and cannot cross or revoke; a crossing takes the gate mutably, so its closure cannot reach the same gate | map §2 |
//! | A cancel callback receives nothing, narrower than the map's "only a `GateScope`" (below), and being `'static` it cannot borrow a gate | map §2 |
//! | A [`GateView`], which clones, keeps `state()` and the token, and cannot cross | map §2 |
//! | The backstop: each gate records the thread running its closure, and a crossing or revoke of that gate from that thread panics instead of deadlocking on the gate's lock | map §2; CS1.4 Safety S-P3-3 |
//! | `state()` takes no lock, so it is safe anywhere, inside a closure included | §8 step 5 |
//! | The handler's context owns the gate and carries its token | §5.2, §16 |
//!
//! The per-session data `S` is core's, and this crate never looks inside it:
//! a scope hands the closure a reference to it (crate map §2, "Core's
//! per-session data passes through the host crate opaquely").
//!
//! A crossing's closure runs holding its gate's lock, which is what makes
//! revocation final, so the closure must be short and must not block. A wait,
//! such as a member-lock wait, happens after its crossing, and observes the
//! token to wake on cancellation (§5.1).
//!
//! The rule: a crossing's closure, or a cancel callback, never crosses a gate,
//! never revokes one, and takes no session lock. Capabilities now carry most
//! of it: a closure's only argument is a scope with neither power, and the
//! gate it runs in is mutably borrowed while it runs. A callback gets no
//! argument, and a gate is not `Clone`, so a callback holds one only if the
//! worker hands its own gate over, and then the worker has none. The gate's
//! thread record backs the rest up: a revoke of the gate, or a callback's
//! revoke, from the thread inside its closure panics before it takes the
//! lock or cancels, and a callback's panic is caught where callbacks run.
//!
//! The map says a cancel callback receives only a `GateScope`. Here it
//! receives nothing, which takes nothing away from what that sentence
//! guards, since neither can cross or revoke. Every planned callback (a
//! transport request's signal to its runtime, CS3.7; a credential helper's
//! kill, §5.8; a lock wait's wake, §5.1) needs no session data, and CS3.7's
//! callbacks may not report. A scope would also need the session alive when
//! the token is cancelled, which a gate that holds its session weakly cannot
//! promise. And a callback runs on the canceller's thread, the host's reading
//! thread for `operation.cancel`, where session data would invite a session
//! lock the rule forbids.
//!
//! What stays undetected, as the map states: controls smuggled into a
//! closure or a callback, such as another operation's gate, or a record's
//! controls reached from its worker. Crossing another gate, or revoking one,
//! from there is not caught, and review checks that no such path exists. So
//! is a closure that blocks on another thread's crossing.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, ThreadId};

use crate::lock;
use crate::token::{CancellationToken, Refused};

struct GateInner<S> {
    token: CancellationToken,
    /// Held weakly, so a gate keeps no session state alive.
    session: Weak<S>,
    /// Held while a crossing's closure runs, and by `revoke`: once `revoke`
    /// returns, no crossing is in flight.
    crossing: Mutex<()>,
    /// Set by `revoke`, holding `crossing`; `state()` reads it without it.
    revoked: AtomicBool,
    /// The thread running a crossing's closure, while one runs: the
    /// backstop's record. Set and cleared holding `crossing`, and read
    /// without it.
    crosser: Mutex<Option<ThreadId>>,
}

impl<S> GateInner<S> {
    /// Panics, rather than deadlock on `crossing`, when this thread is
    /// running this gate's closure.
    fn forbid_reentry(&self, what: &str) {
        let this = thread::current().id();
        if *lock(&self.crosser) == Some(this) {
            panic!(
                "{what} of a gate from inside its own crossing: \
                 a crossing's closure or a cancel callback never crosses a gate \
                 and never revokes one"
            );
        }
    }

    fn state(&self) -> GateState {
        if self.revoked.load(Ordering::Acquire) || self.session.strong_count() == 0 {
            GateState::Revoked
        } else if self.token.is_cancelled() {
            GateState::Cancelled
        } else {
            GateState::Live
        }
    }
}

/// Records this thread as the gate's crosser until it drops, unwinding
/// included.
struct Crosser<'a> {
    record: &'a Mutex<Option<ThreadId>>,
}

impl<'a> Crosser<'a> {
    fn enter(record: &'a Mutex<Option<ThreadId>>) -> Self {
        *lock(record) = Some(thread::current().id());
        Self { record }
    }
}

impl Drop for Crosser<'_> {
    fn drop(&mut self) {
        *lock(self.record) = None;
    }
}

/// What a call's record keeps: the authority to cancel the call's token and
/// to revoke its gate (O7). Only `operation.cancel` and close use it (§5.3,
/// §8). It is not `Clone`, so the record is its one holder, and dropping it
/// cancels nothing.
pub struct CallControls<S> {
    inner: Arc<GateInner<S>>,
}

impl<S> CallControls<S> {
    /// Creates the call's token, its controls and its gate together. The
    /// session host calls it when it reads the call's frame: before
    /// resolution, admission, any `accepted` reply or any worker (O7, §5.1).
    /// The record keeps the controls, and the gate goes to the call's worker
    /// when it starts, the token with it.
    pub fn new(session: &Arc<S>) -> (Self, OperationGate<S>) {
        let inner = Arc::new(GateInner {
            token: CancellationToken::new(),
            session: Arc::downgrade(session),
            crossing: Mutex::new(()),
            revoked: AtomicBool::new(false),
            crosser: Mutex::new(None),
        });
        let gate = OperationGate {
            inner: Arc::clone(&inner),
        };
        (Self { inner }, gate)
    }

    /// Cancels the call's token: `operation.cancel` and close, and nothing
    /// else (§5.3). Idempotent. The token's callbacks run on this thread.
    pub fn cancel(&self) {
        self.inner.token.inner.cancel();
    }

    /// Revokes the gate at the close bound (§8 step 5). Close cancels a token
    /// before it revokes the gate, so this cancels first if that has not
    /// happened. It returns once no crossing is in flight; after that, none
    /// touches session state. Call it holding no session lock, since it waits
    /// for a crossing whose closure may take one.
    ///
    /// # Panics
    ///
    /// From the thread running this gate's closure, before it cancels
    /// anything, instead of deadlocking.
    pub fn revoke(&self) {
        self.inner.forbid_reentry("a revoke");
        self.cancel();
        let _held = lock(&self.inner.crossing);
        self.inner.revoked.store(true, Ordering::Release);
    }

    /// A view of the gate, which clones.
    pub fn view(&self) -> GateView<S> {
        GateView {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S> fmt::Debug for CallControls<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CallControls")
            .field("state", &self.inner.state())
            .finish_non_exhaustive()
    }
}

/// The three states of a gate (O8, §5.6).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GateState {
    /// Every crossing goes through.
    Live,
    /// The token is cancelled: effectful requests and log appends are
    /// refused, and reports still land.
    Cancelled,
    /// Revoked at the close bound, or the session has ended: reports are
    /// ignored as well.
    Revoked,
}

/// A call's gate: the only way its worker reaches session state and
/// resources (O8, §5.6). It knows the call's token and, holding its session
/// weakly, the session's lifetime.
///
/// It is not `Clone`, and a crossing borrows it mutably, so a crossing's
/// closure cannot cross the gate it runs in:
///
/// ```compile_fail,E0499
/// # use std::sync::Arc;
/// # use gwz_session_host::CallControls;
/// let session = Arc::new(());
/// let (_controls, mut gate) = CallControls::new(&session);
/// let _ = gate.effect(|_| gate.effect(|_| ()));
/// ```
///
/// whereas one crossing after another is fine:
///
/// ```
/// # use std::sync::Arc;
/// # use gwz_session_host::CallControls;
/// let session = Arc::new(());
/// let (_controls, mut gate) = CallControls::new(&session);
/// let first = gate.effect(|_| ());
/// let second = gate.effect(|_| ());
/// assert!(first.is_ok() && second.is_ok());
/// ```
pub struct OperationGate<S> {
    inner: Arc<GateInner<S>>,
}

/// What a crossing may touch while it holds the gate: the session's data.
/// Later steps add the call's record here: its event log and terminal (CS2.4,
/// CS2.5).
///
/// It cannot cross a gate or revoke one:
///
/// ```compile_fail,E0599
/// # use std::sync::Arc;
/// # use gwz_session_host::CallControls;
/// let session = Arc::new(());
/// let (_controls, mut gate) = CallControls::new(&session);
/// let _ = gate.effect(|scope| scope.effect(|_| ()));
/// ```
///
/// ```compile_fail,E0599
/// # use std::sync::Arc;
/// # use gwz_session_host::CallControls;
/// let session = Arc::new(());
/// let (_controls, mut gate) = CallControls::new(&session);
/// let _ = gate.effect(|scope| scope.revoke());
/// ```
pub struct GateScope<'a, S> {
    session: &'a S,
}

impl<'a, S> GateScope<'a, S> {
    /// The session's data.
    pub fn session(&self) -> &'a S {
        self.session
    }
}

/// What became of a crossing.
enum Crossed<R> {
    Ran(R),
    Revoked,
    Cancelled,
}

impl<S> OperationGate<S> {
    /// The call's token.
    pub fn token(&self) -> &CancellationToken {
        &self.inner.token
    }

    /// The gate's state. It takes no lock, so it is safe anywhere.
    pub fn state(&self) -> GateState {
        self.inner.state()
    }

    /// A view of the gate, which clones: for code, a closure included, that
    /// reads the state or holds the token but never crosses.
    pub fn view(&self) -> GateView<S> {
        GateView {
            inner: Arc::clone(&self.inner),
        }
    }

    /// An effectful request: a lock, a transport open or a log registration
    /// (§5.6). Refused once the token is cancelled, and after revocation,
    /// without running `f` (§5.3: every crossing is a cancellation point).
    ///
    /// # Errors
    ///
    /// [`Refused::Cancelled`] or [`Refused::Revoked`].
    pub fn effect<R>(&mut self, f: impl FnOnce(GateScope<'_, S>) -> R) -> Result<R, Refused> {
        self.refusable(f)
    }

    /// An append to a log the operation produces. Refused like an effect, so
    /// a cancelled producer stops (§5.6).
    ///
    /// # Errors
    ///
    /// [`Refused::Cancelled`] or [`Refused::Revoked`].
    pub fn append<R>(&mut self, f: impl FnOnce(GateScope<'_, S>) -> R) -> Result<R, Refused> {
        self.refusable(f)
    }

    /// A report: an event, the terminal, or the seal or close of a log the
    /// operation produces (§5.6, §6). It lands after cancellation, and is
    /// ignored after revocation: `f` does not run and the result is `None`.
    pub fn report<R>(&mut self, f: impl FnOnce(GateScope<'_, S>) -> R) -> Option<R> {
        match self.cross(false, f) {
            Crossed::Ran(value) => Some(value),
            Crossed::Revoked | Crossed::Cancelled => None,
        }
    }

    fn refusable<R>(&mut self, f: impl FnOnce(GateScope<'_, S>) -> R) -> Result<R, Refused> {
        match self.cross(true, f) {
            Crossed::Ran(value) => Ok(value),
            Crossed::Revoked => Err(Refused::Revoked),
            Crossed::Cancelled => Err(Refused::Cancelled),
        }
    }

    /// One crossing: `f` runs holding the gate, with this thread recorded as
    /// its crosser. The mutable borrow already keeps `f` from this gate; the
    /// check is the backstop's other half.
    fn cross<R>(
        &mut self,
        refuse_if_cancelled: bool,
        f: impl FnOnce(GateScope<'_, S>) -> R,
    ) -> Crossed<R> {
        let inner = &*self.inner;
        inner.forbid_reentry("a crossing");
        let _held = lock(&inner.crossing);
        if inner.revoked.load(Ordering::Acquire) {
            return Crossed::Revoked;
        }
        let Some(session) = inner.session.upgrade() else {
            return Crossed::Revoked;
        };
        if refuse_if_cancelled && inner.token.is_cancelled() {
            return Crossed::Cancelled;
        }
        let _crosser = Crosser::enter(&inner.crosser);
        Crossed::Ran(f(GateScope { session: &session }))
    }
}

impl<S> fmt::Debug for OperationGate<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationGate")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

/// A view of a call's gate, which clones: `state()` and the token, and no
/// crossing (crate map §2).
pub struct GateView<S> {
    inner: Arc<GateInner<S>>,
}

impl<S> GateView<S> {
    /// The gate's state. It takes no lock, so it is safe anywhere, inside a
    /// crossing's closure included.
    pub fn state(&self) -> GateState {
        self.inner.state()
    }

    /// The call's token.
    pub fn token(&self) -> &CancellationToken {
        &self.inner.token
    }
}

impl<S> Clone for GateView<S> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S> fmt::Debug for GateView<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GateView")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

/// The handler's context: how a handler reaches its gate. It carries the
/// call's token, so handlers can check it at safe points without another API
/// change (§5.2, §16). It is the operation's context that CS1.6's dispatch
/// signature takes. It owns the gate and, like it, is not `Clone`.
pub struct HandlerContext<S> {
    gate: OperationGate<S>,
}

impl<S> HandlerContext<S> {
    /// The context of the worker that holds `gate`.
    pub fn new(gate: OperationGate<S>) -> Self {
        Self { gate }
    }

    /// The gate, to cross.
    pub fn gate(&mut self) -> &mut OperationGate<S> {
        &mut self.gate
    }

    /// The call's token.
    pub fn token(&self) -> &CancellationToken {
        self.gate.token()
    }

    /// A view of the gate, which clones.
    pub fn view(&self) -> GateView<S> {
        self.gate.view()
    }
}

impl<S> fmt::Debug for HandlerContext<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HandlerContext")
            .field("gate", &self.gate)
            .finish()
    }
}

mod tests;
