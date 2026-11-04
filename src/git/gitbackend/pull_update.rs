//! Pull's worktree contract (gwz 1.0.19): integrate while untracked files the
//! update does not touch stay where they are, and name exactly the entries the
//! update would overwrite so the caller can refuse. Pull deletes and overwrites
//! nothing of its own: an entry in the update's way, untracked or ignored, file,
//! directory or symlink, refuses the pull before any participant moves, and the
//! libgit2 checkouts pull runs do not overwrite an ignored entry either, so a race
//! between the check and the update fails safe.
//!
//! Tracked changes (staged, unstaged, unresolved) still refuse, as they do for
//! `git pull`; callers check them before asking for collisions.

use super::merge_prepared::{
    execute_prepared_merge_upstream_checked_with, prepare_merge_upstream_checked_with,
    validate_prepared_merge_upstream_state_with,
};
use super::repository_support::open_repo;
use super::*;

mod collisions;
use collisions::{blocked_entries, replay_blocked};

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod checkout_tests;
        mod test_dispatch;
    }
}

/// Which worktree entries a checked update treats as blocking dirt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorktreeRule {
    /// Any staged, unstaged, unresolved or untracked entry blocks (the
    /// released merge rule).
    RequireClean,
    /// Only tracked changes block. Before the checkout the entries in the way are
    /// re-checked, and any one refuses before anything is written; the checkout
    /// itself overwrites no ignored entry.
    AllowUntracked,
}

impl WorktreeRule {
    pub(super) fn blocks(self, status: &GitStatus) -> bool {
        match self {
            Self::RequireClean => status.is_dirty,
            Self::AllowUntracked => status.has_tracked_changes(),
        }
    }

    /// Re-check, under the branch lock, that nothing is in the way of checking out
    /// `tree` (pull's rule only).
    pub(super) fn before_checkout(
        self,
        repo: &git2::Repository,
        path: &Path,
        tree: &git2::Tree<'_>,
    ) -> ModelResult<()> {
        match self {
            Self::RequireClean => Ok(()),
            Self::AllowUntracked => refuse_collisions(
                &blocked_entries(repo, path, tree, None)?,
                "would be overwritten by pull",
            ),
        }
    }

    /// The safe checkout of `object` the update runs. Under pull's rule libgit2
    /// treats an ignored entry at a path it writes as a conflict
    /// (`GIT_CHECKOUT_DONT_OVERWRITE_IGNORED`): its conflict pass runs before it
    /// writes or removes anything, so the checkout fails with "conflict prevents
    /// checkout" and leaves the worktree as it was. The check before it already
    /// refused every such entry, so this only decides what a race does.
    pub(super) fn checkout_tree(
        self,
        repo: &git2::Repository,
        object: &git2::Object<'_>,
    ) -> Result<(), git2::Error> {
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        if self == Self::AllowUntracked {
            checkout.overwrite_ignored(false);
        }
        repo.checkout_tree(object, Some(&mut checkout))
    }
}

/// One worktree entry, not tracked, that is in the way of an update: untracked or
/// ignored, a file, a directory or a symlink.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct GitUpdateCollision {
    /// The incoming path it blocks, as git names it.
    pub path: String,
    /// Why the pull refuses with its own message instead of the plain list (the
    /// entry is tracked, holds a repository, or cannot be inspected), when that is
    /// so. The reason carries its own guidance.
    pub reason: Option<String>,
}

mod sealed {
    /// `GitPullUpdate` is implemented only inside gwz-core: its shape follows
    /// the libgit2 dry run and is expected to move with the forward-port.
    pub(crate) trait Sealed {}
}
pub(crate) use sealed::Sealed as GitPullUpdateSealed;

impl GitPullUpdateSealed for Git2Backend {}

/// The worktree operations `gwz pull` needs beyond `GitRepository` (sealed).
///
/// A backend that models none of this keeps the strict rule: it reports no
/// collisions, and its prepare step (the clean-worktree merge pipeline) refuses
/// any untracked entry.
#[allow(private_bounds)]
pub trait GitPullUpdate: GitRepository + GitPullUpdateSealed {
    /// The entries, neither tracked nor in the index, in the way of a checkout
    /// from HEAD to `target` (a commit, or the tree a merge will check out),
    /// sorted by the incoming path. The rule is git's `verify_absent`, with the
    /// filesystem as the authority: whatever it resolves at an incoming path, or
    /// at a parent of one as a non-directory, is in the way unless it is proven
    /// to be a tracked entry the update removes (the entry, as spelled on disk, is
    /// an index path; never a shared file identity, which a hard link has too),
    /// plus whatever libgit2's own safe checkout refuses. So a case variant
    /// collides exactly where the filesystem folds case, and a case alias of a
    /// tracked entry the update keeps is reported with a reason. Ignored entries
    /// count like any other: pull replaces nothing. Nothing is written, not even
    /// the index.
    fn update_collisions(
        &self,
        _path: &Path,
        _target: &str,
    ) -> ModelResult<Vec<GitUpdateCollision>> {
        Ok(Vec::new())
    }

