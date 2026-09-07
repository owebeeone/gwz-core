//! Fresh, fail-closed admission of canonical derived marker bytes. Artifact
//! rendering belongs to core; ordinary work and history inspection stay intact.
use std::{fs, path::Path};

use gwz_repo_contract::{Observation, WorkKind, WorkObservation};

use crate::artifact::{
    CONF_INTEGRITY_MARKER_PATH, GUARDED_CONF_PATHS, canonical_conf_integrity_marker,
};

/// Status flags can hide physical edits. Regeneration requires byte evidence,
/// not a clean status bit, so assume-unchanged never authorizes an overwrite.
pub(super) fn source_marker_is_committed(root: &Path) -> bool {
    fn check(root: &Path) -> Option<bool> {
        let repo = git2::Repository::open(root).ok()?;
        let index = repo.index().ok()?;
        if index.has_conflicts() { return Some(false); }
        let path = Path::new(CONF_INTEGRITY_MARKER_PATH);
        let indexed = index.get_path(path, 0)?;
        let tree = repo.head().ok()?.peel_to_tree().ok()?;
        let committed = tree.get_path(path).ok()?;
        if indexed.id != committed.id() || indexed.mode != 0o100644
            || committed.filemode() != 0o100644 || indexed.flags & 0x8000 != 0
            || indexed.flags_extended != 0
        { return Some(false); }
        let mut physical = root.to_path_buf();
        for component in path.components() {
            physical.push(component);
            if fs::symlink_metadata(&physical).ok()?.file_type().is_symlink() { return Some(false); }
        }
        let blob = repo.find_blob(indexed.id).ok()?;
        Some(fs::read(&physical).ok()?.as_slice() == blob.content())
    }
    check(root).unwrap_or(false)
}

pub(super) fn discount_generated_marker(root: &Path, work: &mut Observation<WorkObservation>) -> Option<String> {
    let Observation::Known(known) = work else { return None };
    let marker = CONF_INTEGRITY_MARKER_PATH.as_bytes();
    if !known.unknown.is_empty() || !known.suppressed.is_empty()
        || !known.sparse_absent.is_empty() || known.native_operation.is_some()
        || known.entries.iter().any(|entry| {
            entry.path.as_slice() == marker && entry.kind != WorkKind::Unstaged
        })
    {
        return None;
    }
    if !known.entries.iter().any(|entry| entry.path.as_slice() == marker) {
        return None;
    }
    let head = admitted_marker_head(root)?;
    known.entries.retain(|entry| entry.path.as_slice() != marker);
    Some(head)
}

pub(super) fn admitted_marker_head(root: &Path) -> Option<String> {
    let repo = git2::Repository::open(root).ok()?;
    if repo.state() != git2::RepositoryState::Clean { return None; }
    let head = repo.head().ok()?.peel_to_commit().ok()?;
    let tree = head.tree().ok()?;
    let index = repo.index().ok()?;
    if index.has_conflicts() { return None; }
    for relative in GUARDED_CONF_PATHS.into_iter().chain([CONF_INTEGRITY_MARKER_PATH]) {
        let path = Path::new(relative);
        // Reject symlinks at every component, including the containing directories.
        let mut physical = root.to_path_buf();
        for part in path.components() {
            physical.push(part);
            if fs::symlink_metadata(&physical).ok()?.file_type().is_symlink() {
                return None;
            }
        }
        if !fs::symlink_metadata(&physical).ok()?.is_file() { return None; }
        let indexed = index.get_path(path, 0)?;
        let committed = tree.get_path(path).ok()?;
        if indexed.id != committed.id() || indexed.mode != committed.filemode() as u32
            || indexed.mode != 0o100644 || indexed.flags & 0x8000 != 0
            || indexed.flags_extended != 0
        { return None; }
        if relative != CONF_INTEGRITY_MARKER_PATH {
            let blob = repo.find_blob(indexed.id).ok()?;
            if fs::read(&physical).ok()?.as_slice() != blob.content() { return None; }
        }
    }
    let canonical = canonical_conf_integrity_marker(root).ok()??;
    (fs::read(root.join(CONF_INTEGRITY_MARKER_PATH)).ok()? == canonical.as_bytes()).then(|| head.id().to_string())
}
