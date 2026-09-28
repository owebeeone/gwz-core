//! Cancellation tokens, operation gates and the handler's context (session
//! plan CS1.4), for the core session contract (gwz-dev
//! `dev-docs/GwzCoreSessionDesign.md`).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | A call's token and gate are created together, before admission, any reply or any worker (`CallControls::new`, called when the host reads the call's frame) | O7, §5.1 |
//! | Only the call's record holds the authority to cancel (`CallControls`); workers, handlers and transport requests hold the token and observe it | §5.2, §5.3 "The token is the only authority" |
//! | Nothing else cancels: no crossing, failure or drop, and no record discarded without a cancel | §5.3 "Nothing else does" |
//! | A registration attaches to the token, and is refused with `Cancelled` once the token is cancelled | §5.2 transport entry, §5.1 lock waits |
//! | Live: every crossing goes through | O8, §5.6 |
//! | Cancelled: effectful requests and log appends are refused with `Cancelled`; reports still land | O8, §5.3, §5.6 |
//! | Revoked: reports are ignored as well; once `revoke` returns no crossing is in flight, so none touches session state again | O8, §5.6, §8 step 5 |
//! | A gate holds its session weakly, so it keeps no session state alive; one whose session has ended acts revoked | O8, §5.6 |
//! | No crossing inside a crossing or a cancel callback, on any gate: one panics instead of deadlocking, and `state()` takes no lock, so close's bound stays meetable | §8 step 5, §16 |
//! | The handler's context carries the gate and its token | §5.2, §16 |
//!
//! A crossing's closure runs holding its gate's lock, which is what makes
//! revocation final, so the closure must be short and must not block. A wait,
//! such as a member-lock wait, happens after its crossing, and observes the
//! token to wake on cancellation (§5.1).
//!
//! The rule: a crossing's closure, or a cancel callback, never crosses a gate,
//! whichever gate, never revokes one, and takes no session lock. A thread
//! that tries a crossing or a `revoke` there panics with the rule's words
//! instead of deadlocking. The panic unwinds through the outer crossing,
//! whose lock is then free, so `revoke` proceeds; a callback's panic is caught
//! where callbacks run. `state()` never waits on a crossing, so it is safe
//! anywhere, inside a closure included.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use super::context::SessionContext;
use crate::model::{ErrorCode, ModelError, ModelResult};

thread_local! {
    /// Whether this thread is running a crossing's closure or a cancel
    /// callback, where it may not cross or revoke a gate.
    static CROSSING: Cell<bool> = const { Cell::new(false) };
}

/// Marks this thread as running a crossing's closure or a cancel callback,
/// until it drops, unwinding included, and restores the previous mark.
struct CrossingMark {
    previous: bool,
}

impl CrossingMark {
    fn enter() -> Self {
        Self {
            previous: CROSSING.with(|crossing| crossing.replace(true)),
        }
    }
}

impl Drop for CrossingMark {
    fn drop(&mut self) {
        CROSSING.with(|crossing| crossing.set(self.previous));
    }
}

/// Panics, rather than deadlock, when this thread is running a crossing's
/// closure or a cancel callback.
fn forbid_nesting(what: &str) {
    if CROSSING.with(Cell::get) {
        panic!(
            "{what} inside a gate crossing or a cancel callback: \
             a crossing's closure or a cancel callback never crosses a gate"
        );
    }
}

type Callback = Box<dyn FnOnce() + Send>;

struct TokenInner {
    cancelled: AtomicBool,
    callbacks: Mutex<Callbacks>,
}

#[derive(Default)]
struct Callbacks {
    next: u64,
    pending: Vec<(u64, Callback)>,
}

impl TokenInner {
    /// Cancels once. The callbacks run on the cancelling thread, outside the
    /// token's lock and marked like a crossing's closure; one that panics does
    /// not unwind into the canceller.
    fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        let pending = std::mem::take(&mut lock(&self.callbacks).pending);
        let _mark = CrossingMark::enter();
        for (_, callback) in pending {
            let _ = catch_unwind(AssertUnwindSafe(callback));
        }
    }
}

/// A call's cancellation token as its observers hold it: the gate, the worker,
/// the handler's context and a transport request (O7, §5.2, §5.3). It cannot
/// cancel; only the call's record can, through `CallControls`.
#[derive(Clone)]
pub(crate) struct CancellationToken {
    inner: Arc<TokenInner>,
}

