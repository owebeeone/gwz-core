use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::cell::{Cell, RefCell};

use sha2::{Digest, Sha256};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::*;

mod authority_backend;
mod backend;
mod comparison;
mod contract;
mod factory;
mod merge_prepared;
mod merge_recovery;
mod merge_support;
mod preservation;
mod preservation_image;
mod preservation_root;
mod push_plan;
mod recovery_support;
mod refs;
mod repository;
mod repository_support;
mod scoped_evidence;
mod scoped_support;
mod stash;
mod stash_support;
mod transport;
mod transport_observations;
mod transport_support;
mod types;

pub use authority_backend::MergeAuthorityBackend;
pub use backend::*;
pub use contract::*;
#[cfg(not(test))]
pub use factory::make_repository;
#[cfg(test)]
pub(crate) use factory::make_repository;
pub use transport_observations::TransportObservations;
pub(crate) use transport_support::identity::has_options as has_transport_options;
pub(crate) use transport_support::identity::{
    resolve_path as resolve_ssh_identity_path, validate_file as validate_ssh_identity_file,
};
pub use transport_support::{configure_server_timeout_ms, set_server_timeout_ms};
pub use types::*;

pub(crate) use repository_support::open_repo;

#[cfg(test)]
pub(crate) use crate::checked_artifact::{CheckedArtifactFault, fail_next_checked_artifact_at};
#[cfg(test)]
pub(crate) use merge_support::{conflict_paths, render_git_path};
#[cfg(test)]
pub(crate) use preservation_image::raw_path_preimage_for_test;
#[cfg(test)]
pub(crate) use preservation_root::{FaultBoundary, fail_next_at, run_next_at};
#[cfg(test)]
pub(crate) use stash_support::stash_message_matches_gwz_prefix;
#[cfg(test)]
pub(crate) use transport_support::remote_credential;

use backend::{
    record_preparation_call, run_before_prepared_execution, run_before_preservation_stash,
    run_before_scoped_commit_ref_lock,
};
macro_rules! delegate {
    ($name:ident($($arg:ident: $arg_type:ty),* $(,)?) -> $result:ty => $module:ident::$function:ident) => {
        fn $name(&self $(, $arg: $arg_type)*) -> $result {
            $module::$function(self $(, $arg)*)
        }
    };
}

impl GitBackend for Git2Backend {
    fn repository_index(&self, path: &Path) -> ModelResult<GitIndexSnapshot> {
        let repo = open_repo(path)?;
        let index = repo.index().map_err(git_error)?;
        Ok(GitIndexSnapshot {
            path: index.path().map(Path::to_path_buf),
            entries: index
                .iter()
                .map(|e| GitIndexEntry {
                    path: e.path,
                    object_id: e.id.as_bytes().to_vec(),
                    mode: e.mode,
                    flags: e.flags,
                    flags_extended: e.flags_extended,
                    ctime: (e.ctime.seconds(), e.ctime.nanoseconds()),
                    mtime: (e.mtime.seconds(), e.mtime.nanoseconds()),
                    stat: [e.dev, e.ino, e.uid, e.gid, e.file_size],
                })
                .collect(),
        })
    }
    fn repository_paths(&self, path: &Path) -> ModelResult<GitRepositoryPaths> {
        let repo = open_repo(path)?;
        Ok(GitRepositoryPaths {
            worktree: repo.workdir().map(Path::to_path_buf),
            git_dir: repo.path().to_path_buf(),
            common_dir: repo.commondir().to_path_buf(),
        })
    }

    fn root_preservation_image(
        &self,
        root: &Path,
        clean: &GitRootManagedForm,
        excluded: &[String],
    ) -> ModelResult<GitPreservationImage> {
        preservation_image::capture_normalized(self.filesystem.as_ref(), root, clean, excluded)
    }

    fn validate_root_preservation_spec(
        &self,
        root: &Path,
        spec: &GitRootPreservationSpec,
    ) -> ModelResult<()> {
        preservation_root::index::validate_spec(root, spec)
    }

    fn root_managed_index_matches(
        &self,
        root: &Path,
        form: &GitRootManagedIndexForm,
    ) -> ModelResult<bool> {
        preservation_root::index::observe(self.filesystem.as_ref(), root, form)
    }

