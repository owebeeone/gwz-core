use std::path::Path;

use crate::git::{Git2Backend, GitBackend};
use crate::model::ModelResult;
use crate::workspace::WORKSPACE_DIR;



pub(crate) fn stage_workspace_git_metadata(root: &Path) -> ModelResult<()> {
    let mut pathspecs = vec![WORKSPACE_DIR];
    if root.join(".gitignore").is_file() {
        pathspecs.push(".gitignore");
    }
    Git2Backend::new().stage_paths(root, &pathspecs).map(|_| ())
}

