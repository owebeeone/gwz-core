use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::artifact::{LockArtifact, ManifestArtifact};
use crate::git::GitBackend;
use crate::model::ModelResult;
use crate::workspace::{RUNTIME_DIR, WORKSPACE_DIR};

use super::*;

const EXCLUDE_BEGIN: &str = "# BEGIN GWZ managed member repositories";
const EXCLUDE_END: &str = "# END GWZ managed member repositories";

/// Refresh the workspace git boundary and stage the workspace metadata. gwz hides member
/// repos, its tmp dir, and local runtime state from the ROOT repo via a managed block in
/// `.git/info/exclude`: local, never committed, regenerated on every run (we don't persist
/// it). Members are therefore untracked; `gwz.yml` / `gwz.lock.yml` is the authoritative
/// record of member state. (Supersedes the gitlink boundary.)
pub(crate) fn sync_workspace_boundary<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
) -> ModelResult<()> {
    ensure_workspace_exclude(backend, root, manifest, lock)?;
    stage_workspace_git_metadata(backend, root)
}

/// Regenerate gwz's managed block in `<root>/.git/info/exclude` so the root repo ignores
/// `/{RUNTIME_DIR}/`, `/{WORKSPACE_DIR}/.tmp/`, and every member path. Idempotent, preserves any
/// non-gwz lines, and is purely local (never committed). Paths are the union of stale
/// lock records, active manifest rows, and inactive rows that still have a Git checkout.
pub(crate) fn ensure_workspace_exclude<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
) -> ModelResult<()> {
    let mut paths: BTreeSet<String> = lock
        .members
        .values()
        .map(|member| member.path.clone())
        .collect();
    for member in &manifest.members {
        if member.active || backend.is_repository(&root.join(&member.path))? {
            paths.insert(member.path.clone());
        }
    }

    let mut lines = vec![
        EXCLUDE_BEGIN.to_owned(),
        format!("/{RUNTIME_DIR}/"),
        format!("/{WORKSPACE_DIR}/.tmp/"),
    ];
    lines.extend(paths.into_iter().map(|path| format!("/{path}/")));
    lines.push(EXCLUDE_END.to_owned());
    let block = lines.join("\n");

    let exclude_path = root.join(".git").join("info").join("exclude");
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let existing = match fs::read_to_string(&exclude_path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io_error(error)),
    };
    let updated = replace_managed_block(&existing, &block);
    if updated != existing {
        fs::write(&exclude_path, updated).map_err(io_error)?;
    }
    Ok(())
}

/// Surgically replace gwz's `BEGIN..END` block within `existing`, preserving everything
/// else (including a user's own exclude lines); appends the block if not yet present.
fn replace_managed_block(existing: &str, block: &str) -> String {
    if let Some(begin) = existing.find(EXCLUDE_BEGIN)
        && let Some(relative_end) = existing[begin..].find(EXCLUDE_END)
    {
        let end = begin + relative_end + EXCLUDE_END.len();
        let after_end = if existing[end..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        let mut out = String::with_capacity(existing.len() + block.len());
        out.push_str(&existing[..begin]);
        out.push_str(block);
        out.push('\n');
        out.push_str(&existing[after_end..]);
        return out;
    }
    if existing.trim().is_empty() {
        format!("{block}\n")
    } else if existing.ends_with('\n') {
        format!("{existing}{block}\n")
    } else {
        format!("{existing}\n{block}\n")
    }
}