    fn rewrite_root_managed_index_checked(
        &self,
        root: &Path,
        form: &GitRootManagedIndexForm,
    ) -> ModelResult<()> {
        preservation_root::index::rewrite(self.filesystem.as_ref(), root, form)
    }

    #[cfg(test)]
    delegate!(test_init_repo(repo: &Path, spec: &TestRepoSpec) -> ModelResult<()> => fixture_native::test_init_repo);
    #[cfg(test)]
    delegate!(test_create_commit(repo: &Path, spec: &TestCommitSpec) -> ModelResult<String> => fixture_native::test_create_commit);
    #[cfg(test)]
    delegate!(test_read_commit(repo: &Path, oid: &str) -> ModelResult<TestCommit> => fixture_native::test_read_commit);
    #[cfg(test)]
    delegate!(test_set_ref(repo: &Path, name: &str, target: Option<&TestRefTarget>) -> ModelResult<()> => fixture_native::test_set_ref);
    #[cfg(test)]
    delegate!(test_set_head(repo: &Path, state: &TestHead) -> ModelResult<()> => fixture_native::test_set_head);
    #[cfg(test)]
    delegate!(test_replace_index(repo: &Path, entries: &[TestIndexEntry]) -> ModelResult<()> => fixture_native::test_replace_index);
    #[cfg(test)]
    delegate!(test_read_index(repo: &Path) -> ModelResult<Vec<TestIndexEntry>> => fixture_native::test_read_index);
    #[cfg(test)]
    delegate!(test_set_config(repo: &Path, key: &str, values: &[String]) -> ModelResult<()> => fixture_native::test_set_config);
    #[cfg(test)]
    delegate!(test_read_config(repo: &Path, key: &str) -> ModelResult<Vec<String>> => fixture_native::test_read_config);
    #[cfg(test)]
    delegate!(test_force_checkout(repo: &Path, commit: &str) -> ModelResult<()> => fixture_native::test_force_checkout);
    #[cfg(test)]
    delegate!(test_reset_mixed(repo: &Path, commit: &str) -> ModelResult<()> => fixture_native::test_reset_mixed);
    #[cfg(test)]
    delegate!(test_set_repository_state(repo: &Path, state: GitRepositoryState, merge_head: Option<&str>) -> ModelResult<()> => fixture_native::test_set_repository_state);
    #[cfg(test)]
    delegate!(test_seed_merge_conflict(repo: &Path, before: &str, source: &str) -> ModelResult<GitMergeConflictSnapshot> => fixture_native::test_seed_merge_conflict);
    #[cfg(test)]
    delegate!(test_create_commit_from_parent(repo: &Path, parent: &str, message: &str, edits: &[TestCommitFileEdit]) -> ModelResult<String> => fixture_native::test_create_commit_from_parent);

    fn preservation_stashes(
        &self,
        path: &Path,
        merge_id: &str,
    ) -> ModelResult<Vec<GitPreservationStashEvidence>> {
        observe_preservation_stashes_read_only(path, merge_id)
    }

