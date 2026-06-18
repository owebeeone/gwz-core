use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};


use super::*;

pub trait GitBackend {
    fn is_repository(&self, path: &Path) -> ModelResult<bool>;
    fn create_repo(&self, path: &Path) -> ModelResult<GitCreateResult>;
    fn clone_repo(&self, url: &str, path: &Path) -> ModelResult<GitCloneResult>;
    /// Clone, forwarding libgit2 transfer progress to `progress`. The default
    /// ignores progress; backends that support it override this.
    fn clone_repo_with_progress(
        &self,
        url: &str,
        path: &Path,
        _progress: &dyn Fn(crate::GitTransferProgress),
    ) -> ModelResult<GitCloneResult> {
        self.clone_repo(url, path)
    }
    fn fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult>;
    /// List the refs a remote advertises WITHOUT fetching objects (porcelain
    /// `git ls-remote`): connect, read the advertised refs, disconnect. Non-mutating
    /// — used to plan a selection before any fetch (Q1).
    fn ls_remote(&self, path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>>;
    fn fast_forward(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult>;
    /// Integrate `upstream_ref` into `branch` by **merge** (porcelain `git merge`):
    /// fast-forward when the branch is strictly behind, else record a two-parent merge
    /// commit. On conflicts, leave the worktree mid-merge — `MERGE_HEAD` recorded so
    /// `git merge --continue` works — and return the conflicted paths instead of erroring;
    /// a conflict is an expected, developer-resolved outcome, not a failure. Self-verifies.
    fn merge_upstream(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult>;
    /// Integrate `upstream_ref` into `branch` by **rebase** (porcelain `git rebase`):
    /// replay the branch's commits onto the upstream tip. Fast-forwards when strictly
    /// behind. On conflict, leave `.git/rebase-merge/` in place (do NOT abort) so the
    /// developer can resolve and `git rebase --continue`, and return the conflicted
    /// paths instead of erroring. Self-verifies HEAD is reattached and based on upstream.
    fn rebase_onto(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult>;
    /// Snap `branch` to `upstream_ref` by **hard reset** (porcelain `git reset --hard`):
    /// discard local commits AND uncommitted changes, moving the branch onto upstream.
    /// Destructive and conflict-free; the caller gates it on `policy.destructive`.
    /// Self-verifies the branch (not a detached HEAD) is at the upstream commit, clean.
    fn reset_hard(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult>;
    fn checkout_commit(&self, path: &Path, commit: &str) -> ModelResult<GitUpdateResult>;
    /// Put HEAD on `branch` at `commit` — create the branch if missing, checkout if it
    /// is already there. Per AD3(c)'s orphan-safety rule, REFUSE (`DivergedMember`) if
    /// the branch exists at a different commit — never silently reset it. Self-verifies
    /// HEAD is on the branch at the commit with a clean worktree.
    fn checkout_branch(
        &self,
        path: &Path,
        branch: &str,
        commit: &str,
    ) -> ModelResult<GitUpdateResult>;
    fn status(&self, path: &Path) -> ModelResult<GitStatus>;
    fn head(&self, path: &Path) -> ModelResult<GitHeadState>;
    fn remotes(&self, path: &Path) -> ModelResult<Vec<GitRemote>>;
    fn add_remote(&self, path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult>;
    fn push(&self, path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult>;
    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>>;
    fn is_ancestor(&self, path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool>;
    /// Stage `pathspecs` into the index — `git add` semantics: add new/modified
    /// files, remove deleted ones, honor `.gitignore`. Self-verifies the index
    /// persisted with the requested files staged before returning success.
    /// Content parity with porcelain `git add` is proven by contract test.
    fn stage_paths(&self, path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialHelperPolicy {
    Disabled,
    AllowConfigured,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Git2Backend {
    pub(crate) credential_helpers: CredentialHelperPolicy,
}

impl Git2Backend {
    pub fn new() -> Self {
        Self {
            credential_helpers: CredentialHelperPolicy::AllowConfigured,
        }
    }

    pub fn without_credential_helpers() -> Self {
        Self {
            credential_helpers: CredentialHelperPolicy::Disabled,
        }
    }
}

impl Default for Git2Backend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCreateResult {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCloneResult {
    pub path: PathBuf,
    pub head: GitHeadState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFetchResult {
    pub remote: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemoteRef {
    /// Full ref name as advertised by the remote (e.g. `refs/heads/main`, `HEAD`).
    pub name: String,
    /// Object id the ref points at, as a hex string.
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitUpdateResult {
    pub updated: bool,
    pub commit: Option<String>,
}

/// Outcome of a merge/rebase integration. A conflict is reported, not errored:
/// `conflicts` names the paths and `commit` is `None`, with the worktree left
/// mid-integration for the developer to resolve — exactly as porcelain git leaves it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitIntegrateResult {
    /// New HEAD commit when the integration completed cleanly; `None` on conflict.
    pub commit: Option<String>,
    /// Conflicted paths; empty iff the integration completed cleanly.
    pub conflicts: Vec<String>,
}

impl GitIntegrateResult {
    pub(crate) fn clean(commit: String) -> Self {
        Self {
            commit: Some(commit),
            conflicts: Vec::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemoteResult {
    pub remote: GitRemote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPushResult {
    pub remote: String,
    pub refspec: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStageResult {
    /// Top-level *file* pathspecs confirmed present in the index by the self-verify
    /// pass. Directory pathspecs are staged but not counted here.
    pub staged: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitStatus {
    pub is_dirty: bool,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub files: Vec<GitFileStatus>,
}

impl GitStatus {
    pub fn clean() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
    pub original_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHeadState {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub is_detached: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemote {
    pub name: String,
    pub url: Option<String>,
    pub push_url: Option<String>,
}

impl GitBackend for Git2Backend {
    fn is_repository(&self, path: &Path) -> ModelResult<bool> {
        match git2::Repository::open(path) {
            Ok(_) => Ok(true),
            Err(err) if err.code() == git2::ErrorCode::NotFound => Ok(false),
            Err(err) => Err(git_error(err)),
        }
    }

    fn create_repo(&self, path: &Path) -> ModelResult<GitCreateResult> {
        let mut opts = git2::RepositoryInitOptions::new();
        opts.bare(false).no_reinit(true).initial_head("main");
        git2::Repository::init_opts(path, &opts).map_err(git_error)?;
        Ok(GitCreateResult {
            path: path.to_path_buf(),
        })
    }

    fn clone_repo(&self, url: &str, path: &Path) -> ModelResult<GitCloneResult> {
        self.clone_repo_with_progress(url, path, &|_progress| {})
    }

    fn clone_repo_with_progress(
        &self,
        url: &str,
        path: &Path,
        progress: &dyn Fn(crate::GitTransferProgress),
    ) -> ModelResult<GitCloneResult> {
        ensure_clone_target_is_empty(path)?;
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options_with_progress(
            self.credential_helpers,
            Some(progress),
        ));
        builder.clone(url, path).map_err(git_error)?;
        Ok(GitCloneResult {
            path: path.to_path_buf(),
            head: self.head(path)?,
        })
    }

    fn fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult> {
        let repo = open_repo(path)?;
        let mut remote_handle = find_remote(&repo, remote)?;
        let refspecs: [&str; 0] = [];
        remote_handle
            .fetch(
                &refspecs,
                Some(&mut remote_fetch_options(self.credential_helpers)),
                Some("gwz fetch"),
            )
            .map_err(git_error)?;
        Ok(GitFetchResult {
            remote: remote.to_owned(),
        })
    }

    fn ls_remote(&self, path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>> {
        let repo = open_repo(path)?;
        let mut remote_handle = find_remote(&repo, remote)?;
        let connection = remote_handle
            .connect_auth(
                git2::Direction::Fetch,
                Some(remote_callbacks(self.credential_helpers)),
                None,
            )
            .map_err(git_error)?;
        let refs = connection
            .list()
            .map_err(git_error)?
            .iter()
            .map(|head| GitRemoteRef {
                name: head.name().to_owned(),
                target: head.oid().to_string(),
            })
            .collect::<Vec<_>>();
        // `connection` disconnects on drop.
        Ok(refs)
    }

    fn fast_forward(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let target = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
        let annotated = repo.find_annotated_commit(target).map_err(git_error)?;
        let (analysis, _) = repo.merge_analysis(&[&annotated]).map_err(git_error)?;

        if analysis.is_up_to_date() {
            return Ok(GitUpdateResult {
                updated: false,
                commit: Some(target.to_string()),
            });
        }
        if !analysis.is_fast_forward() {
            return Err(ModelError::new(
                ErrorCode::DivergedMember,
                "branch cannot be fast-forwarded",
            ));
        }

        let local_ref_name = format!("refs/heads/{branch}");
        let mut local_ref = repo.find_reference(&local_ref_name).map_err(git_error)?;
        let target_object = repo.find_object(target, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&target_object, Some(&mut checkout))
            .map_err(git_error)?;
        local_ref
            .set_target(target, "gwz fast-forward")
            .map_err(git_error)?;
        repo.set_head(&local_ref_name).map_err(git_error)?;
        verify_checkout_state(path, target)?;
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(target.to_string()),
        })
    }

    fn merge_upstream(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult> {
        let repo = open_repo(path)?;
        let target = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
        let annotated = repo.find_annotated_commit(target).map_err(git_error)?;
        let (analysis, _) = repo.merge_analysis(&[&annotated]).map_err(git_error)?;
        if analysis.is_up_to_date() {
            return Ok(GitIntegrateResult::clean(target.to_string()));
        }
        if analysis.is_fast_forward() {
            // `git merge` fast-forwards by default when the branch is strictly behind.
            let ff = self.fast_forward(path, branch, upstream_ref)?;
            return Ok(GitIntegrateResult {
                commit: ff.commit,
                conflicts: Vec::new(),
            });
        }

        // True three-way merge: git2 stages the result into the index + worktree.
        repo.merge(&[&annotated], None, None).map_err(git_error)?;
        let mut index = repo.index().map_err(git_error)?;
        if index.has_conflicts() {
            // Faithful to porcelain: leave the conflict in the worktree and record
            // MERGE_HEAD so the developer can resolve and `git merge --continue`.
            std::fs::write(repo.path().join("MERGE_HEAD"), format!("{target}\n"))
                .map_err(|err| ModelError::new(ErrorCode::GitCommandFailed, err.to_string()))?;
            let conflicts = conflict_paths(&index)?;
            // AD1 self-verify: the conflict state actually persisted on disk.
            if conflicts.is_empty()
                || !open_repo(path)?.index().map_err(git_error)?.has_conflicts()
            {
                return Err(ModelError::new(
                    ErrorCode::GitCommandFailed,
                    "merge reported conflicts but none persisted in the index",
                ));
            }
            return Ok(GitIntegrateResult {
                commit: None,
                conflicts,
            });
        }

        // Clean merge: write the merged tree and record the two-parent merge commit.
        let tree_oid = index.write_tree().map_err(git_error)?;
        let tree = repo.find_tree(tree_oid).map_err(git_error)?;
        let signature = merge_signature(&repo)?;
        let head_commit = repo
            .head()
            .map_err(git_error)?
            .peel_to_commit()
            .map_err(git_error)?;
        let upstream_commit = repo.find_commit(target).map_err(git_error)?;
        let merge_oid = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                &format!("Merge {upstream_ref} into {branch}"),
                &tree,
                &[&head_commit, &upstream_commit],
            )
            .map_err(git_error)?;
        repo.cleanup_state().map_err(git_error)?;
        // AD1 self-verify: HEAD advanced to the merge commit with no residual conflicts.
        let observed = self.head(path)?;
        if observed.commit.as_deref() != Some(merge_oid.to_string().as_str()) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "post-merge HEAD is not the merge commit",
            ));
        }
        Ok(GitIntegrateResult::clean(merge_oid.to_string()))
    }

    fn rebase_onto(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult> {
        let repo = open_repo(path)?;
        let upstream_oid = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
        let upstream_annotated = repo.find_annotated_commit(upstream_oid).map_err(git_error)?;
        let (analysis, _) = repo.merge_analysis(&[&upstream_annotated]).map_err(git_error)?;
        if analysis.is_up_to_date() {
            return Ok(GitIntegrateResult::clean(upstream_oid.to_string()));
        }
        if analysis.is_fast_forward() {
            // Nothing to replay: `git rebase` of a strictly-behind branch fast-forwards.
            let ff = self.fast_forward(path, branch, upstream_ref)?;
            return Ok(GitIntegrateResult {
                commit: ff.commit,
                conflicts: Vec::new(),
            });
        }

        let signature = merge_signature(&repo)?;
        let mut rebase = repo
            .rebase(None, Some(&upstream_annotated), None, None)
            .map_err(git_error)?;
        // Replay each commit; git2 patches it into the index + worktree on `next()`. The
        // operation handle is dropped within the loop condition so the body can re-borrow.
        while rebase.next().transpose().map_err(git_error)?.is_some() {
            let index = repo.index().map_err(git_error)?;
            if index.has_conflicts() {
                // Faithful to porcelain: leave the rebase in progress for the developer
                // to resolve and `git rebase --continue`. Dropping `rebase` frees the
                // in-memory handle but leaves `.git/rebase-merge/` on disk.
                return Ok(GitIntegrateResult {
                    commit: None,
                    conflicts: conflict_paths(&index)?,
                });
            }
            rebase.commit(None, &signature, None).map_err(git_error)?;
        }
        rebase.finish(Some(&signature)).map_err(git_error)?;

        // AD1 self-verify: HEAD reattached to the branch and now descends from upstream.
        let observed = self.head(path)?;
        let Some(new_head) = observed.commit.clone() else {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "post-rebase HEAD is unborn",
            ));
        };
        if observed.is_detached || observed.branch.as_deref() != Some(branch) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("post-rebase HEAD is not on branch '{branch}'"),
            ));
        }
        if !self.is_ancestor(path, &upstream_oid.to_string(), &new_head)? {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "post-rebase HEAD is not based on upstream",
            ));
        }
        Ok(GitIntegrateResult::clean(new_head))
    }

    fn reset_hard(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let target = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
        let target_object = repo.find_object(target, None).map_err(git_error)?;
        repo.reset(&target_object, git2::ResetType::Hard, None)
            .map_err(git_error)?;
        verify_checkout_state(path, target)?;
        // AD1 self-verify: the branch (not a detached HEAD) now points at upstream.
        let observed = self.head(path)?;
        if observed.is_detached || observed.branch.as_deref() != Some(branch) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("post-reset HEAD is not on branch '{branch}'"),
            ));
        }
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(target.to_string()),
        })
    }

    fn checkout_commit(&self, path: &Path, commit: &str) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let oid = git2::Oid::from_str(commit).map_err(git_error)?;
        let object = repo.find_object(oid, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&object, Some(&mut checkout))
            .map_err(git_error)?;
        repo.set_head_detached(oid).map_err(git_error)?;
        verify_checkout_state(path, oid)?;
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(oid.to_string()),
        })
    }

    fn checkout_branch(
        &self,
        path: &Path,
        branch: &str,
        commit: &str,
    ) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let oid = git2::Oid::from_str(commit).map_err(git_error)?;
        let ref_name = format!("refs/heads/{branch}");
        // AD3(c) orphan-safety: never silently reset a branch. Create it if missing;
        // refuse if it already exists at a different commit (that would orphan work).
        match repo.find_reference(&ref_name) {
            Ok(existing) => {
                let existing_oid = existing.peel_to_commit().map_err(git_error)?.id();
                if existing_oid != oid {
                    return Err(ModelError::new(
                        ErrorCode::DivergedMember,
                        format!(
                            "branch '{branch}' is at {existing_oid}, not the target {oid}; refusing to move it"
                        ),
                    ));
                }
            }
            Err(err) if err.code() == git2::ErrorCode::NotFound => {
                let target = repo.find_commit(oid).map_err(git_error)?;
                repo.branch(branch, &target, false).map_err(git_error)?;
            }
            Err(err) => return Err(git_error(err)),
        }
        let object = repo.find_object(oid, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&object, Some(&mut checkout))
            .map_err(git_error)?;
        repo.set_head(&ref_name).map_err(git_error)?;
        verify_checkout_state(path, oid)?;
        // AD1 self-verify: HEAD is attached to the branch, not detached.
        let observed = self.head(path)?;
        if observed.is_detached || observed.branch.as_deref() != Some(branch) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("post-checkout HEAD is not on branch '{branch}'"),
            ));
        }
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(oid.to_string()),
        })
    }

    fn status(&self, path: &Path) -> ModelResult<GitStatus> {
        let repo = open_repo(path)?;
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            // F17: detect renames so a `git mv` reports `R` + original_path (the status
            // model already carries `original_path`) instead of an unrelated delete+add.
            .renames_head_to_index(true)
            .renames_index_to_workdir(true);
        let statuses = repo.statuses(Some(&mut opts)).map_err(git_error)?;
        let mut out = GitStatus::default();
        for entry in statuses.iter() {
            let status = entry.status();
            if status.intersects(staged_statuses()) {
                out.staged += 1;
            }
            if status.intersects(unstaged_statuses()) {
                out.unstaged += 1;
            }
            if status.contains(git2::Status::WT_NEW) {
                out.untracked += 1;
            }
            if let Some(file) = git_file_status(&entry) {
                out.files.push(file);
            }
        }
        out.is_dirty = out.staged > 0 || out.unstaged > 0 || out.untracked > 0;
        Ok(out)
    }

    fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
        let repo = open_repo(path)?;
        repo_head(&repo)
    }

    fn remotes(&self, path: &Path) -> ModelResult<Vec<GitRemote>> {
        let repo = open_repo(path)?;
        let names = repo.remotes().map_err(git_error)?;
        let mut remotes = Vec::new();
        for name in names.iter() {
            let Some(name) = name.map_err(git_error)? else {
                continue;
            };
            let remote = find_remote(&repo, name)?;
            remotes.push(GitRemote {
                name: name.to_owned(),
                url: Some(remote.url().map_err(git_error)?.to_owned()),
                push_url: remote.pushurl().map_err(git_error)?.map(ToOwned::to_owned),
            });
        }
        Ok(remotes)
    }

    fn add_remote(&self, path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult> {
        let repo = open_repo(path)?;
        let remote = repo.remote(name, url).map_err(git_error)?;
        Ok(GitRemoteResult {
            remote: GitRemote {
                name: name.to_owned(),
                url: Some(remote.url().map_err(git_error)?.to_owned()),
                push_url: remote.pushurl().map_err(git_error)?.map(ToOwned::to_owned),
            },
        })
    }

    fn push(&self, path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult> {
        let repo = open_repo(path)?;
        let mut remote_handle = find_remote(&repo, remote)?;
        remote_handle
            .push(
                &[refspec],
                Some(&mut remote_push_options(self.credential_helpers)),
            )
            .map_err(git_error)?;
        Ok(GitPushResult {
            remote: remote.to_owned(),
            refspec: refspec.to_owned(),
        })
    }

    fn stage_paths(&self, path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult> {
        let repo = open_repo(path)?;
        let mut index = repo.index().map_err(git_error)?;
        index
            .add_all(
                pathspecs.iter().copied(),
                git2::IndexAddOption::DEFAULT,
                None,
            )
            .map_err(git_error)?;
        index.write().map_err(git_error)?;

        // AD1 self-verify: re-open the repo so the index is read fresh from disk,
        // and confirm every requested *file* persisted into the index. Directory
        // pathspecs are covered by the fresh read; full content parity with
        // porcelain `git add` is proven by the contract test, not asserted here.
        let verify = open_repo(path)?.index().map_err(git_error)?;
        if verify.has_conflicts() {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "index has conflicts after staging",
            ));
        }
        let mut staged = 0usize;
        for spec in pathspecs {
            if path.join(spec).is_file() {
                if verify.get_path(Path::new(spec), 0).is_none() {
                    return Err(ModelError::new(
                        ErrorCode::GitCommandFailed,
                        format!("staged path missing from index after write: {spec}"),
                    ));
                }
                staged += 1;
            }
        }
        Ok(GitStageResult { staged })
    }

    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>> {
        let repo = open_repo(path)?;
        match repo.revparse_single(ref_spec) {
            Ok(object) => Ok(Some(object.id().to_string())),
            Err(err)
                if matches!(
                    err.code(),
                    git2::ErrorCode::NotFound | git2::ErrorCode::UnbornBranch
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(git_error(err)),
        }
    }

    fn is_ancestor(&self, path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool> {
        let repo = open_repo(path)?;
        let ancestor = git2::Oid::from_str(ancestor).map_err(git_error)?;
        let descendant = git2::Oid::from_str(descendant).map_err(git_error)?;
        repo.graph_descendant_of(descendant, ancestor)
            .map_err(git_error)
    }
}

pub(crate) fn open_repo(path: &Path) -> ModelResult<git2::Repository> {
    git2::Repository::open(path).map_err(git_error)
}

/// Conflicted paths in `index`, sorted and de-duplicated. A conflict carries up to
/// three stages (ancestor/our/their); any one supplies the path.
pub(crate) fn conflict_paths(index: &git2::Index) -> ModelResult<Vec<String>> {
    let mut paths = Vec::new();
    for conflict in index.conflicts().map_err(git_error)? {
        let conflict = conflict.map_err(git_error)?;
        if let Some(entry) = conflict.our.or(conflict.their).or(conflict.ancestor)
            && let Ok(path) = std::str::from_utf8(&entry.path)
        {
            paths.push(path.to_owned());
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Author/committer for gwz-created merge commits: the repo's configured identity
/// when present, else a stable gwz fallback so an unconfigured repo can still merge.
pub(crate) fn merge_signature(repo: &git2::Repository) -> ModelResult<git2::Signature<'static>> {
    if let Ok(signature) = repo.signature() {
        return Ok(signature);
    }
    git2::Signature::now("gwz", "gwz@localhost").map_err(git_error)
}

pub(crate) fn remote_fetch_options(credential_helpers: CredentialHelperPolicy) -> git2::FetchOptions<'static> {
    fetch_options_with_progress(credential_helpers, None)
}

pub(crate) fn fetch_options_with_progress<'a>(
    credential_helpers: CredentialHelperPolicy,
    progress: Option<&'a dyn Fn(crate::GitTransferProgress)>,
) -> git2::FetchOptions<'a> {
    let mut callbacks = remote_callbacks(credential_helpers);
    if let Some(progress) = progress {
        callbacks.transfer_progress(move |stats| {
            progress(git_transfer_progress(&stats));
            true
        });
    }
    let mut options = git2::FetchOptions::new();
    options.remote_callbacks(callbacks);
    options
}

pub(crate) fn remote_push_options(credential_helpers: CredentialHelperPolicy) -> git2::PushOptions<'static> {
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(remote_callbacks(credential_helpers));
    options
}

