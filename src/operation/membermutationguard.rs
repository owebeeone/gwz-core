//! The guard of one member step, and the port a network handler takes it through (core session design §5.1;
//! step 2.3c of `dev-docs/GwzServerCore-Step2.3-Plan.md`).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | A member step, that member's network exchange and ref update, holds its member's guard from before the exchange to after the ref update (for a pull preflight, after the tracking-ref read that names its plan) | §5.1 "Serialization" |
//! | The guard is released when it drops, on every path, a panic included | §5.1 |
//! | The handlers that take the step are `fetch`, `push`, `pull_head` and `tag` (its push, fetch and remote delete); the only exclusion is the dry-run push plans, which update no refs | §5.1 |
//! | The manager that hands guards out is [`MemberLockManager`](super::MemberLockManager), beside this module; the host context owns and serves the one manager, and a handler sees only [`MemberSteps`] (a direct caller's takes a manager of its own call) | §5.1, §5.6 |
//!
//! The guard hides the manager that made it, and the port is a trait, so the handlers (`workspace_ops`) name
//! nothing of the session host.

use std::path::Path;

use crate::model::ModelResult;

/// Held while one member's network exchange and ref update runs; dropping it lets the next step of that member
/// proceed. It is made by the member lock manager and by nothing else, and nothing outside this crate can name it.
#[must_use = "the member is released when the guard drops"]
pub(crate) struct MemberMutationGuard {
    /// What releases the member when it drops.
    _held: Box<dyn Send>,
}

impl MemberMutationGuard {
    /// A guard that releases the member when `held` drops.
    pub(crate) fn new(held: impl Send + 'static) -> Self {
        Self {
            _held: Box::new(held),
        }
    }
}

/// What a fetch, push, pull or tag handler takes around each member step.
///
/// `enter` blocks until the member is free, and refuses with `cancelled` when the call's token is cancelled
/// before or during the wait. A handler that holds no token (a direct caller's) gets a manager of its own call,
/// which serves that call only.
pub(crate) trait MemberSteps: Sync {
    /// The guard of `member` of the workspace at `workspace` (`@root` for the root repository).
    fn enter(&self, workspace: &Path, member: &str) -> ModelResult<MemberMutationGuard>;
}
