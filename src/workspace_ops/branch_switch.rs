use std::path::Path;

use crate::git::{GitBackend, GitRepositoryState, GitStatus};
use crate::model::{ErrorCode, ModelError, ModelResult};

/// Accept dirty branch attachment only when it cannot move the checked-out
/// commit or overlap another native Git operation.
pub(crate) fn preflight_branch_switch<B: GitBackend>(
    backend: &B,
    path: &Path,
    member_id: &str,
    member_path: &str,
    target_commit: &str,
    status: &GitStatus,
) -> ModelResult<()> {
    if backend
        .repository_state(path)
        .map_err(|error| error.with_member(member_id, member_path))?
        != GitRepositoryState::Clean
    {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "has an in-progress Git operation",
        )
        .with_member(member_id, member_path));
    }
    if !status.is_dirty {
        return Ok(());
    }
    let head = backend
        .head(path)
        .map_err(|error| error.with_member(member_id, member_path))?;
    if head.commit.as_deref() == Some(target_commit) {
        return Ok(());
    }
    Err(
        ModelError::new(ErrorCode::DirtyMember, "has uncommitted changes")
            .with_member(member_id, member_path),
    )
}