impl CancellationToken {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Attaches `callback`, which runs once, on the cancelling thread, when the
    /// token is cancelled. It must be short, take no session lock, and never
    /// cross or revoke a gate: one that tries panics, and the panic is caught.
    /// Refused with `Cancelled` if the token already is, and then it never
    /// runs (§5.2: a request registering with a cancelled token fails with
    /// `Cancelled`). Dropping the registration detaches the callback; one
    /// dropped while the token is being cancelled may still run once.
    pub(crate) fn on_cancel(
        &self,
        callback: impl FnOnce() + Send + 'static,
    ) -> ModelResult<CancelRegistration> {
        let mut callbacks = lock(&self.inner.callbacks);
        if self.is_cancelled() {
            return Err(cancelled("the operation's token is cancelled"));
        }
        let id = callbacks.next;
        callbacks.next += 1;
        callbacks.pending.push((id, Box::new(callback)));
        Ok(CancelRegistration {
            token: Arc::downgrade(&self.inner),
            id,
        })
    }
}

/// Keeps a callback attached to its token; dropping it detaches the callback.
#[must_use = "dropping the registration detaches its callback"]
pub(crate) struct CancelRegistration {
    token: Weak<TokenInner>,
    id: u64,
}

impl Drop for CancelRegistration {
    fn drop(&mut self) {
        let Some(token) = self.token.upgrade() else {
            return;
        };
        let detached = {
            let mut callbacks = lock(&token.callbacks);
            let index = callbacks.pending.iter().position(|(id, _)| *id == self.id);
            index.map(|index| callbacks.pending.swap_remove(index))
        };
        drop(detached);
    }
}

/// What a call's record keeps: the authority over the call's token, and its
/// gate (O7). Only `operation.cancel` and close use it (§5.3, §8). Dropping it
/// cancels nothing.
pub(crate) struct CallControls {
    token: Arc<TokenInner>,
    gate: OperationGate,
}

impl CallControls {
    /// Creates the call's token and gate together. The session host calls it
    /// when it reads the call's frame: before resolution, admission, any
    /// `accepted` reply or any worker (O7, §5.1). The worker later receives a
    /// clone of the gate, and the token with it.
    pub(crate) fn new(session: &Arc<SessionContext>) -> Self {
        let token = Arc::new(TokenInner {
            cancelled: AtomicBool::new(false),
            callbacks: Mutex::default(),
        });
        let gate = OperationGate {
            inner: Arc::new(GateInner {
                token: CancellationToken {
                    inner: Arc::clone(&token),
                },
                session: Arc::downgrade(session),
                crossing: Mutex::new(()),
                revoked: AtomicBool::new(false),
            }),
        };
        Self { token, gate }
    }

    pub(crate) fn gate(&self) -> &OperationGate {
        &self.gate
    }

    /// Cancels the call's token: `operation.cancel` and close, and nothing
    /// else (§5.3). Idempotent. The token's callbacks run on this thread.
    pub(crate) fn cancel(&self) {
        self.token.cancel();
    }

    /// Revokes the gate at the close bound (§8 step 5). Close cancels a token
    /// before it revokes the gate, so this cancels first if that has not
    /// happened. It returns once no crossing is in flight; after that, none
    /// touches session state. Call it holding no session lock, since it waits
    /// for a crossing whose closure may take one, and never from a crossing's
    /// closure or a cancel callback, where it panics.
    pub(crate) fn revoke(&self) {
        forbid_nesting("a revoke");
        self.cancel();
        let _held = lock(&self.gate.inner.crossing);
        self.gate.inner.revoked.store(true, Ordering::Release);
    }
}

/// The three states of a gate (O8, §5.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GateState {
    /// Every crossing goes through.
    Live,
    /// The token is cancelled: effectful requests and log appends are
    /// refused with `Cancelled`, and reports still land.
    Cancelled,
    /// Revoked at the close bound, or the session has ended: reports are
    /// ignored as well.
    Revoked,
}

/// A call's gate: the only way its worker reaches session state and
/// resources (O8, §5.6). It knows the call's token and, holding its session
/// weakly, the session's lifetime.
#[derive(Clone)]
pub(crate) struct OperationGate {
    inner: Arc<GateInner>,
}

struct GateInner {
    token: CancellationToken,
    /// Held weakly, so a gate keeps no session state alive.
    session: Weak<SessionContext>,
    /// Held while a crossing's closure runs, and by `revoke`: once `revoke`
    /// returns, no crossing is in flight.
    crossing: Mutex<()>,
    /// Set by `revoke`, holding `crossing`; `state()` reads it without it.
    revoked: AtomicBool,
}

