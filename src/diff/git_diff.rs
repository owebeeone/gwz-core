//! The D1 Git backend diff primitive over a single libgit2 repository.
//!
//! [`resolve_comparison`] turns raw per-repo revision tokens into concrete
//! libgit2 tree sides (the [`RepoDiffComparison`] the diff engine consumes),
//! and [`diff_repo`] runs the matching libgit2 diff, applies rename detection,
//! and reports a repo-scoped [`RepoDiffManifest`]. Both operate on one already
//! opened repository; the workspace-level fan-out, member-prefix rewriting, and
//! `gwz.conf` exclusion are the D2 planner's responsibility.

use git2::{
    Delta, Diff, DiffFindOptions, DiffOptions as Git2DiffOptions, FileMode, Repository, Tree,
};

use crate::git::git_error;
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::model::{
    RepoDiffAlgorithm, RepoDiffComparison, RepoDiffComparisonKind, RepoDiffEntry, RepoDiffManifest,
    RepoDiffOptions, RepoDiffStatus, RepoDiffWhitespace,
};

/// The per-repo comparison request before tree resolution: the raw revision
/// tokens as classified for *this* repository, plus the parsed `--cached` /
/// `--merge-base` intent. D2 builds these from operands and snapshot resolution;
/// [`resolve_comparison`] lowers them to a [`RepoDiffComparison`] with concrete
/// tree oids.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ComparisonSpec {
    pub kind: RepoDiffComparisonKind,
    /// Left/old-side revision token (branch, tag, commit-ish). `None` means the
    /// default old side for the kind (`HEAD` for `--cached`/`<commit>` forms).
    pub left: Option<String>,
    /// Right/new-side revision token, for the two-tree form.
    pub right: Option<String>,
    /// Use `merge-base(left, right-or-HEAD)` as the old side (`--merge-base` and
    /// the `A...B` range form).
    pub merge_base: bool,
}

/// Resolve a per-repo comparison spec to concrete libgit2 tree sides.
///
/// Follows the plan's comparison table: `--cached` old side is `HEAD` (or the
/// empty tree when unborn), a `<commit>` old side peels to that commit's tree, a
/// two-tree form peels both sides (an omitted side falls back to `HEAD`), and a
/// merge-base old side resolves `merge-base(left, right-or-HEAD)`. A `<commit>`
/// form against an unborn repo is a typed [`ErrorCode::GitCommandFailed`] error.
pub fn resolve_comparison(
    repo: &Repository,
    spec: &ComparisonSpec,
) -> ModelResult<RepoDiffComparison> {
    match spec.kind {
        RepoDiffComparisonKind::WorktreeVsIndex => Ok(RepoDiffComparison {
            kind: RepoDiffComparisonKind::WorktreeVsIndex,
            old_tree: None,
            new_tree: None,
        }),
        RepoDiffComparisonKind::IndexVsTree => {
            // `--cached [<commit>]`: old side is <commit> or HEAD; unborn HEAD is
            // the empty tree.
            let old_tree = resolve_old_tree_side(repo, spec, /*allow_unborn_empty=*/ true)?;
            Ok(RepoDiffComparison {
                kind: RepoDiffComparisonKind::IndexVsTree,
                old_tree,
                new_tree: None,
            })
        }
        RepoDiffComparisonKind::WorktreeVsTree => {
            // `git diff <commit>`: the old side must resolve; an unborn repo has
            // no commit to name and is a member-scoped error.
            let old_tree = resolve_old_tree_side(repo, spec, /*allow_unborn_empty=*/ false)?;
            Ok(RepoDiffComparison {
                kind: RepoDiffComparisonKind::WorktreeVsTree,
                old_tree,
                new_tree: None,
            })
        }
        RepoDiffComparisonKind::TreeVsTree => {
            let left = spec.left.as_deref().unwrap_or("HEAD");
            let right = spec.right.as_deref().unwrap_or("HEAD");
            let left_tree = resolve_tree_oid(repo, left)?;
            let right_tree = resolve_tree_oid(repo, right)?;
            let old_tree = if spec.merge_base {
                let base = merge_base_tree(repo, left, right)?;
                Some(base)
            } else {
                Some(left_tree)
            };
            Ok(RepoDiffComparison {
                kind: RepoDiffComparisonKind::TreeVsTree,
                old_tree,
                new_tree: Some(right_tree),
            })
        }
    }
}

