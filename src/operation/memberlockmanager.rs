//! The member lock manager of the host context (core session design §5.1, §5.6; step 2.3c of
//! `dev-docs/GwzServerCore-Step2.3-Plan.md`), and the port a handler takes it through.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | One manager lives in the host context, so the sessions of one host context never update a member's refs at once | §5.1, §5.6; `GwzSplitCommandChannel.md` §1 |
//! | A member is a workspace and a member id (`@root` for the root repository); different members never wait for each other | §5.1 |
//! | **The workspace of the key is the root's filesystem identity**, `std::fs::canonicalize` of the root the caller resolved (on Windows the verbatim on-disk form), so a symlink, a case variant or a short name of one workspace is one key. A root that does not canonicalize refuses the step with that I/O error (`io_error`); the lexical spelling is never the key | §5.1 |
//! | `lock` blocks until the member is free: it waits, it never refuses | §5.1 |
//! | A wait in progress wakes when the call's token is cancelled, and refuses with `cancelled`; a token already cancelled refuses before any wait | §5.1 "a gate crossing", §5.3 |
//! | A panic while holding a guard releases the member (the guard drops as it unwinds), and a lock poisoned by a panic is recovered, so one panic fails no later operation | §5.1 |
//! | Waiters take a freed member in no promised order | §5.1 |
//! | The steps that take it are every network update of a member's refs: `fetch` and `push` (members and root), `pull_head`'s member and root fetches (the merge plan's root fetch included), and `tag`'s push, fetch and remote delete | §5.1; step 2.3c |
//! | **Excluded:** the dry-run push plans (`push_member`, `push_root`), which read and plan and update no refs | §5.1 |
//! | The handler layer (`workspace_ops`) names this module and the [`MemberSteps`] port only; the session host owns and serves the one manager (`scripts/checks/check_member_step_boundary.py` holds both rules) | §5.6 |
//!
//! The wait is the caller's thread, parked on a condition variable. The cancellation callback runs on the
//! cancelling thread and only takes the manager's own short lock, then notifies; it takes no session lock and
//! crosses no gate (`gwz_session_host::CancellationToken::on_cancel`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use gwz_session_host::CancellationToken;

use super::{MemberMutationGuard, MemberSteps};
use crate::model::{ErrorCode, ModelError, ModelResult};

/// One member of one workspace.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MemberKey {
    /// The workspace root as the filesystem names it (`std::fs::canonicalize`), not as a request spelled it.
    workspace: PathBuf,
    member: String,
}

#[derive(Default)]
struct State {
    /// The members held now.
    held: HashSet<MemberKey>,
    /// The `lock` calls parked on `changed` now.
    waiting: usize,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    /// Notified when a member is released, and when a waiting token is cancelled.
    changed: Condvar,
    /// Notified when a call starts to wait, so an observer of `State::waiting` needs no polling.
    arrived: Condvar,
}

impl Shared {
    /// The manager's state. A panic cannot leave it half-changed (it only inserts and removes one key and
    /// counts waiters under the lock), so a poisoned lock is recovered.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes the waiters. It takes the lock first, so a waiter that has checked its token and not yet parked
    /// cannot miss the wake.
    fn wake(&self) {
        drop(self.state());
        self.changed.notify_all();
    }
}

/// The host context's member lock manager. A clone is another handle to the same manager.
#[derive(Clone, Default)]
pub(crate) struct MemberLockManager {
    shared: Arc<Shared>,
}

