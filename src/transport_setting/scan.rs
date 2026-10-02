//! The scan for repository values (design §3, D6, E9): each `gwz.transport`
//! in the own configuration of the repositories a request targets, which
//! never selects the transport and is never checked, listed for the drivers'
//! notes.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::{Found, IgnoredValue, Location, Scope, last_value};
use crate::transport_scope::Operation;
use crate::workspace_ops::{
    SelectedTarget, assert_workspace_id, resolve_action_targets, resolve_request_workspace_root,
};
use crate::{ActionKind, RequestMeta, artifact};

/// The largest file the scan opens: the bound of the server design's
/// configuration scan (its §5, "The files it reads").
const LIMIT: u64 = 1 << 20;

pub(super) fn ignored_values(
    operation: Operation,
    start: &Path,
    meta: &RequestMeta,
) -> Vec<IgnoredValue> {
    let mut ignored = Vec::new();
    let mut seen = BTreeSet::new();
    for (scope, worktree) in targets(operation, start, meta) {
        for file in repository_files(&worktree) {
            // A file that two targets share, such as a common directory's
            // `config`, is read once, and its note names the first target.
            if !seen.insert(file.clone()) {
                continue;
            }
            if let Some(Found { included, value }) = value_in(&file) {
                ignored.push(IgnoredValue {
                    scope: scope.clone(),
                    location: Location { file, included },
                    value: value.map(|value| String::from_utf8_lossy(&value).into_owned()),
                });
            }
        }
    }
    ignored
}

/// The worktree of each repository the request targets, as the operation's
/// handler resolves its targets (design §3; `target_selection.rs`'s
/// `resolve_action_targets`, which `handle_fetch.rs` and `push_member.rs`
/// call), in the handler's order, the root first.
///
/// What the handler would refuse, a workspace it cannot read, another
/// workspace's ID or a selection it rejects, targets nothing here: the
/// operation reports its own errors. A materialize to a tag later keeps only
/// the members the tag covers, which takes a backend; the scan reads its
/// whole selection, the members the tag can cover.
fn targets(operation: Operation, start: &Path, meta: &RequestMeta) -> Vec<(Scope, PathBuf)> {
    let Some(action) = action(operation) else {
        return Vec::new();
    };
    let Ok(root) = resolve_request_workspace_root(start, meta) else {
        return Vec::new();
    };
    let Ok(manifest) = artifact::read_manifest(&root) else {
        return Vec::new();
    };
    if assert_workspace_id(&manifest, meta.workspace.as_ref()).is_err() {
        return Vec::new();
    }
    let Ok(selected) = resolve_action_targets(&manifest, meta.selection.as_ref(), action) else {
        return Vec::new();
    };
    selected
        .into_iter()
        .map(|target| match target {
            SelectedTarget::Root => (Scope::Root, root.clone()),
            SelectedTarget::Member(member) => {
                (Scope::Member(member.id.clone()), root.join(&member.path))
            }
        })
        .collect()
}

/// The action whose target policy the operation's handler applies, or `None`
/// for an operation whose repositories do not exist before it runs: a clone
/// of a workspace into a new directory, a member clone and an initialisation
/// from sources check nothing. A pull to a snapshot runs materialize's
/// handler, so it takes materialize's targets.
fn action(operation: Operation) -> Option<ActionKind> {
    match operation {
        Operation::Fetch => Some(ActionKind::Fetch),
        Operation::Push => Some(ActionKind::Push),
        Operation::PullHead => Some(ActionKind::PullHead),
        Operation::Tag => Some(ActionKind::Tag),
        Operation::Materialize | Operation::PullSnapshot => Some(ActionKind::Materialize),
        Operation::CloneWorkspace | Operation::CloneRepoMember | Operation::InitFromSources => None,
    }
}

/// `<common dir>/config` and `<git dir>/config.worktree` of the repository at
/// `worktree`, as gwz's repository inspector names them
/// (`crates/repo-inspect/src/config_scan.rs`), or none when the git directory
/// cannot be located.
fn repository_files(worktree: &Path) -> Vec<PathBuf> {
    match git_dirs(worktree) {
        Some((git_dir, common_dir)) => {
            vec![common_dir.join("config"), git_dir.join("config.worktree")]
        }
        None => Vec::new(),
    }
}

/// The repository's git directory and common directory, found from the text
/// of its `.git` and `commondir` files as libgit2 reads them (`repository.c`'s
/// `read_gitfile` and its `commondir` lookup), never by opening the
/// repository, which would read `config` before any guard could run (design
/// §3, E9). `.git` is a directory, which is the git directory, or a regular
/// file of at most 1 MiB reading `gitdir:` and a path, relative to the
/// worktree; a `commondir` of at most 1 MiB names the common directory,
/// relative to the git directory, which is otherwise the common directory
/// too. Anything else, a link or a FIFO among them, locates nothing.
fn git_dirs(worktree: &Path) -> Option<(PathBuf, PathBuf)> {
    let dot_git = worktree.join(".git");
    let metadata = fs::symlink_metadata(&dot_git).ok()?;
    let git_dir = if metadata.is_dir() {
        dot_git
    } else {
        let text = small_text(&dot_git, &metadata)?;
        let target = text.trim_end().strip_prefix("gitdir:")?.trim_start();
        if target.is_empty() {
            return None;
        }
        worktree.join(target)
    };
    let commondir = git_dir.join("commondir");
    let common_dir = match fs::symlink_metadata(&commondir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => git_dir.clone(),
        Err(_) => return None,
        Ok(metadata) => {
            let text = small_text(&commondir, &metadata)?;
            let target = text.trim_end();
            if target.is_empty() {
                return None;
            }
            git_dir.join(target)
        }
    };
    Some((git_dir, common_dir))
}

/// The last `gwz.transport` the file reaches, when `symlink_metadata` reports
/// a regular file of at most 1 MiB (design §3, D6). Anything else, and a file
/// libgit2 cannot read, gets no note.
fn value_in(file: &Path) -> Option<Found> {
    let metadata = fs::symlink_metadata(file).ok()?;
    if !small_regular(&metadata) {
        return None;
    }
    last_value(file).ok().flatten()
}

/// The text of a regular file of at most 1 MiB, read no further than that.
fn small_text(file: &Path, metadata: &fs::Metadata) -> Option<String> {
    if !small_regular(metadata) {
        return None;
    }
    let mut text = Vec::new();
    fs::File::open(file)
        .ok()?
        .take(LIMIT + 1)
        .read_to_end(&mut text)
        .ok()?;
    if text.len() as u64 > LIMIT {
        return None;
    }
    String::from_utf8(text).ok()
}

/// Whether `metadata`, from `symlink_metadata`, is a regular file of at most
/// 1 MiB: not a link, a FIFO, a device or a directory.
fn small_regular(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_file() && metadata.len() <= LIMIT
}
