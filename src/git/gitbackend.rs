use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::cell::{Cell, RefCell};

use sha2::{Digest, Sha256};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::*;

mod backend;
mod comparison;
mod contract;
mod merge_prepared;
mod merge_recovery;
mod merge_support;
mod preservation;
mod preservation_image;
mod preservation_root;
mod recovery_support;
mod refs;
mod repository;
mod repository_support;
mod scoped_evidence;
mod scoped_support;
mod stash;
mod stash_support;
mod transport;
mod transport_support;
mod types;

pub use backend::*;
pub use contract::*;
pub use transport_support::set_server_timeout_ms;
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
    record_preparation_call, run_before_prepared_execution, run_before_scoped_commit_ref_lock,
};
macro_rules! delegate {
    ($name:ident($($arg:ident: $arg_type:ty),* $(,)?) -> $result:ty => $module:ident::$function:ident) => {
        fn $name(&self $(, $arg: $arg_type)*) -> $result {
            $module::$function(self $(, $arg)*)
        }
    };
}

impl GitBackend for Git2Backend {
    delegate!(is_repository(path: &Path) -> ModelResult<bool> => repository::is_repository);
    delegate!(commit_exists(path: &Path, oid: &str) -> ModelResult<bool> => repository::commit_exists);
    delegate!(read_file_at_commit(path: &Path, commit: &str, relative_path: &str,) -> ModelResult<Option<Vec<u8>>> => repository::read_file_at_commit);
    delegate!(commit_matches_merge(path: &Path, commit: &str, first_parent: &str, second_parent: &str, message: &str,) -> ModelResult<bool> => merge_prepared::commit_matches_merge);
    delegate!(commit_matches_prepared_merge(path: &Path, commit: &str, first_parent: &str, second_parent: &str, message: &str, prepared: &GitPreparedCommit,) -> ModelResult<bool> => merge_prepared::commit_matches_prepared_merge);
    delegate!(create_repo(path: &Path) -> ModelResult<GitCreateResult> => repository::create_repo);
    delegate!(clone_repo(url: &str, path: &Path) -> ModelResult<GitCloneResult> => transport::clone_repo);
    delegate!(clone_repo_with_progress(url: &str, path: &Path, progress: &dyn Fn(crate::GitTransferProgress),) -> ModelResult<GitCloneResult> => transport::clone_repo_with_progress);
    delegate!(fetch(path: &Path, remote: &str) -> ModelResult<GitFetchResult> => transport::fetch);
    delegate!(tag_fetch(path: &Path, remote: &str) -> ModelResult<GitFetchResult> => transport::tag_fetch);
    delegate!(ls_remote(path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>> => transport::ls_remote);
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
    delegate!(preservation_stashes(path: &Path, merge_id: &str,) -> ModelResult<Vec<GitPreservationStashEvidence>> => preservation::preservation_stashes);
    delegate!(observe_direct_ref(path: &Path, name: &str,) -> ModelResult<GitDirectRefObservation> => preservation::observe_direct_ref);
    delegate!(checkout_matches_commit(path: &Path, branch: &str, commit: &str,) -> ModelResult<bool> => preservation::checkout_matches_commit);
    delegate!(prepare_root_preservation_stash(root: &Path, spec: &GitRootPreservationSpec,) -> ModelResult<GitPreparedRootStash> => preservation_root::prepare_root_preservation_stash);
    delegate!(observe_root_preservation_step(root: &Path, spec: &GitRootPreservationSpec, step: &GitRootPreservationPhysicalStep, guard: &GitRootPreservationGuard,) -> ModelResult<GitRootPreservationStepObservation> => preservation_root::observe_root_preservation_step);
    delegate!(execute_root_preservation_step_checked(root: &Path, spec: &GitRootPreservationSpec, step: &GitRootPreservationPhysicalStep, guard: &GitRootPreservationGuard,) -> ModelResult<GitCheckedPreservationMutation> => preservation_root::execute_root_preservation_step_checked);
    delegate!(index_matches_candidate_files(path: &Path, expected_files: &[GitCandidateFile], absent_paths: &[String],) -> ModelResult<bool> => preservation::index_matches_candidate_files);
    delegate!(index_entries_match_candidate_files(path: &Path, expected_files: &[GitCandidateFile], absent_paths: &[String],) -> ModelResult<bool> => preservation::index_entries_match_candidate_files);
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
