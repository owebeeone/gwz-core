//! Shared workspace pathspec → owning-repo routing.
//!
//! The GWZ workspace is nested repos: the root repository plus materialized
//! members at `root/<member_path>`. A pathspec is owned by the *innermost* repo
//! whose directory contains it. `gwz add` (stage), `gwz diff` and `gwz log` need
//! the same primitive — resolve a raw pathspec cwd-relative, reject escapes,
//! find the owning repo, and strip the member prefix — but they layer different
//! selection/ordering semantics on top (stage fans `.` out into members and
//! orders with a `BTreeMap`; diff intersects a pre-computed candidate set and
//! orders root-first then manifest order). This module owns *only* the routing
//! primitive; the callers own their own semantics.
//!
//! Extracted from `stage_routing.rs` (D2) so the callers share one routing
//! implementation: [`workspace_relative_operand`] resolves an operand into the
//! workspace, and [`route_workspace_path`] maps the result to its owning repo.
//! Containment is physical, so only the first step reads the filesystem.

use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::{canonical_existing_path, lexical_normalize, physical_spelling};

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

/// Resolve the raw operand `spec` from `cwd` (like `git add`/`git diff`) and
/// express it relative to the workspace `root`.
///
/// Containment is decided by physical identity, so `root`, `cwd` and an absolute
/// `spec` may each be spelled through symbolic links (macOS's `/tmp` is
/// `/private/tmp`). The operand is first normalized lexically, as Git does. It
/// lies in the workspace when it starts with the root's given or physical
/// spelling or, as in Git's `abspath_part_inside_repo`, when one of its leading
/// directories resolves to the physical root; the shortest such directory
/// anchors it. The remainder keeps the operand's own spelling, so a pathspec
/// goes on naming tree paths rather than a link's target.
///
/// A link inside the workspace must not carry the operand out of it: the
/// remainder's leading directories must resolve inside the physical root, and a
/// link among them that does not resolve refuses. The final component is never
/// followed, so an operand naming a link still names the link. Every refusal is
/// [`ErrorCode::PathEscape`], reporting the spelling, the resolved path, the
/// caller directory and the allowed root.
pub(crate) fn workspace_relative_operand(
    root: &Path,
    cwd: &Path,
    spec: &str,
) -> ModelResult<PathBuf> {
    let given_root = lexical_normalize(root);
    let root = physical_spelling(&given_root).unwrap_or_else(|| given_root.clone());
    let cwd = lexical_normalize(cwd);
    let cwd = physical_spelling(&cwd).unwrap_or(cwd);
    let abs = lexical_normalize(&join_cwd(&cwd, spec));
    let allowed_root = || {
        if given_root == root {
            root.display().to_string()
        } else {
            format!("{} (physically {})", given_root.display(), root.display())
        }
    };
    let Some(relative) = anchor_in_root(&root, &given_root, &abs) else {
        return Err(ModelError::new(
            ErrorCode::PathEscape,
            format!(
                "pathspec {spec:?} resolved to {} from caller directory {}, outside the allowed workspace root {}. --root selects the workspace; it does not change the base of relative operands.",
                abs.display(),
                cwd.display(),
                allowed_root()
            ),
        ));
    };
    if !leading_directories_inside(&root, &relative) {
        return Err(ModelError::new(
            ErrorCode::PathEscape,
            format!(
                "pathspec {spec:?} resolved to {} from caller directory {}, but a symbolic link inside the allowed workspace root {} leads outside it or does not resolve.",
                abs.display(),
                cwd.display(),
                allowed_root()
            ),
        ));
    }
    Ok(relative)
}

/// The part of `abs` inside the workspace: after the root's physical or given
/// spelling, or else after the shortest leading directory of `abs` that resolves
/// to the physical `root`. `None` when no leading directory is the root.
fn anchor_in_root(root: &Path, given_root: &Path, abs: &Path) -> Option<PathBuf> {
    if let Some(relative) = [root, given_root]
        .into_iter()
        .find_map(|base| abs.strip_prefix(base).ok())
    {
        return Some(relative.to_path_buf());
    }
    let mut ancestors: Vec<&Path> = abs.ancestors().collect();
    ancestors.reverse();
    // A leading directory that does not resolve has no resolvable descendants.
    ancestors
        .into_iter()
        .map_while(|ancestor| Some((ancestor, canonical_existing_path(ancestor)?)))
        .find(|(_, physical)| physical.as_path() == root)
        .and_then(|(anchor, _)| abs.strip_prefix(anchor).ok())
        .map(Path::to_path_buf)
}

/// Whether the leading directories of the workspace-relative `relative` resolve
/// inside the physical `root`. Its final component is not followed.
fn leading_directories_inside(root: &Path, relative: &Path) -> bool {
    match relative.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            physical_spelling(&root.join(parent)).is_some_and(|physical| physical.starts_with(root))
        }
        _ => true,
    }
}

/// Route a workspace-relative path, as [`workspace_relative_operand`] returns
/// it, to the innermost repo that owns it: the member whose path is the longest
/// component-wise prefix of `relative`, or the root when no member contains it.
/// The returned pathspec is repo-relative with the member prefix stripped.
pub(crate) fn route_workspace_path(member_paths: &[String], relative: &Path) -> RoutedPathspec {
    match owning_member(member_paths, relative) {
        Some(member) => {
            let inner = relative.strip_prefix(&member).unwrap_or(relative);
            RoutedPathspec {
                member_path: Some(member),
                pathspec: pathspec_str(inner),
            }
        }
        None => RoutedPathspec {
            member_path: None,
            pathspec: pathspec_str(relative),
        },
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

/// Repo-relative pathspec string: the repo root itself (empty) becomes ".", and
/// path separators are normalized to `/` for Git.
pub(crate) fn pathspec_str(rel: &Path) -> String {
    if rel.as_os_str().is_empty() {
        return ".".to_owned();
    }
    rel.to_string_lossy().replace('\\', "/")
}