/// Resolve the old-side tree for `--cached`/`<commit>` forms. When
/// `allow_unborn_empty`, an unresolvable default `HEAD` (unborn branch) yields
/// the empty tree (`None`); otherwise it is a member-scoped error.
fn resolve_old_tree_side(
    repo: &Repository,
    spec: &ComparisonSpec,
    allow_unborn_empty: bool,
) -> ModelResult<Option<String>> {
    match &spec.left {
        Some(token) => {
            if spec.merge_base {
                // `--cached --merge-base <commit>`: old side is
                // merge-base(<commit>, HEAD).
                Ok(Some(merge_base_tree(repo, token, "HEAD")?))
            } else {
                Ok(Some(resolve_tree_oid(repo, token)?))
            }
        }
        None => match repo.head() {
            Ok(head) => {
                let commit = head.peel_to_commit().map_err(git_error)?;
                Ok(Some(commit.tree().map_err(git_error)?.id().to_string()))
            }
            Err(err)
                if allow_unborn_empty
                    && matches!(
                        err.code(),
                        git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
                    ) =>
            {
                // Unborn HEAD under --cached: compare the empty tree to the index.
                Ok(None)
            }
            Err(err)
                if matches!(
                    err.code(),
                    git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
                ) =>
            {
                Err(ModelError::new(
                    ErrorCode::GitCommandFailed,
                    "cannot diff against HEAD in a repository with no commits",
                ))
            }
            Err(err) => Err(git_error(err)),
        },
    }
}

/// Peel a revision token to its tree oid (hex).
fn resolve_tree_oid(repo: &Repository, token: &str) -> ModelResult<String> {
    let object = repo.revparse_single(token).map_err(git_error)?;
    let tree = object.peel_to_tree().map_err(git_error)?;
    Ok(tree.id().to_string())
}

/// Resolve `merge-base(left, right)` and return its tree oid. A missing merge
/// base (unrelated histories) is a member-scoped error.
fn merge_base_tree(repo: &Repository, left: &str, right: &str) -> ModelResult<String> {
    let left_commit = repo
        .revparse_single(left)
        .and_then(|o| o.peel_to_commit())
        .map_err(git_error)?;
    let right_commit = repo
        .revparse_single(right)
        .and_then(|o| o.peel_to_commit())
        .map_err(git_error)?;
    let base = repo
        .merge_base(left_commit.id(), right_commit.id())
        .map_err(|err| {
            if err.code() == git2::ErrorCode::NotFound {
                ModelError::new(
                    ErrorCode::GitCommandFailed,
                    format!("no merge base between '{left}' and '{right}'"),
                )
            } else {
                git_error(err)
            }
        })?;
    let base_commit = repo.find_commit(base).map_err(git_error)?;
    Ok(base_commit.tree().map_err(git_error)?.id().to_string())
}

/// Run the resolved comparison over one repository and build its changed-file
/// manifest.
pub fn diff_repo(
    repo: &Repository,
    comparison: &RepoDiffComparison,
    options: &RepoDiffOptions,
) -> ModelResult<RepoDiffManifest> {
    let diff = build_repo_diff(repo, comparison, options, |_| {})?;
    build_manifest(&diff)
}

/// Build the raw libgit2 [`Diff`] for a resolved comparison, with rename
/// detection applied when requested. `customize` is handed the mapped
/// [`Git2DiffOptions`] before the diff runs so a caller (the D4 renderer) can set
/// presentation-only knobs — `old_prefix`/`new_prefix`, `show_binary`, and
/// narrowing pathspecs — without duplicating the whitespace/algorithm mapping or
/// the comparison-kind dispatch. Callers that only need the manifest use
/// [`diff_repo`] and ignore this.
pub(crate) fn build_repo_diff<'repo>(
    repo: &'repo Repository,
    comparison: &RepoDiffComparison,
    options: &RepoDiffOptions,
    customize: impl FnOnce(&mut Git2DiffOptions),
) -> ModelResult<Diff<'repo>> {
    let mut diff = run_git2_diff(repo, comparison, options, customize)?;

    if options.find_renames {
        let mut find = DiffFindOptions::new();
        find.renames(true);
        // v0 never enables copy detection here; find_copies=true is rejected up
        // front (reject_unsupported_options).
        find.copies(false);
        if let Some(threshold) = options.rename_threshold {
            find.rename_threshold(threshold);
        }
        if let Some(limit) = options.rename_limit {
            find.rename_limit(limit);
        }
        diff.find_similar(Some(&mut find)).map_err(git_error)?;
    }

    Ok(diff)
}

