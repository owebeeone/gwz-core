//! Bare-operand disambiguation (D5, git's rev/path split).
//!
//! `gwz diff <arg>…` (no `--`) mirrors `git diff`: each positional token is
//! *either* a revision or a pathspec, and git decides per token by asking two
//! questions — does it resolve as a revision, and does it name an existing path
//! (cwd-relative)? This module implements that rule for the GWZ workspace, where
//! "resolves as a revision" spans several candidate repos rather than one.
//!
//! Why here (not in [`parse_comparison`](super::parse_comparison)): classification
//! needs the filesystem (stat the operand cwd-relative) and Git (resolve the
//! operand in the candidate repos). `parse_comparison` is pure over already-split
//! revision tokens, so the handler runs [`classify_operands`] first and feeds the
//! revision half to `parse_comparison` and the pathspec half to
//! [`plan_diff`](super::plan_diff) ahead of the explicit (`--`) pathspecs. The D0
//! wire protocol is unchanged: operands stay raw strings in `DiffRequest`; the
//! split is server-side semantics.
//!
//! ## Rule table (per operand, left to right)
//!
//! A *range* (`A..B` / `A...B`) or a `+snapshot` operand is **never** a path — it
//! is always a revision (git treats `..`/`...` and our `+` snapshot sigil as
//! rev-only syntax). For every other token, with `is_rev` = resolves in ≥1
//! candidate repo and `is_path` = names an existing entry under the cwd:
//!
//! | zone      | is_rev | is_path | outcome                                        |
//! |-----------|--------|---------|------------------------------------------------|
//! | revision  |  yes   |   no    | revision                                       |
//! | revision  |  no    |  yes    | pathspec — *this and every later token* is one |
//! | revision  |  yes   |  yes    | error: ambiguous, suggest `--`                 |
//! | revision  |  no    |   no    | error: unknown revision or path, suggest `--`  |
//! | pathspec  |   –    |   –     | pathspec (a rev-looking, non-path token errors)|
//!
//! Ordering is git's: revisions precede pathspecs; once a token classifies as a
//! pathspec the parser leaves the revision zone and every later token is a
//! pathspec. A later token that looks like a revision but is not an existing path
//! is the ambiguous case again (git's `verify_filename`), so it errors suggesting
//! `--`.
//!
//! ## Multi-repo adaptation
//!
//! - **existing path**: stat the operand cwd-relative using the same
//!   `join_cwd` + `lexical_normalize` route [`route_pathspec`](crate::workspace_ops)
//!   uses, against the *physical* workspace paths. A token that escapes the
//!   workspace (`../..`) is treated as "not a path" (it can never be a workspace
//!   pathspec), deferring to the revision/error arms.
//! - **resolvable revision**: resolves in at least one candidate target repo —
//!   the root repo plus every active, materialized Git member of the default
//!   plan. This matches `plan_diff`'s default candidate set, so a bare branch that
//!   exists in any member (but no file matches) still classifies as a revision.

use std::path::{Path, PathBuf};

use crate::artifact::{ArtifactSourceKind, ManifestArtifact};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::operands::is_never_path_operand;

/// The candidate repositories a bare operand may resolve as a revision in: the
/// workspace root plus each active, materialized Git member. The handler builds
/// this from the manifest + a resolver over real repos; tests build it in-memory.
pub struct RevContext<'a> {
    /// Absolute path of each candidate repo (root first, then members). Only used
    /// by the default resolver; a custom `resolve` may ignore it.
    pub repos: Vec<PathBuf>,
    /// Physical cwd the operands stat against, and the physical workspace root the
    /// stat must stay within.
    pub cwd: PathBuf,
    pub workspace_root: PathBuf,
    /// Resolve a token as a revision in *some* candidate repo: `true` when it
    /// revparses anywhere in `repos`.
    pub resolve: &'a dyn Fn(&[PathBuf], &str) -> ModelResult<bool>,
}

/// The operands split into the revision half (fed to `parse_comparison`) and the
/// pathspec half derived from bare path operands (prepended to the explicit `--`
/// pathspecs by the caller).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClassifiedOperands {
    pub revisions: Vec<String>,
    pub pathspecs: Vec<String>,
}

