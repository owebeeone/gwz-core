//! What `git commit -a` stages, and the index states native refuses.

use std::path::PathBuf;

use super::config_view::{ConfigView, Flag};
use super::route::CliReason;

/// A tracked-file change `git commit -a` stages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Staging {
    Add(PathBuf),
    Remove(PathBuf),
}

const ENTRY_VALID: u16 = 0x8000;
const ENTRY_INTENT_TO_ADD: u16 = 1 << 13;
const ENTRY_SKIP_WORKTREE: u16 = 1 << 14;
const MODE_GITLINK: u32 = 0o160_000;

pub(super) fn index_check(repo: &git2::Repository, all: bool) -> Result<(), CliReason> {
    let index = repo
        .index()
        .map_err(|_| CliReason::IndexState("unreadable"))?;
    if index.has_conflicts() {
        return Err(CliReason::IndexState("unmerged entries"));
    }
    for entry in index.iter() {
        if entry.flags_extended & ENTRY_INTENT_TO_ADD != 0 {
            return Err(CliReason::IndexState("intent-to-add entry"));
        }
        if all {
            if entry.mode == MODE_GITLINK {
                return Err(CliReason::AllCommit("submodule entry"));
            }
            if entry.flags_extended & ENTRY_SKIP_WORKTREE != 0 {
                return Err(CliReason::AllCommit("skip-worktree entry"));
            }
            if entry.flags & ENTRY_VALID != 0 {
                return Err(CliReason::AllCommit("assume-unchanged entry"));
            }
        }
    }
    Ok(())
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// Git for Windows ships `core.autocrlf=true` in a system file libgit2
        /// may not find, so an unset value proves nothing there.
        const AUTOCRLF_UNSET_IS_FALSE: bool = false;
    } else {
        const AUTOCRLF_UNSET_IS_FALSE: bool = true;
    }
}

/// Attributes that can change what `git add` stores for a path.
const CONTENT_ATTRIBUTES: &[&str] = &[
    "text",
    "eol",
    "crlf",
    "filter",
    "ident",
    "working-tree-encoding",
];

fn all_config_check(config: &ConfigView) -> Result<(), CliReason> {
    let filters = config
        .names_matching("^filter\\.")
        .map_err(|_| CliReason::AllCommit("filter configuration unreadable"))?;
    if !filters.is_empty() {
        return Err(CliReason::AllCommit("filter.* configuration"));
    }
    match config.flag("core.autocrlf") {
        Flag::False => {}
        Flag::Unset if AUTOCRLF_UNSET_IS_FALSE => {}
        _ => return Err(CliReason::AllCommit("core.autocrlf")),
    }
    match config.text("core.eol") {
        Ok(None) => {}
        Ok(Some(value)) if value.eq_ignore_ascii_case("lf") => {}
        _ => return Err(CliReason::AllCommit("core.eol")),
    }
    if config.is_set("core.fsmonitor") {
        return Err(CliReason::AllCommit("core.fsmonitor"));
    }
    Ok(())
}

/// The tracked files `git commit -a` would stage, or the reason native will
/// not do it.
pub(super) fn staging_plan(
    repo: &git2::Repository,
    config: &ConfigView,
) -> Result<Vec<Staging>, CliReason> {
    all_config_check(config)?;
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(false)
        .include_ignored(false)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false);
    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|_| CliReason::AllCommit("status unreadable"))?;
    let mut plan = Vec::new();
    for entry in statuses.iter() {
        let status = entry.status();
        let unsupported = git2::Status::WT_TYPECHANGE
            | git2::Status::WT_RENAMED
            | git2::Status::CONFLICTED
            | git2::Status::IGNORED;
        if status.intersects(unsupported) {
            return Err(CliReason::AllCommit("typechange or unreadable path"));
        }
        let wanted = status.intersects(git2::Status::WT_MODIFIED | git2::Status::WT_DELETED);
        if !wanted {
            continue;
        }
        let path = entry
            .path()
            .map(PathBuf::from)
            .map_err(|_| CliReason::AllCommit("path is not UTF-8"))?;
        if status.contains(git2::Status::WT_DELETED) {
            plan.push(Staging::Remove(path));
            continue;
        }
        for attribute in CONTENT_ATTRIBUTES {
            let value = repo
                .get_attr_bytes(&path, attribute, git2::AttrCheckFlags::FILE_THEN_INDEX)
                .map_err(|_| CliReason::AllCommit("attributes unreadable"))?;
            if !matches!(
                git2::AttrValue::from_bytes(value),
                git2::AttrValue::Unspecified
            ) {
                return Err(CliReason::AllCommit("content attributes"));
            }
        }
        plan.push(Staging::Add(path));
    }
    Ok(plan)
}