/// Resolve the tree(s) referenced by the comparison and dispatch to the matching
/// libgit2 diff call. `customize` post-adjusts the mapped options (prefixes /
/// binary / narrowing pathspecs) before the diff runs.
fn run_git2_diff<'repo>(
    repo: &'repo Repository,
    comparison: &RepoDiffComparison,
    options: &RepoDiffOptions,
    customize: impl FnOnce(&mut Git2DiffOptions),
) -> ModelResult<Diff<'repo>> {
    let mut opts = build_git2_options(options);
    customize(&mut opts);
    let old_tree = lookup_tree(repo, comparison.old_tree.as_deref())?;
    match comparison.kind {
        RepoDiffComparisonKind::WorktreeVsIndex => repo
            .diff_index_to_workdir(None, Some(&mut opts))
            .map_err(git_error),
        RepoDiffComparisonKind::IndexVsTree => repo
            .diff_tree_to_index(old_tree.as_ref(), None, Some(&mut opts))
            .map_err(git_error),
        RepoDiffComparisonKind::WorktreeVsTree => repo
            .diff_tree_to_workdir_with_index(old_tree.as_ref(), Some(&mut opts))
            .map_err(git_error),
        RepoDiffComparisonKind::TreeVsTree => {
            let new_tree = lookup_tree(repo, comparison.new_tree.as_deref())?;
            repo.diff_tree_to_tree(old_tree.as_ref(), new_tree.as_ref(), Some(&mut opts))
                .map_err(git_error)
        }
    }
}

/// Look up a tree by hex oid; `None` (empty-tree side) yields `None`.
fn lookup_tree<'repo>(
    repo: &'repo Repository,
    oid: Option<&str>,
) -> ModelResult<Option<Tree<'repo>>> {
    match oid {
        Some(oid) => {
            let oid = git2::Oid::from_str(oid).map_err(git_error)?;
            Ok(Some(repo.find_tree(oid).map_err(git_error)?))
        }
        None => Ok(None),
    }
}

/// Map the internal option knobs onto a git2 [`Git2DiffOptions`].
fn build_git2_options(options: &RepoDiffOptions) -> Git2DiffOptions {
    let mut opts = Git2DiffOptions::new();
    // Emit type-change deltas as their own status rather than add+delete pairs;
    // Git shows these by default.
    opts.include_typechange(options.include_typechange);
    opts.reverse(options.reverse);
    opts.force_text(options.force_text);

    if let Some(context) = options.context_lines {
        opts.context_lines(context);
    }
    if let Some(interhunk) = options.interhunk_lines {
        opts.interhunk_lines(interhunk);
    }
    match options.algorithm {
        RepoDiffAlgorithm::Default | RepoDiffAlgorithm::Myers => {}
        RepoDiffAlgorithm::Minimal => {
            opts.minimal(true);
        }
        RepoDiffAlgorithm::Patience => {
            opts.patience(true);
        }
    }
    match options.whitespace {
        RepoDiffWhitespace::Default => {}
        RepoDiffWhitespace::IgnoreAll => {
            opts.ignore_whitespace(true);
        }
        RepoDiffWhitespace::IgnoreChange => {
            opts.ignore_whitespace_change(true);
        }
        RepoDiffWhitespace::IgnoreEol => {
            opts.ignore_whitespace_eol(true);
        }
        RepoDiffWhitespace::IgnoreBlankLines => {
            opts.ignore_blank_lines(true);
        }
    }
    for pathspec in &options.pathspecs {
        opts.pathspec(pathspec.as_str());
    }
    opts
}

