//! The member lock manager of the host context (core session design §5.1, §5.6; step 2.3c of
//! `dev-docs/GwzServerCore-Step2.3-Plan.md`).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | One manager lives in the host context, so the sessions of one host context never update a member's refs at once | §5.1, §5.6; `GwzSplitCommandChannel.md` §1 |
//! | A member is a workspace root and a member id (`@root` for the root repository); different members never wait for each other | §5.1 |
//! | `lock` blocks until the member is free: it waits, it never refuses | §5.1 |
//! | A wait in progress wakes when the call's token is cancelled, and refuses with `cancelled`; a token already cancelled refuses before any wait | §5.1 "a gate crossing", §5.3 |
//! | A panic while holding a guard releases the member (the guard drops as it unwinds), and a lock poisoned by a panic is recovered, so one panic fails no later operation | §5.1 |
//! | Waiters take a freed member in no promised order | §5.1 |
//! | The steps that take it are every network update of a member's refs: `fetch` and `push` (members and root), `pull_head`'s member and root fetches (the merge plan's root fetch included) and `tag`'s push | §5.1; step 2.3c |
//! | **Excluded:** the dry-run push helpers (`push_member`, `push_root` with `dry_run`), because a dry run updates no refs; `tag`'s fetch (`tag_fetch`, which writes tags) is not plumbed yet and is reported as a finding | §5.1 |
//!
//! The wait is the caller's thread, parked on a condition variable. The cancellation callback runs on the
//! cancelling thread and only takes the manager's own short lock, then notifies; it takes no session lock and
//! crosses no gate (`gwz_session_host::CancellationToken::on_cancel`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use gwz_session_host::CancellationToken;

use crate::model::ModelResult;
use crate::operation::{MemberMutationGuard, MemberSteps};

/// One member of one workspace.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MemberKey {
    workspace: PathBuf,
    member: String,
}

#[derive(Default)]
struct Shared {
    /// The members held now.
    held: Mutex<HashSet<MemberKey>>,
    /// Notified when a member is released, and when a waiting token is cancelled.
    changed: Condvar,
}

impl Shared {
    /// The held set. A panic cannot leave it half-changed (it only inserts and removes one key under the lock),
    /// so a poisoned lock is recovered.
    fn held(&self) -> MutexGuard<'_, HashSet<MemberKey>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes the waiters. It takes the lock first, so a waiter that has checked its token and not yet parked
    /// cannot miss the wake.
    fn wake(&self) {
        drop(self.held());
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

    /// Blocks until `member` of the workspace at `workspace` is free and holds it until the guard drops.
    ///
    /// With a `cancel` token it refuses with `cancelled` when the token is cancelled before the call or while it
    /// waits, and a member it was waiting for stays with whoever holds it. Without one it waits as long as it takes.
    pub(crate) fn lock(
        &self,
        workspace: &Path,
        member: &str,
        cancel: Option<&CancellationToken>,
    ) -> ModelResult<MemberMutationGuard> {
        let key = MemberKey {
            workspace: workspace.to_path_buf(),
            member: member.to_owned(),
        };
        // Attached for the whole wait and detached when it ends. A token already cancelled refuses here.
        let _registration = match cancel {
            Some(token) => {
                let shared = Arc::clone(&self.shared);
                Some(token.on_cancel(move || shared.wake())?)
            }
            None => None,
        };
        let mut held = self.shared.held();
        loop {
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Err(gwz_session_host::Refused::Cancelled.into());
            }
            if held.insert(key.clone()) {
                break;
            }
            held = self
                .shared
                .changed
                .wait(held)
                .unwrap_or_else(PoisonError::into_inner);
        }
        drop(held);
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
        self.shared.held().remove(&self.key);
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
        impl MemberLockManager {
            /// How many members are held now.
            pub(crate) fn held_count(&self) -> usize {
                self.shared.held().len()
            }

            /// Poisons the manager's lock, as a panic inside it would.
            pub(crate) fn poison(&self) {
                let shared = Arc::clone(&self.shared);
                let _ = std::thread::spawn(move || {
                    let _held = shared.held.lock();
                    panic!("poisoning the member lock manager's lock");
                })
                .join();
            }
        }

        mod tests;
    }
}
