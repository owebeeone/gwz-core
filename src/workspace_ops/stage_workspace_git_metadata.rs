use std::path::Path;

use crate::git::GitBackend;
use crate::model::ModelResult;
use crate::workspace::WORKSPACE_DIR;

/// Stage the workspace metadata dir (`gwz.conf`) into the root index. Member repos are
/// hidden via `.git/info/exclude` (not tracked), so only GWZ metadata is staged.
pub(crate) fn stage_workspace_git_metadata<B: GitBackend>(
    backend: &B,
    root: &Path,
) -> ModelResult<()> {
    backend.stage_paths(root, &[WORKSPACE_DIR]).map(|_| ())
}
