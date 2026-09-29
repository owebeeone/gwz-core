//! Cancellation tokens (session plan CS1.4), moved from gwz-core's
//! `session_host/gate.rs`, with the refusal both they and the gate return.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Only the call's record cancels, through `CallControls`; every other holder observes | §5.3 "The token is the only authority" |
//! | Cancelling is idempotent, and its callbacks run once, on the cancelling thread, outside the token's lock; one that panics does not unwind into the canceller | §5.3; CS1.4 |
//! | A registration attaches to the token, and is refused once the token is cancelled; dropping it detaches the callback | §5.2 transport entry, §5.1 lock waits |
//! | A callback receives nothing, and being `'static` it cannot borrow a gate | crate map §2 (see the gate module on why nothing) |

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::lock;

/// Why a crossing, or a registration with a token, was refused. gwz-core
/// reports either as `cancelled` (§4.2), with this text.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Refused {
    /// The call's token is cancelled (§5.3).
    Cancelled,
    /// The gate is revoked, at the close bound or because its session has
    /// ended (§8 step 5).
    Revoked,
}

impl fmt::Display for Refused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Refused::Cancelled => "the operation's token is cancelled",
            Refused::Revoked => "the operation's gate is revoked",
        })
    }
}

impl std::error::Error for Refused {}

type Callback = Box<dyn FnOnce() + Send>;

pub(crate) struct TokenInner {
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
    /// token's lock; one that panics does not unwind into the canceller.
    pub(crate) fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        let pending = std::mem::take(&mut lock(&self.callbacks).pending);
        for (_, callback) in pending {
            let _ = catch_unwind(AssertUnwindSafe(callback));
        }
    }
}

/// A call's cancellation token as its observers hold it: the worker, the
/// handler's context, a view and a transport request (O7, §5.2, §5.3). It
/// cannot cancel; only the call's record can, through
/// [`CallControls`](crate::CallControls).
#[derive(Clone)]
pub struct CancellationToken {
    pub(crate) inner: Arc<TokenInner>,
}

impl CancellationToken {
    /// A new token, not cancelled, with no callbacks.
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(TokenInner {
                cancelled: AtomicBool::new(false),
                callbacks: Mutex::default(),
            }),
        }
    }

    /// Whether the token is cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Attaches `callback`, which runs once, on the cancelling thread, when
    /// the token is cancelled. It must be short and take no session lock; it
    /// receives nothing, and it never crosses or revokes a gate. Refused if
    /// the token already is cancelled, and then it never runs (§5.2: a
    /// request registering with a cancelled token fails with `Cancelled`).
    /// Dropping the registration detaches the callback; one dropped while
    /// the token is being cancelled may still run once.
    ///
    /// Being `'static`, a callback cannot borrow a gate. It holds one only
    /// if it is given the worker's own, and then the worker has none:
    ///
    /// ```compile_fail,E0373
    /// # use std::sync::Arc;
    /// # use gwz_session_host::CallControls;
    /// let session = Arc::new(());
    /// let (_controls, mut gate) = CallControls::new(&session);
    /// let token = gate.token().clone();
    /// let _registration = token.on_cancel(|| {
    ///     let _ = gate.report(|_| ());
    /// });
    /// ```
    ///
    /// # Errors
    ///
    /// [`Refused::Cancelled`] when the token is already cancelled.
    pub fn on_cancel(
        &self,
        callback: impl FnOnce() + Send + 'static,
    ) -> Result<CancelRegistration, Refused> {
        let mut callbacks = lock(&self.inner.callbacks);
        if self.is_cancelled() {
            return Err(Refused::Cancelled);
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

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Keeps a callback attached to its token; dropping it detaches the
/// callback.
#[must_use = "dropping the registration detaches its callback"]
pub struct CancelRegistration {
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

impl fmt::Debug for CancelRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancelRegistration")
            .finish_non_exhaustive()
    }
}