    /// The entries in the way of the local commits a rebase onto `onto`
    /// replays (the non-merge commits reachable from `local` and not from
    /// `onto`): for each, the collisions of its tree at the paths it adds or
    /// changes against its parent, by incoming path.
    fn replay_collisions(
        &self,
        _path: &Path,
        _onto: &str,
        _local: &str,
    ) -> ModelResult<Vec<GitUpdateCollision>> {
        Ok(Vec::new())
    }

    /// `prepare_merge_upstream_checked` with untracked entries tolerated.
    fn prepare_pull_update_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
    ) -> ModelResult<GitPreparedMerge> {
        self.prepare_merge_upstream_checked(path, branch, expected_before, source_commit, None)
    }

    /// `validate_prepared_merge_upstream_state` with untracked entries tolerated.
    fn validate_prepared_pull_update(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<()> {
        self.validate_prepared_merge_upstream_state(
            path,
            branch,
            expected_before,
            source_commit,
            prepared,
        )
    }

    /// `execute_prepared_merge_upstream_checked` with untracked entries
    /// tolerated. After the final validation, under the branch lock, the entries
    /// in the way are checked once more and the checkout stays safe.
    fn execute_prepared_pull_update(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        message: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult> {
        self.execute_prepared_merge_upstream_checked(
            path,
            branch,
            expected_before,
            source_commit,
            message,
            prepared,
        )
    }
}

impl GitPullUpdate for Git2Backend {
    fn update_collisions(&self, path: &Path, target: &str) -> ModelResult<Vec<GitUpdateCollision>> {
        let repo = open_repo(path)?;
        let tree = target_tree(&repo, target)?;
        blocked_entries(&repo, path, &tree, None)
    }

    fn replay_collisions(
        &self,
        path: &Path,
        onto: &str,
        local: &str,
    ) -> ModelResult<Vec<GitUpdateCollision>> {
        let repo = open_repo(path)?;
        replay_blocked(&repo, path, onto, local)
    }

    fn prepare_pull_update_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
    ) -> ModelResult<GitPreparedMerge> {
        prepare_merge_upstream_checked_with(
            self,
            path,
            branch,
            expected_before,
            source_commit,
            GitPreparedMergeMode::AllowFastForward,
            None,
            WorktreeRule::AllowUntracked,
        )
    }

    fn validate_prepared_pull_update(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<()> {
        validate_prepared_merge_upstream_state_with(
            self,
            path,
            branch,
            expected_before,
            source_commit,
            prepared,
            WorktreeRule::AllowUntracked,
        )
    }

    fn execute_prepared_pull_update(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        message: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult> {
        execute_prepared_merge_upstream_checked_with(
            self,
            path,
            branch,
            expected_before,
            source_commit,
            message,
            prepared,
            WorktreeRule::AllowUntracked,
        )
    }
}

fn target_tree<'r>(repo: &'r git2::Repository, target: &str) -> ModelResult<git2::Tree<'r>> {
    let oid = git2::Oid::from_str(target).map_err(git_error)?;
    repo.find_object(oid, None)
        .and_then(|object| object.peel_to_tree())
        .map_err(git_error)
}

/// Refuse when `collisions` (the entries in an update's way) is not empty. An entry
/// pull cannot continue past is named first, with its own reason; otherwise every
/// path is listed (`detail` says why) with the advice to move or delete them. Pull
/// deletes none of them, with or without `--force`, so no message mentions it.
pub(crate) fn refuse_collisions(
    collisions: &[GitUpdateCollision],
    detail: &str,
) -> ModelResult<()> {
    if let Some((path, reason)) = collisions.iter().find_map(|collision| {
        collision
            .reason
            .as_ref()
            .map(|reason| (&collision.path, reason))
    }) {
        return Err(ModelError::new(
            ErrorCode::DirtyMember,
            format!("pull cannot continue, because of what is in the way of '{path}': {reason}"),
        ));
    }
    if collisions.is_empty() {
        return Ok(());
    }
    let paths = collisions
        .iter()
        .map(|collision| collision.path.clone())
        .collect::<Vec<_>>();
    Err(ModelError::new(
        ErrorCode::DirtyMember,
        format!(
            "untracked or ignored working tree entries {detail}: {}; move or delete them, \
             then run pull again",
            listed_paths(&paths)
        ),
    ))
}

/// How many colliding paths a refusal names before it gives only the count.
const LISTED_COLLISIONS: usize = 10;

/// `a, b, c` up to `LISTED_COLLISIONS`, then `(and N more; M in all)`.
fn listed_paths(paths: &[String]) -> String {
    let listed = paths
        .iter()
        .take(LISTED_COLLISIONS)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if paths.len() > LISTED_COLLISIONS {
        format!(
            "{listed} (and {} more; {} in all)",
            paths.len() - LISTED_COLLISIONS,
            paths.len()
        )
    } else {
        listed
    }
}