impl MemberLockManager {
    /// A manager with nothing held.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Blocks until `member` of the workspace at `workspace` is free and holds it until the guard drops. The
    /// workspace is keyed by its canonical path, so every spelling of one root is one workspace.
    ///
    /// With a `cancel` token it refuses with `cancelled` when the token is cancelled before the call or while it
    /// waits, and a member it was waiting for stays with whoever holds it. Without one it waits as long as it takes.
    ///
    /// # Errors
    ///
    /// `cancelled` as above, and `io_error` when `workspace` does not canonicalize (nothing is held then).
    pub(crate) fn lock(
        &self,
        workspace: &Path,
        member: &str,
        cancel: Option<&CancellationToken>,
    ) -> ModelResult<MemberMutationGuard> {
        // Attached for the whole wait and detached when it ends. A token already cancelled refuses here, before
        // the root is looked at.
        let _registration = match cancel {
            Some(token) => {
                let shared = Arc::clone(&self.shared);
                Some(token.on_cancel(move || shared.wake())?)
            }
            None => None,
        };
        let key = MemberKey {
            workspace: std::fs::canonicalize(workspace).map_err(|error| {
                ModelError::new(
                    ErrorCode::IoError,
                    format!("{} does not resolve: {error}", workspace.display()),
                )
            })?,
            member: member.to_owned(),
        };
        let mut state = self.shared.state();
        loop {
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Err(gwz_session_host::Refused::Cancelled.into());
            }
            if state.held.insert(key.clone()) {
                break;
            }
            state.waiting += 1;
            self.shared.arrived.notify_all();
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
            state.waiting -= 1;
        }
        drop(state);
        Ok(MemberMutationGuard::new(Release {
            shared: Arc::clone(&self.shared),
            key,
        }))
    }
}

/// Releases a member when its guard drops.
struct Release {
    shared: Arc<Shared>,
    key: MemberKey,
}

impl Drop for Release {
    fn drop(&mut self) {
        self.shared.state().held.remove(&self.key);
        self.shared.changed.notify_all();
    }
}

/// The manager as a handler sees it: the host context's manager and the call's token, or a manager of the call's
/// own and no token (a direct caller's handler, which serves that call only).
pub(crate) struct BoundMemberLocks {
    manager: MemberLockManager,
    cancel: Option<CancellationToken>,
}

impl BoundMemberLocks {
    /// The host context's `manager`, waited on under `cancel`.
    pub(crate) fn new(manager: MemberLockManager, cancel: CancellationToken) -> Self {
        Self {
            manager,
            cancel: Some(cancel),
        }
    }

    /// A manager of one call's own, for a handler its caller runs outside a session.
    pub(crate) fn per_call() -> Self {
        Self {
            manager: MemberLockManager::new(),
            cancel: None,
        }
    }
}

impl MemberSteps for BoundMemberLocks {
    fn enter(&self, workspace: &Path, member: &str) -> ModelResult<MemberMutationGuard> {
        self.manager.lock(workspace, member, self.cancel.as_ref())
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        /// A bound on a call reaching its wait; a row that is red because nothing waits pays it once.
        const WAITER_BOUND: std::time::Duration = std::time::Duration::from_secs(10);

        impl MemberLockManager {
            /// How many members are held now.
            pub(crate) fn held_count(&self) -> usize {
                self.shared.state().held.len()
            }

            /// How many `lock` calls are parked waiting for a member now.
            pub(crate) fn waiting_count(&self) -> usize {
                self.shared.state().waiting
            }

            /// Blocks until at least `count` calls are parked waiting for a member, or `bound` passes; whether they
            /// are. It is notified when a call starts to wait, so a row that needs "it is waiting" reads the
            /// manager's own state instead of letting a time window pass.
            pub(crate) fn wait_for_waiters(&self, count: usize, bound: std::time::Duration) -> bool {
                let (state, _) = self
                    .shared
                    .arrived
                    .wait_timeout_while(self.shared.state(), bound, |state| state.waiting < count)
                    .unwrap_or_else(PoisonError::into_inner);
                state.waiting >= count
            }

            /// Asserts that one call is parked waiting for a member (the manager's own state, not a time window) and
            /// that the call behind `answer` has not answered. `what` names the row.
            pub(crate) fn assert_waiting<R>(&self, answer: &std::sync::mpsc::Receiver<R>, what: &str) {
                assert!(
                    self.wait_for_waiters(1, WAITER_BOUND),
                    "{what}: no call is waiting in the manager"
                );
                assert!(answer.try_recv().is_err(), "{what}: the call answered");
            }

            /// Poisons the manager's lock, as a panic inside it would.
            pub(crate) fn poison(&self) {
                let shared = Arc::clone(&self.shared);
                let _ = std::thread::spawn(move || {
                    let _held = shared.state.lock();
                    panic!("poisoning the member lock manager's lock");
                })
                .join();
            }
        }

        mod tests;
    }
}