/// What a crossing may touch while it holds the gate. Later steps add the
/// call's record here: its event log and terminal (CS2.4, CS2.5).
pub(crate) struct GateScope<'a> {
    session: &'a SessionContext,
}

impl<'a> GateScope<'a> {
    pub(crate) fn session(&self) -> &'a SessionContext {
        self.session
    }
}

/// What became of a crossing.
enum Crossed<R> {
    Ran(R),
    Revoked,
    Cancelled,
}

impl OperationGate {
    pub(crate) fn token(&self) -> &CancellationToken {
        &self.inner.token
    }

    /// The gate's state. It takes no lock, so it is safe anywhere, inside a
    /// crossing's closure included.
    pub(crate) fn state(&self) -> GateState {
        let inner = &self.inner;
        if inner.revoked.load(Ordering::Acquire) || inner.session.strong_count() == 0 {
            GateState::Revoked
        } else if inner.token.is_cancelled() {
            GateState::Cancelled
        } else {
            GateState::Live
        }
    }

    /// An effectful request: a lock, a transport open or a log registration
    /// (§5.6). Refused with `Cancelled` once the token is cancelled, and after
    /// revocation, without running `f` (§5.3: every crossing is a
    /// cancellation point).
    pub(crate) fn effect<R>(&self, f: impl FnOnce(GateScope<'_>) -> R) -> ModelResult<R> {
        self.refusable(f)
    }

    /// An append to a log the operation produces. Refused like an effect, so a
    /// cancelled producer stops (§5.6).
    pub(crate) fn append<R>(&self, f: impl FnOnce(GateScope<'_>) -> R) -> ModelResult<R> {
        self.refusable(f)
    }

    /// A report: an event, the terminal, or the seal or close of a log the
    /// operation produces (§5.6, §6). It lands after cancellation, and is
    /// ignored after revocation: `f` does not run and the result is `None`.
    pub(crate) fn report<R>(&self, f: impl FnOnce(GateScope<'_>) -> R) -> Option<R> {
        match self.cross(false, f) {
            Crossed::Ran(value) => Some(value),
            Crossed::Revoked | Crossed::Cancelled => None,
        }
    }

    fn refusable<R>(&self, f: impl FnOnce(GateScope<'_>) -> R) -> ModelResult<R> {
        match self.cross(true, f) {
            Crossed::Ran(value) => Ok(value),
            Crossed::Revoked => Err(cancelled("the operation's gate is revoked")),
            Crossed::Cancelled => Err(cancelled("the operation's token is cancelled")),
        }
    }

    /// One crossing. `f` runs holding the gate, marked so that it cannot cross
    /// a gate itself. A crossing attempted where one is forbidden panics
    /// before it takes the lock.
    fn cross<R>(
        &self,
        refuse_if_cancelled: bool,
        f: impl FnOnce(GateScope<'_>) -> R,
    ) -> Crossed<R> {
        forbid_nesting("a crossing");
        let _held = lock(&self.inner.crossing);
        if self.inner.revoked.load(Ordering::Acquire) {
            return Crossed::Revoked;
        }
        let Some(session) = self.inner.session.upgrade() else {
            return Crossed::Revoked;
        };
        if refuse_if_cancelled && self.inner.token.is_cancelled() {
            return Crossed::Cancelled;
        }
        let _mark = CrossingMark::enter();
        Crossed::Ran(f(GateScope { session: &session }))
    }
}

/// The handler's context: how a handler reaches its gate. It carries the
/// call's token, so handlers can check it at safe points without another API
/// change (§5.2, §16). It is the operation's context that CS1.6's dispatch
/// signature takes.
#[derive(Clone)]
pub(crate) struct HandlerContext {
    gate: OperationGate,
}

impl HandlerContext {
    pub(crate) fn new(gate: OperationGate) -> Self {
        Self { gate }
    }

    pub(crate) fn gate(&self) -> &OperationGate {
        &self.gate
    }

    pub(crate) fn token(&self) -> &CancellationToken {
        self.gate.token()
    }
}

fn cancelled(message: &str) -> ModelError {
    ModelError::new(ErrorCode::Cancelled, message)
}

/// Every lock here guards state that a panic cannot leave half-written, so a
/// poisoned lock is recovered.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