pub(crate) fn remote_callbacks<'a>(credential_helpers: CredentialHelperPolicy) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |url, username_from_url, allowed_types| {
        remote_credential(url, username_from_url, allowed_types, credential_helpers)
    });
    callbacks
}

pub(crate) fn remote_credential(
    url: &str,
    username_from_url: Option<&str>,
    allowed_types: git2::CredentialType,
    credential_helpers: CredentialHelperPolicy,
) -> Result<git2::Cred, git2::Error> {
    let username = username_from_url.unwrap_or("git");
    if allowed_types.is_ssh_key() {
        return git2::Cred::ssh_key_from_agent(username);
    }
    if allowed_types.is_username() {
        return git2::Cred::username(username);
    }
    if allowed_types.is_user_pass_plaintext()
        && credential_helpers == CredentialHelperPolicy::AllowConfigured
        && let Ok(config) = git2::Config::open_default()
        && let Ok(credential) = git2::Cred::credential_helper(&config, url, username_from_url)
    {
        return Ok(credential);
    }
    if allowed_types.is_default() {
        return git2::Cred::default();
    }
    Err(git2::Error::from_str(
        "GWZ could not acquire credentials for the requested remote",
    ))
}

pub(crate) fn git_file_status(entry: &git2::StatusEntry<'_>) -> Option<GitFileStatus> {
    let status = entry.status();
    // git2 reports a rename entry under the OLD path; model it the way `git status` does —
    // current path = the new path, `original_path` = where it came from.
    let (path, original_path) = match rename_delta(entry, status) {
        Some((old, new)) => (new, Some(old)),
        None => (entry.path().ok()?.to_owned(), None),
    };
    Some(GitFileStatus {
        path,
        index_status: index_status_char(status).to_owned(),
        worktree_status: worktree_status_char(status).to_owned(),
        original_path,
    })
}

/// `(old_path, new_path)` when `entry` is a rename (staged or unstaged), else `None`.
pub(crate) fn rename_delta(
    entry: &git2::StatusEntry<'_>,
    status: git2::Status,
) -> Option<(String, String)> {
    if !status.intersects(git2::Status::INDEX_RENAMED | git2::Status::WT_RENAMED) {
        return None;
    }
    let delta = entry.head_to_index().or_else(|| entry.index_to_workdir())?;
    let old = delta.old_file().path()?.to_str()?.to_owned();
    let new = delta.new_file().path()?.to_str()?.to_owned();
    Some((old, new))
}