/// Walk the diff deltas into repo-scoped manifest entries with per-file line
/// stats, preserving libgit2's delta ordering.
///
/// Each delta's per-file patch is generated once via [`git2::Patch::from_diff`].
/// That patch is the source of truth for three things libgit2 only settles
/// during patch generation, not on the bare delta: the binary flag, the
/// per-file line stats, and (for renames) the exact `similarity index`.
fn build_manifest(diff: &Diff<'_>) -> ModelResult<RepoDiffManifest> {
    let mut manifest = RepoDiffManifest::default();
    for (idx, delta) in diff.deltas().enumerate() {
        let status = match map_status(delta.status()) {
            Some(status) => status,
            // Unmodified/ignored/untracked/unreadable deltas are never surfaced
            // by `git diff` in the modes D1 targets; skip them defensively.
            None => continue,
        };

        let old_file = delta.old_file();
        let new_file = delta.new_file();
        let old_path = old_file.path().map(path_to_string);
        let new_path = new_file.path().map(path_to_string);
        let old_mode = mode_octal(old_file.mode(), old_file.exists());
        let new_mode = mode_octal(new_file.mode(), new_file.exists());

        // Generate the per-file patch: it settles the binary flag and yields
        // workspace-correct line stats and the rename similarity header.
        let mut patch = git2::Patch::from_diff(diff, idx).map_err(git_error)?;
        let is_binary = patch
            .as_ref()
            .map(|p| p.delta().flags().is_binary())
            .unwrap_or_else(|| delta.flags().is_binary());

        let (insertions, deletions) = if is_binary {
            (None, None)
        } else {
            match patch.as_ref() {
                Some(patch) => {
                    let (_context, adds, dels) = patch.line_stats().map_err(git_error)?;
                    manifest.insertions += adds;
                    manifest.deletions += dels;
                    (Some(adds), Some(dels))
                }
                None => (Some(0), Some(0)),
            }
        };

        let similarity = if matches!(status, RepoDiffStatus::Renamed) {
            Some(rename_similarity(patch.as_mut())?)
        } else {
            None
        };

        manifest.entries.push(RepoDiffEntry {
            status,
            old_path,
            new_path,
            old_mode,
            new_mode,
            similarity,
            insertions,
            deletions,
            is_binary,
        });
    }
    Ok(manifest)
}

/// Map a libgit2 [`Delta`] status onto the internal status. Returns `None` for
/// statuses `git diff` does not surface in D1's comparison modes.
fn map_status(delta: Delta) -> Option<RepoDiffStatus> {
    match delta {
        Delta::Added => Some(RepoDiffStatus::Added),
        Delta::Deleted => Some(RepoDiffStatus::Deleted),
        Delta::Modified => Some(RepoDiffStatus::Modified),
        Delta::Renamed => Some(RepoDiffStatus::Renamed),
        Delta::Typechange => Some(RepoDiffStatus::TypeChanged),
        Delta::Conflicted => Some(RepoDiffStatus::Unmerged),
        // Copied is rejected in v0 (find_copies never enabled); the rest are not
        // diff output.
        Delta::Copied
        | Delta::Unmodified
        | Delta::Ignored
        | Delta::Untracked
        | Delta::Unreadable => None,
    }
}

/// git2 0.21 does not expose `git_diff_delta.similarity` (the wrapper's
/// `Binding::raw` is crate-private), so read the exact libgit2-computed value
/// from the rename patch header line `similarity index NN%`. This is the same
/// number `git diff -M` prints. A pure rename with no content change (no textual
/// patch) is 100%.
fn rename_similarity(patch: Option<&mut git2::Patch<'_>>) -> ModelResult<u16> {
    let Some(patch) = patch else {
        return Ok(100);
    };
    let buf = patch.to_buf().map_err(git_error)?;
    let text = String::from_utf8_lossy(&buf);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("similarity index ") {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(value) = digits.parse::<u16>() {
                return Ok(value.min(100));
            }
        }
    }
    // A rename with identical content emits no `similarity index` line.
    Ok(100)
}

/// Raw octal Git file mode for a diff side, or `None` when the side is absent.
fn mode_octal(mode: FileMode, exists: bool) -> Option<u32> {
    if !exists {
        return None;
    }
    match mode {
        FileMode::Blob => Some(0o100_644),
        FileMode::BlobGroupWritable => Some(0o100_664),
        FileMode::BlobExecutable => Some(0o100_755),
        FileMode::Link => Some(0o120_000),
        FileMode::Commit => Some(0o160_000),
        FileMode::Tree => Some(0o040_000),
        FileMode::Unreadable => None,
    }
}

fn path_to_string(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Reject option combinations the v0 backend cannot honor, per the D0 error
/// taxonomy: `find_copies=true` and any unsupported diff algorithm reuse
/// [`ErrorCode::UnsupportedOperation`] with the offending option named in the
/// message. Call this at request-planning time before building
/// [`RepoDiffOptions`].
pub fn reject_unsupported_options(
    find_copies: Option<bool>,
    algorithm: Option<crate::protocol::generated::DiffAlgorithm>,
) -> ModelResult<()> {
    if find_copies == Some(true) {
        return Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "unsupported diff option 'find_copies': copy detection is not implemented in v0",
        ));
    }
    // All wire DiffAlgorithm values map to a supported git2 setter; histogram is
    // absent from the enum entirely. This guard remains a forward-compatible
    // hook for any future unsupported algorithm value.
    let _ = algorithm;
    Ok(())
}
