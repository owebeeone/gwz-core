//! Shared workspace pathspec → owning-repo routing.
//!
//! The GWZ workspace is nested repos: the root repository plus materialized
//! members at `root/<member_path>`. A pathspec is owned by the *innermost* repo
//! whose directory contains it. Both `gwz add` (stage) and `gwz diff` need the
//! same primitive — resolve a raw pathspec cwd-relative, reject escapes, find
//! the owning repo, and strip the member prefix — but they layer different
//! selection/ordering semantics on top (stage fans `.` out into members and
//! orders with a `BTreeMap`; diff intersects a pre-computed candidate set and
//! orders root-first then manifest order). This module owns *only* the routing
//! primitive; the callers own their own semantics.
//!
//! Extracted from `stage_routing.rs` (D2) so the two callers share one routing
//! implementation. `resolve_stage_targets` is now a thin wrapper over
//! [`route_pathspec`]; the diff planner (`crate::diff::plan`) is the second
//! caller. Pure — no filesystem access.

use std::path::{Component, Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};

/// Which repo owns a routed pathspec, plus the pathspec rewritten repo-relative.
///
/// `member_path == None` is the workspace root repo; `Some(path)` is the member
/// at `root/<path>`, with the member prefix already stripped from `pathspec`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RoutedPathspec {
    /// `None` = root repo; `Some(member_path)` = that member.
    pub member_path: Option<String>,
    /// Repo-relative pathspec (member prefix stripped; the repo root itself is
    /// `"."`), separators normalized to `/`.
    pub pathspec: String,
}

/// Route one raw pathspec to the innermost repo that owns it.
///
/// `spec` is resolved relative to `cwd` (like `git add`/`git diff`), lexically
/// normalized, and required to stay inside `root` (otherwise
/// [`ErrorCode::PathEscape`]). The owning repo is the member whose path is the
/// longest component-wise prefix of the resolved path, or the root when no
/// member contains it. The returned pathspec is repo-relative with the member
/// prefix stripped.
pub(crate) fn route_pathspec(
    root: &Path,
    member_paths: &[String],
    cwd: &Path,
    spec: &str,
) -> ModelResult<RoutedPathspec> {
    let abs = lexical_normalize(&join_cwd(cwd, spec));
    let rel = abs.strip_prefix(root).map_err(|_| {
        ModelError::new(
            ErrorCode::PathEscape,
            format!("pathspec '{spec}' is outside the workspace"),
        )
    })?;

    match owning_member(member_paths, rel) {
        Some(member) => {
            let inner = rel.strip_prefix(&member).unwrap_or(rel);
            Ok(RoutedPathspec {
                member_path: Some(member),
                pathspec: pathspec_str(inner),
            })
        }
        None => Ok(RoutedPathspec {
            member_path: None,
            pathspec: pathspec_str(rel),
        }),
    }
}

/// The innermost member whose path is a component-wise prefix of `rel`, or
/// `None` when the path is root territory. Component-wise so `gwz-cli` does not
/// falsely capture a sibling `gwz-client/...`.
pub(crate) fn owning_member(member_paths: &[String], rel: &Path) -> Option<String> {
    member_paths
        .iter()
        .filter(|member| rel.starts_with(member.as_str()))
        .max_by_key(|member| Path::new(member.as_str()).components().count())
        .cloned()
}

/// Join `spec` onto `cwd` unless it is already absolute.
pub(crate) fn join_cwd(cwd: &Path, spec: &str) -> PathBuf {
    let path = Path::new(spec);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// Lexically resolve `.` / `..` without touching the filesystem (unlike
/// `normalize_path`, which canonicalizes — wrong for not-yet-existing or deleted
/// paths, and for symlinks).
pub(crate) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(value) => out.push(value),
            Component::RootDir | Component::Prefix(_) => out.push(component.as_os_str()),
        }
    }
    out
}

/// Repo-relative pathspec string: the repo root itself (empty) becomes ".", and
/// path separators are normalized to `/` for Git.
pub(crate) fn pathspec_str(rel: &Path) -> String {
    if rel.as_os_str().is_empty() {
        return ".".to_owned();
    }
    rel.to_string_lossy().replace('\\', "/")
}
