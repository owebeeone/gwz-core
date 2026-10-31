//! The guard of one member step, and the port a fetch or push handler takes it through (core session design
//! §5.1; step 2.3c of `dev-docs/GwzServerCore-Step2.3-Plan.md`).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | A member step, that member's network exchange and ref update, holds its member's guard from before the exchange to after the ref update | §5.1 "Serialization" |
//! | The guard is released when it drops, on every path, a panic included | §5.1 |
//! | The manager that hands guards out is `session_host::members`'s, and lives in the host context; a handler sees only [`MemberSteps`] | §5.1, §5.6 |
//!
//! The guard hides the manager that made it, so this module (below the handlers) names nothing of the session host.

use std::path::Path;

use crate::model::ModelResult;

/// Held while one member's network exchange and ref update runs; dropping it lets the next step of that member
/// proceed. It is made by the member lock manager (`session_host::members`) and by nothing else.
#[must_use = "the member is released when the guard drops"]
pub struct MemberMutationGuard {
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

/// What a fetch or push handler takes around each member step.
///
/// `enter` blocks until the member is free, and refuses with `cancelled` when the call's token is cancelled
/// before or during the wait. A handler that holds no token (a direct caller's) gets a manager of its own call,
/// which serves that call only.
pub(crate) trait MemberSteps: Sync {
    /// The guard of `member` of the workspace at `workspace` (`@root` for the root repository).
    fn enter(&self, workspace: &Path, member: &str) -> ModelResult<MemberMutationGuard>;
}