/// Split `operands` (the raw tokens *before* `--`) into revisions and pathspecs
/// per git's rule table (see the module docs). `ctx` supplies the path-stat and
/// revision-resolution context; `manifest` is unused today but kept on the
/// signature so a future adaptation can consult member metadata without a
/// signature churn.
pub fn classify_operands(
    operands: &[String],
    manifest: &ManifestArtifact,
    ctx: &RevContext<'_>,
) -> ModelResult<ClassifiedOperands> {
    let _ = manifest;
    let mut out = ClassifiedOperands::default();
    let mut in_pathspec_zone = false;

    for operand in operands {
        // Ranges and `+snapshot`s are rev-only syntax in either zone. In the
        // pathspec zone a rev-looking token that is not a path is the ambiguous
        // case → error (git's verify_filename).
        if is_never_path_operand(operand) {
            if in_pathspec_zone {
                return Err(ambiguous_after_path(operand));
            }
            out.revisions.push(operand.clone());
            continue;
        }

        let is_path = operand_is_existing_path(operand, ctx);

        if in_pathspec_zone {
            // Everything after the first pathspec is a pathspec. A token that is
            // not a path but resolves as a revision is git's ambiguous error.
            if !is_path && (ctx.resolve)(&ctx.repos, operand)? {
                return Err(ambiguous_after_path(operand));
            }
            out.pathspecs.push(operand.clone());
            continue;
        }

        let is_rev = (ctx.resolve)(&ctx.repos, operand)?;
        match (is_rev, is_path) {
            (true, false) => out.revisions.push(operand.clone()),
            (false, true) => {
                in_pathspec_zone = true;
                out.pathspecs.push(operand.clone());
            }
            (true, true) => return Err(ambiguous(operand)),
            (false, false) => return Err(unknown_rev_or_path(operand)),
        }
    }

    Ok(out)
}

/// Does `operand` name an existing filesystem entry, cwd-relative, inside the
/// workspace? Uses the same `join_cwd` + `lexical_normalize` route as
/// `route_pathspec`, then stats the physical path. A workspace escape is *not* a
/// path (it can never be a workspace pathspec), so the caller falls through to
/// the revision / error arms.
fn operand_is_existing_path(operand: &str, ctx: &RevContext<'_>) -> bool {
    use crate::workspace_ops::{join_cwd, lexical_normalize};

    let abs = lexical_normalize(&join_cwd(&ctx.cwd, operand));
    // Must stay within the physical workspace root (route_pathspec's escape rule).
    if abs.strip_prefix(&ctx.workspace_root).is_err() {
        return false;
    }
    Path::new(&abs).exists()
}

fn ambiguous(operand: &str) -> ModelError {
    ModelError::new(
        ErrorCode::InvalidRequest,
        format!(
            "ambiguous argument '{operand}': both a revision and a path exist. \
             Use '--' to separate paths from revisions, like this:\n\
             'gwz diff [<revision>...] -- [<file>...]'"
        ),
    )
}

fn ambiguous_after_path(operand: &str) -> ModelError {
    ModelError::new(
        ErrorCode::InvalidRequest,
        format!(
            "ambiguous argument '{operand}': a revision cannot follow a path operand. \
             Use '--' to separate paths from revisions, like this:\n\
             'gwz diff [<revision>...] -- [<file>...]'"
        ),
    )
}

fn unknown_rev_or_path(operand: &str) -> ModelError {
    ModelError::new(
        ErrorCode::InvalidRequest,
        format!(
            "ambiguous argument '{operand}': unknown revision or path not in the working tree. \
             Use '--' to separate paths from revisions, like this:\n\
             'gwz diff [<revision>...] -- [<file>...]'"
        ),
    )
}

/// The default revision resolver: a token resolves when `read_ref` finds it in
/// *any* candidate repo. Repos that fail to open (e.g. a stale worktree) are
/// skipped, not fatal — a bare operand should still classify off the repos that
/// do open. Used by the handler; tests inject their own resolver.
pub fn default_rev_resolver(repos: &[PathBuf], token: &str) -> ModelResult<bool> {
    use crate::git::{Git2Backend, GitBackend};
    let backend = Git2Backend::new();
    for repo in repos {
        if !backend.is_repository(repo).unwrap_or(false) {
            continue;
        }
        if backend.read_ref(repo, token).unwrap_or(None).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Physical candidate repos for revision resolution: the workspace root plus each
/// active, materialized Git member (the default `plan_diff` candidate set). The
/// materialization check is the same filesystem probe the plan uses.
pub fn candidate_repos(root: &Path, manifest: &ManifestArtifact) -> Vec<PathBuf> {
    use crate::git::{Git2Backend, GitBackend};
    let backend = Git2Backend::new();
    let mut repos = vec![root.to_path_buf()];
    for member in &manifest.members {
        if !member.active || member.source_kind != ArtifactSourceKind::Git {
            continue;
        }
        let member_root = root.join(&member.path);
        if backend.is_repository(&member_root).unwrap_or(false) {
            repos.push(member_root);
        }
    }
    repos
}