    fn transport_observations(&self) -> Option<TransportObservations> {
        Some(self.observations.clone())
    }
    fn remote_identity(&self, path: &Path, remote: &str) -> ModelResult<Option<String>> {
        transport_support::identity::configured_identity(path, remote)
    }
    fn set_remote_identity(
        &self,
        path: &Path,
        remote: &str,
        value: Option<&str>,
    ) -> ModelResult<()> {
        transport_support::identity::set_configured_identity(path, remote, value)
    }
    fn with_transport(
        &self,
        start: &Path,
        options: Option<&crate::TransportOptions>,
    ) -> ModelResult<Option<Self>> {
        let empty = crate::TransportOptions::default();
        let identities =
            transport_support::identity::Selection::from_options(start, options.unwrap_or(&empty))?;
        identities.validate_files()?;
        Ok(Some(Self {
            filesystem: self.filesystem.clone(),
            credential_helpers: self.credential_helpers,
            identities,
            observations: Default::default(),
        }))
    }
    fn validate_transport_remotes(&self, names: &[String]) -> ModelResult<()> {
        self.identities.validate_remote_names(names)
    }
    fn validate_remote_identity(&self, path: &Path, remote: &str, push: bool) -> ModelResult<()> {
        let repo = open_repo(path)?;
        let handle = repo.find_remote(remote).map_err(git_error)?;
        let url = if push {
            handle
                .pushurl()
                .map_err(git_error)?
                .unwrap_or(handle.url().map_err(git_error)?)
        } else {
            handle.url().map_err(git_error)?
        };
        transport_support::identity::for_remote(self, Some(&repo), Some(remote), url).map(|_| ())
    }
    fn validate_url_identity(
        &self,
        identity_repo: Option<&Path>,
        remote: &str,
        url: &str,
    ) -> ModelResult<()> {
        let repo = identity_repo.map(open_repo).transpose()?;
        transport_support::identity::for_remote(self, repo.as_ref(), Some(remote), url).map(|_| ())
    }
    delegate!(is_repository(path: &Path) -> ModelResult<bool> => repository::is_repository);
    delegate!(commit_exists(path: &Path, oid: &str) -> ModelResult<bool> => repository::commit_exists);
    delegate!(read_file_at_commit(path: &Path, commit: &str, relative_path: &str,) -> ModelResult<Option<Vec<u8>>> => repository::read_file_at_commit);
    delegate!(commit_matches_merge(path: &Path, commit: &str, first_parent: &str, second_parent: &str, message: &str,) -> ModelResult<bool> => merge_prepared::commit_matches_merge);
    delegate!(commit_matches_prepared_merge(path: &Path, commit: &str, first_parent: &str, second_parent: &str, message: &str, prepared: &GitPreparedCommit,) -> ModelResult<bool> => merge_prepared::commit_matches_prepared_merge);
    delegate!(create_repo(path: &Path) -> ModelResult<GitCreateResult> => repository::create_repo);
    delegate!(clone_repo(url: &str, path: &Path) -> ModelResult<GitCloneResult> => transport::clone_repo);
    delegate!(clone_repo_with_progress(url: &str, path: &Path, progress: &dyn Fn(crate::GitTransferProgress),) -> ModelResult<GitCloneResult> => transport::clone_repo_with_progress);
    delegate!(clone_repo_named(url: &str, path: &Path, remote: &str, progress: &dyn Fn(crate::GitTransferProgress)) -> ModelResult<GitCloneResult> => transport::clone_repo_named);
    delegate!(read_remote_file(url: &str, remote: &str, relative_path: &str) -> ModelResult<Option<Vec<u8>>> => transport::read_remote_file);
    delegate!(fetch(path: &Path, remote: &str) -> ModelResult<GitFetchResult> => transport::fetch);
    delegate!(tag_fetch(path: &Path, remote: &str) -> ModelResult<GitFetchResult> => transport::tag_fetch);
    delegate!(ls_remote(path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>> => transport::ls_remote);
    delegate!(ls_remote_url(path: &Path, url: &str, remote_name: &str, identity_repo: Option<&Path>) -> ModelResult<Vec<GitRemoteRef>> => transport::ls_remote_url);
    delegate!(fast_forward(path: &Path, branch: &str, upstream_ref: &str,) -> ModelResult<GitUpdateResult> => refs::fast_forward);
    delegate!(merge_upstream(path: &Path, branch: &str, upstream_ref: &str,) -> ModelResult<GitIntegrateResult> => merge_prepared::merge_upstream);
    delegate!(merge_upstream_checked(path: &Path, branch: &str, expected_before: &str, source_commit: &str, message: &str, attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitIntegrateResult> => merge_prepared::merge_upstream_checked);
    delegate!(prepare_merge_upstream_checked(path: &Path, branch: &str, expected_before: &str, source_commit: &str, attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitPreparedMerge> => merge_prepared::prepare_merge_upstream_checked);
    delegate!(prepare_merge_upstream_mode_checked(path: &Path, branch: &str, expected_before: &str, source_commit: &str, mode: GitPreparedMergeMode, attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitPreparedMerge> => merge_prepared::prepare_merge_upstream_mode_checked);
    delegate!(validate_prepared_merge_upstream_state(path: &Path, branch: &str, expected_before: &str, source_commit: &str, prepared: &GitPreparedMerge,) -> ModelResult<()> => merge_prepared::validate_prepared_merge_upstream_state);
    delegate!(execute_prepared_merge_upstream_checked(path: &Path, branch: &str, expected_before: &str, source_commit: &str, message: &str, prepared: &GitPreparedMerge,) -> ModelResult<GitIntegrateResult> => merge_prepared::execute_prepared_merge_upstream_checked);
    delegate!(merge_analysis(path: &Path, target_branch: &str, source: &str,) -> ModelResult<GitMergeAnalysis> => merge_prepared::merge_analysis);
    delegate!(merge_simulate(path: &Path, target_commit: &str, source_commit: &str,) -> ModelResult<GitMergeSimulation> => merge_prepared::merge_simulate);
    delegate!(merge_state(path: &Path) -> ModelResult<Option<GitNativeMergeState>> => merge_recovery::merge_state);
    delegate!(repository_state(path: &Path) -> ModelResult<GitRepositoryState> => merge_recovery::repository_state);
    delegate!(validate_merge_recovery_state(path: &Path, expected_before: &str, expected_merge_head: &str, require_resolved: bool,) -> ModelResult<()> => merge_recovery::validate_merge_recovery_state);
    delegate!(merge_conflict_snapshot(path: &Path, expected_before: &str, expected_merge_head: &str,) -> ModelResult<GitMergeConflictSnapshot> => merge_recovery::merge_conflict_snapshot);
    delegate!(abort_merge(path: &Path, expected_before: &str, expected_merge_head: &str,) -> ModelResult<()> => merge_recovery::abort_merge);
    delegate!(set_branch_target_checked(path: &Path, branch: &str, expected_current: &str, target: &str,) -> ModelResult<GitUpdateResult> => merge_recovery::set_branch_target_checked);
    delegate!(delete_branch_target_checked(path: &Path, branch: &str, expected_current: &str,) -> ModelResult<()> => merge_recovery::delete_branch_target_checked);
    delegate!(create_backup_ref(path: &Path, name: &str, target: &str,) -> ModelResult<GitBackupRefResult> => preservation::create_backup_ref);
    delegate!(create_backup_ref_checked(path: &Path, branch: &str, expected_head: &str, name: &str, target: &str,) -> ModelResult<GitBackupRefResult> => preservation::create_backup_ref_checked);
    delegate!(delete_backup_ref_checked(path: &Path, name: &str, expected_target: &str,) -> ModelResult<()> => preservation::delete_backup_ref_checked);
    delegate!(stash_for_merge_preservation(path: &Path, merge_id: &str, include_untracked: bool,) -> ModelResult<GitStashPushResult> => preservation::stash_for_merge_preservation);
    delegate!(stash_for_merge_preservation_checked(path: &Path, branch: &str, expected_head: &str, expected_preimage_sha256: &str, merge_id: &str, include_untracked: bool,) -> ModelResult<GitStashPushResult> => preservation::stash_for_merge_preservation_checked);
    delegate!(preservation_image(path: &Path, include_untracked: bool,) -> ModelResult<GitPreservationImage> => preservation::preservation_image);
    delegate!(observe_direct_ref(path: &Path, name: &str,) -> ModelResult<GitDirectRefObservation> => preservation::observe_direct_ref);
    delegate!(checkout_matches_commit(path: &Path, branch: &str, commit: &str,) -> ModelResult<bool> => preservation::checkout_matches_commit);
    delegate!(checkout_matches_commit_except(path: &Path, commit: &str, allowed_paths: &[String],) -> ModelResult<bool> => preservation::checkout_matches_commit_except);
    delegate!(checkout_matches_commit_with_overlay(path: &Path, commit: &str, overlay: &GitCheckoutOverlay,) -> ModelResult<bool> => preservation::checkout_matches_commit_with_overlay);
    fn prepare_root_preservation_stash(
        &self,
        root: &Path,
        spec: &GitRootPreservationSpec,
    ) -> ModelResult<GitPreparedRootStash> {
        preservation_root::prepare_root_preservation_stash(
            self.filesystem.as_ref(),
            self,
            root,
            spec,
        )
    }
    fn observe_root_preservation_step(
        &self,
        root: &Path,
        spec: &GitRootPreservationSpec,
        step: &GitRootPreservationPhysicalStep,
        guard: &GitRootPreservationGuard,
    ) -> ModelResult<GitRootPreservationStepObservation> {
        preservation_root::observe_root_preservation_step(
            self.filesystem.as_ref(),
            self,
            root,
            spec,
            step,
            guard,
        )
    }
    fn execute_root_preservation_step_checked(
        &self,
        root: &Path,
        spec: &GitRootPreservationSpec,
        step: &GitRootPreservationPhysicalStep,
        guard: &GitRootPreservationGuard,
    ) -> ModelResult<GitCheckedPreservationMutation> {
        preservation_root::execute_root_preservation_step_checked(
            self.filesystem.as_ref(),
            self,
            root,
            spec,
            step,
            guard,
        )
    }
    delegate!(index_matches_candidate_files(path: &Path, expected_files: &[GitCandidateFile], absent_paths: &[String],) -> ModelResult<bool> => preservation::index_matches_candidate_files);
    delegate!(index_entries_match_candidate_files(path: &Path, expected_files: &[GitCandidateFile], absent_paths: &[String],) -> ModelResult<bool> => preservation::index_entries_match_candidate_files);
    delegate!(commit_bootstrap_paths_checked(root: &Path, expected_head: Option<&str>, paths: &[&str], message: &str,) -> ModelResult<Option<GitScopedCommitResult>> => scoped_evidence::commit_bootstrap_paths_checked);
    delegate!(commit_gwz_paths_checked(root: &Path, expected_head: Option<&str>, candidate_files: &[GitCandidateFile], message: &str,) -> ModelResult<GitScopedCommitResult> => scoped_evidence::commit_gwz_paths_checked);
    delegate!(verify_gwz_paths_commit(root: &Path, commit: &str, expected_parent: Option<&str>, candidate_files: &[GitCandidateFile], message: &str,) -> ModelResult<GitScopedCommitResult> => scoped_evidence::verify_gwz_paths_commit);
    delegate!(rollback_gwz_paths_commit_checked(root: &Path, branch: &str, commit: &str, expected_parent: Option<&str>, candidate_files: &[GitCandidateFile], message: &str,) -> ModelResult<()> => scoped_evidence::rollback_gwz_paths_commit_checked);
    delegate!(rebase_onto(path: &Path, branch: &str, upstream_ref: &str,) -> ModelResult<GitIntegrateResult> => merge_prepared::rebase_onto);
    delegate!(reset_hard(path: &Path, branch: &str, upstream_ref: &str,) -> ModelResult<GitUpdateResult> => repository::reset_hard);
    delegate!(checkout_commit(path: &Path, commit: &str) -> ModelResult<GitUpdateResult> => repository::checkout_commit);
    delegate!(checkout_branch(path: &Path, branch: &str, commit: &str,) -> ModelResult<GitUpdateResult> => repository::checkout_branch);
    delegate!(branch_list(path: &Path) -> ModelResult<Vec<GitBranch>> => refs::branch_list);
    delegate!(branch_create(path: &Path, branch: &str, start_ref: &str,) -> ModelResult<GitBranchCreateResult> => refs::branch_create);
    delegate!(branch_delete(path: &Path, branch: &str) -> ModelResult<()> => refs::branch_delete);
    delegate!(switch_branch(path: &Path, branch: &str) -> ModelResult<GitUpdateResult> => refs::switch_branch);
    delegate!(stash_push(path: &Path, message: &str, options: GitStashPushOptions,) -> ModelResult<GitStashPushResult> => stash::stash_push);
    delegate!(stash_list(path: &Path) -> ModelResult<Vec<GitStashEntry>> => stash::stash_list);
    delegate!(stash_apply(path: &Path, target: &GitStashTarget, options: GitStashRestoreOptions,) -> ModelResult<()> => stash::stash_apply);
    delegate!(stash_pop(path: &Path, target: &GitStashTarget, options: GitStashRestoreOptions,) -> ModelResult<()> => stash::stash_pop);
    delegate!(stash_drop(path: &Path, target: &GitStashTarget) -> ModelResult<()> => stash::stash_drop);
    delegate!(status(path: &Path) -> ModelResult<GitStatus> => repository::status);
    delegate!(status_with_options(path: &Path, options: GitStatusOptions,) -> ModelResult<GitStatus> => repository::status_with_options);
    delegate!(head(path: &Path) -> ModelResult<GitHeadState> => repository::head);
    delegate!(remotes(path: &Path) -> ModelResult<Vec<GitRemote>> => transport::remotes);
    delegate!(add_remote(path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult> => transport::add_remote);
    delegate!(push(path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult> => transport::push);
    delegate!(prepare_push(path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPreparedPush> => push_plan::prepare);
    delegate!(push_prepared(path: &Path, plan: &GitPreparedPush) -> ModelResult<GitPushResult> => transport::push_prepared);
    delegate!(fetch_anonymous(path: &Path, url: &str, refspecs: &[&str]) -> ModelResult<GitFetchResult> => transport::fetch_anonymous);
    delegate!(push_anonymous(path: &Path, url: &str, refspec: &str) -> ModelResult<GitPushResult> => transport::push_anonymous);
    delegate!(stage_paths(path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult> => repository::stage_paths);
    delegate!(stage_paths_allowing_other_conflicts(path: &Path, pathspecs: &[&str],) -> ModelResult<GitStageResult> => repository::stage_paths_allowing_other_conflicts);
    delegate!(commit(path: &Path, message: &str, all: bool) -> ModelResult<GitCommitResult> => repository::commit);
    delegate!(commit_merge_resolution(path: &Path, message: &str) -> ModelResult<GitCommitResult> => merge_recovery::commit_merge_resolution);
    delegate!(commit_merge_resolution_checked(path: &Path, target_branch: &str, expected_before: &str, expected_merge_head: &str, message: &str, attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitCommitResult> => merge_recovery::commit_merge_resolution_checked);
    delegate!(prepare_merge_resolution_checked(path: &Path, target_branch: &str, expected_before: &str, expected_merge_head: &str, attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitPreparedCommit> => merge_recovery::prepare_merge_resolution_checked);
    delegate!(validate_prepared_merge_resolution_state(path: &Path, target_branch: &str, expected_before: &str, expected_merge_head: &str, prepared: &GitPreparedCommit,) -> ModelResult<()> => merge_recovery::validate_prepared_merge_resolution_state);
    delegate!(commit_prepared_merge_resolution_checked(path: &Path, target_branch: &str, expected_before: &str, expected_merge_head: &str, message: &str, prepared: &GitPreparedCommit,) -> ModelResult<GitCommitResult> => merge_recovery::commit_prepared_merge_resolution_checked);
    delegate!(tag_create(path: &Path, name: &str, message: Option<&str>, signed: bool,) -> ModelResult<GitTagResult> => refs::tag_create);
    delegate!(tag_list(path: &Path) -> ModelResult<Vec<String>> => refs::tag_list);
    delegate!(tag_delete(path: &Path, name: &str) -> ModelResult<()> => refs::tag_delete);
    delegate!(read_ref(path: &Path, ref_spec: &str) -> ModelResult<Option<String>> => refs::read_ref);
    delegate!(is_ancestor(path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool> => refs::is_ancestor);
    delegate!(merge_base(path: &Path, left: &str, right: &str) -> ModelResult<Option<String>> => comparison::merge_base);
    delegate!(changed_paths_between(path: &Path, old_commit: &str, new_commit: &str,) -> ModelResult<Vec<String>> => comparison::changed_paths_between);
    delegate!(diff_manifest(path: &Path, comparison: &crate::diff::RepoDiffComparison, options: &crate::diff::RepoDiffOptions,) -> ModelResult<crate::diff::RepoDiffManifest> => comparison::diff_manifest);
    delegate!(resolve_comparison(path: &Path, spec: &crate::diff::ComparisonSpec,) -> ModelResult<crate::diff::RepoDiffComparison> => comparison::resolve_comparison);
}

impl Git2Backend {
    /// Decode every native stash carrying this merge's stable preservation id.
    pub fn preservation_stashes(
        &self,
        path: &Path,
        merge_id: &str,
    ) -> ModelResult<Vec<GitPreservationStashEvidence>> {
        preservation_image::preservation_stashes(path, merge_id)
    }
}

/// Read native preservation evidence through the fixed production observer.
///
/// The physical repository implementation delegates here. Merge callers use
/// GitRepository through the existing sealed production authority boundary.
#[allow(
    dead_code,
    reason = "compiled ahead of A1 while all v1 consumers remain test-gated"
)]
pub(crate) fn observe_preservation_stashes_read_only(
    path: &Path,
    merge_id: &str,
) -> ModelResult<Vec<GitPreservationStashEvidence>> {
    preservation_image::preservation_stashes(path, merge_id)
}

#[cfg(test)]
mod repository_contract_tests;

#[cfg(test)]
mod fake_repository;
#[cfg(test)]
pub(crate) use fake_repository::FakeGitRepository;

#[cfg(test)]
mod test_types;
#[cfg(test)]
pub use test_types::*;
#[cfg(test)]
mod fixture_native;

#[cfg(test)]
pub(crate) use factory::GitTestRepository;
