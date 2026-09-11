//! Test-only dispatch for the complete GitRepository contract. All methods
//! forward, including optional methods: native behavior must not fall back to
//! a trait default merely because a caller went through the factory.
use super::*;

#[derive(Clone, Copy)]
enum Mode {
    Real,
    Fake,
}

pub(super) fn make_repository() -> GitTestRepository {
    let mode = if crate::test_backend::modes().fake_git {
        Mode::Fake
    } else {
        Mode::Real
    };
    match mode {
        Mode::Real => GitTestRepository::Real(Box::default()),
        Mode::Fake => GitTestRepository::Fake(Box::new(FakeGitRepository::shared())),
    }
}

#[derive(Clone)]
pub(crate) enum GitTestRepository {
    Real(Box<Git2Repository>),
    Fake(Box<FakeGitRepository>),
}
macro_rules! forward {
    ($name:ident($($arg:ident: $arg_type:ty),* $(,)?) -> $result:ty) => {
        fn $name(&self, $($arg: $arg_type),*) -> $result {
            match self {
                Self::Real(repo) => repo.$name($($arg),*),
                Self::Fake(repo) => repo.$name($($arg),*),
            }
        }
    };
}

impl GitRepository for GitTestRepository {
    forward!(repository_index(path: &Path) -> ModelResult<GitIndexSnapshot>);
    forward!(repository_paths(path: &Path) -> ModelResult<GitRepositoryPaths>);
    forward!(root_preservation_image(root: &Path, clean: &GitRootManagedForm, excluded: &[String]) -> ModelResult<GitPreservationImage>);
    forward!(validate_root_preservation_spec(root: &Path, spec: &GitRootPreservationSpec) -> ModelResult<()>);
    forward!(root_managed_index_matches(root: &Path, form: &GitRootManagedIndexForm) -> ModelResult<bool>);
    forward!(rewrite_root_managed_index_checked(root: &Path, form: &GitRootManagedIndexForm) -> ModelResult<()>);
    forward!(test_init_repo(repo: &Path, spec: &TestRepoSpec) -> ModelResult<()>);
    forward!(test_create_commit(repo: &Path, spec: &TestCommitSpec) -> ModelResult<String>);
    forward!(test_read_commit(repo: &Path, oid: &str) -> ModelResult<TestCommit>);
    forward!(test_set_ref(repo: &Path, name: &str, target: Option<&TestRefTarget>) -> ModelResult<()>);
    forward!(test_set_head(repo: &Path, state: &TestHead) -> ModelResult<()>);
    forward!(test_replace_index(repo: &Path, entries: &[TestIndexEntry]) -> ModelResult<()>);
    forward!(test_read_index(repo: &Path) -> ModelResult<Vec<TestIndexEntry>>);
    forward!(test_set_config(repo: &Path, key: &str, values: &[String]) -> ModelResult<()>);
    forward!(test_read_config(repo: &Path, key: &str) -> ModelResult<Vec<String>>);
    forward!(test_force_checkout(repo: &Path, commit: &str) -> ModelResult<()>);
    forward!(test_reset_mixed(repo: &Path, commit: &str) -> ModelResult<()>);
    forward!(test_set_repository_state(repo: &Path, state: GitRepositoryState, merge_head: Option<&str>) -> ModelResult<()>);
    forward!(test_seed_merge_conflict(repo: &Path, before: &str, source: &str) -> ModelResult<GitMergeConflictSnapshot>);
    forward!(test_create_commit_from_parent(repo: &Path, parent: &str, message: &str, edits: &[TestCommitFileEdit]) -> ModelResult<String>);

    forward!(preservation_stashes(_path: &Path, _merge_id: &str,) -> ModelResult<Vec<GitPreservationStashEvidence>>);
    fn with_transport(
        &self,
        start: &Path,
        options: Option<&crate::TransportOptions>,
    ) -> ModelResult<Option<Self>> {
        match self {
            Self::Real(repo) => repo
                .with_transport(start, options)
                .map(|value| value.map(|repo| Self::Real(Box::new(repo)))),
            Self::Fake(repo) => repo
                .with_transport(start, options)
                .map(|value| value.map(|repo| Self::Fake(Box::new(repo)))),
        }
    }
    forward!(transport_observations() -> Option<super::transport_observations::TransportObservations>);
    forward!(validate_transport_remotes(_names: &[String]) -> ModelResult<()>);
    forward!(remote_identity(_path: &Path, _remote: &str) -> ModelResult<Option<String>>);
    forward!(set_remote_identity(_path: &Path, _remote: &str, _value: Option<&str>,) -> ModelResult<()>);
    forward!(validate_remote_identity(_path: &Path, _remote: &str, _push: bool,) -> ModelResult<()>);
    forward!(validate_url_identity(_identity_repo: Option<&Path>, _remote: &str, _url: &str,) -> ModelResult<()>);
    forward!(read_remote_file(_url: &str, _remote: &str, _relative_path: &str,) -> ModelResult<Option<Vec<u8>>>);
    forward!(is_repository(path: &Path) -> ModelResult<bool>);
    forward!(commit_exists(_path: &Path, _oid: &str) -> ModelResult<bool>);
    forward!(read_file_at_commit(_path: &Path, _commit: &str, _relative_path: &str,) -> ModelResult<Option<Vec<u8>>>);
    forward!(commit_matches_merge(_path: &Path, _commit: &str, _first_parent: &str, _second_parent: &str, _message: &str,) -> ModelResult<bool>);
    forward!(commit_matches_prepared_merge(_path: &Path, _commit: &str, _first_parent: &str, _second_parent: &str, _message: &str, _prepared: &GitPreparedCommit,) -> ModelResult<bool>);
    forward!(create_repo(path: &Path) -> ModelResult<GitCreateResult>);
    forward!(clone_repo(url: &str, path: &Path) -> ModelResult<GitCloneResult>);
    forward!(clone_repo_with_progress(url: &str, path: &Path, _progress: &dyn Fn(crate::GitTransferProgress),) -> ModelResult<GitCloneResult>);
    forward!(clone_repo_named(url: &str, path: &Path, remote: &str, progress: &dyn Fn(crate::GitTransferProgress),) -> ModelResult<GitCloneResult>);
    forward!(fetch(path: &Path, remote: &str) -> ModelResult<GitFetchResult>);
    forward!(ls_remote(path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>>);
    forward!(ls_remote_url(_path: &Path, _url: &str, _remote_name: &str, _identity_repo: Option<&Path>,) -> ModelResult<Vec<GitRemoteRef>>);
    forward!(fast_forward(path: &Path, branch: &str, upstream_ref: &str,) -> ModelResult<GitUpdateResult>);
    forward!(merge_upstream(path: &Path, branch: &str, upstream_ref: &str,) -> ModelResult<GitIntegrateResult>);
    forward!(merge_upstream_checked(_path: &Path, _branch: &str, _expected_before: &str, _source_commit: &str, _message: &str, _attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitIntegrateResult>);
    forward!(prepare_merge_upstream_checked(_path: &Path, _branch: &str, _expected_before: &str, _source_commit: &str, _attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitPreparedMerge>);
    forward!(prepare_merge_upstream_mode_checked(path: &Path, branch: &str, expected_before: &str, source_commit: &str, mode: GitPreparedMergeMode, attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitPreparedMerge>);
    forward!(validate_prepared_merge_upstream_state(_path: &Path, _branch: &str, _expected_before: &str, _source_commit: &str, _prepared: &GitPreparedMerge,) -> ModelResult<()>);
    forward!(execute_prepared_merge_upstream_checked(_path: &Path, _branch: &str, _expected_before: &str, _source_commit: &str, _message: &str, _prepared: &GitPreparedMerge,) -> ModelResult<GitIntegrateResult>);
    forward!(merge_analysis(_path: &Path, _target_branch: &str, _source: &str,) -> ModelResult<GitMergeAnalysis>);
    forward!(merge_simulate(_path: &Path, _target_commit: &str, _source_commit: &str,) -> ModelResult<GitMergeSimulation>);
    forward!(merge_state(_path: &Path) -> ModelResult<Option<GitNativeMergeState>>);
    forward!(repository_state(_path: &Path) -> ModelResult<GitRepositoryState>);
    forward!(validate_merge_recovery_state(_path: &Path, _expected_before: &str, _expected_merge_head: &str, _require_resolved: bool,) -> ModelResult<()>);
    forward!(merge_conflict_snapshot(_path: &Path, _expected_before: &str, _expected_merge_head: &str,) -> ModelResult<GitMergeConflictSnapshot>);
    forward!(validate_prepared_merge_resolution_state(_path: &Path, _target_branch: &str, _expected_before: &str, _expected_merge_head: &str, _prepared: &GitPreparedCommit,) -> ModelResult<()>);
    forward!(abort_merge(_path: &Path, _expected_before: &str, _expected_merge_head: &str,) -> ModelResult<()>);
    forward!(set_branch_target_checked(_path: &Path, _branch: &str, _expected_current: &str, _target: &str,) -> ModelResult<GitUpdateResult>);
    forward!(delete_branch_target_checked(_path: &Path, _branch: &str, _expected_current: &str,) -> ModelResult<()>);
    forward!(create_backup_ref(_path: &Path, _name: &str, _target: &str,) -> ModelResult<GitBackupRefResult>);
    forward!(create_backup_ref_checked(_path: &Path, _branch: &str, _expected_head: &str, _name: &str, _target: &str,) -> ModelResult<GitBackupRefResult>);
    forward!(delete_backup_ref_checked(_path: &Path, _name: &str, _expected_target: &str,) -> ModelResult<()>);
    forward!(stash_for_merge_preservation(_path: &Path, _merge_id: &str, _include_untracked: bool,) -> ModelResult<GitStashPushResult>);
    forward!(stash_for_merge_preservation_checked(_path: &Path, _branch: &str, _expected_head: &str, _expected_preimage_sha256: &str, _merge_id: &str, _include_untracked: bool,) -> ModelResult<GitStashPushResult>);
    forward!(preservation_image(_path: &Path, _include_untracked: bool,) -> ModelResult<GitPreservationImage>);
    forward!(observe_direct_ref(_path: &Path, _name: &str,) -> ModelResult<GitDirectRefObservation>);
    forward!(checkout_matches_commit(_path: &Path, _branch: &str, _commit: &str,) -> ModelResult<bool>);
    forward!(checkout_matches_commit_except(_path: &Path, _commit: &str, _allowed_paths: &[String],) -> ModelResult<bool>);
    forward!(checkout_matches_commit_with_overlay(path: &Path, commit: &str, overlay: &GitCheckoutOverlay,) -> ModelResult<bool>);
    forward!(prepare_root_preservation_stash(_root: &Path, _spec: &GitRootPreservationSpec,) -> ModelResult<GitPreparedRootStash>);
    forward!(observe_root_preservation_step(_root: &Path, _spec: &GitRootPreservationSpec, _step: &GitRootPreservationPhysicalStep, _guard: &GitRootPreservationGuard,) -> ModelResult<GitRootPreservationStepObservation>);
    forward!(execute_root_preservation_step_checked(_root: &Path, _spec: &GitRootPreservationSpec, _step: &GitRootPreservationPhysicalStep, _guard: &GitRootPreservationGuard,) -> ModelResult<GitCheckedPreservationMutation>);
    forward!(index_matches_candidate_files(_path: &Path, _expected_files: &[GitCandidateFile], _absent_paths: &[String],) -> ModelResult<bool>);
    forward!(index_entries_match_candidate_files(_path: &Path, _expected_files: &[GitCandidateFile], _absent_paths: &[String],) -> ModelResult<bool>);
    forward!(commit_bootstrap_paths_checked(_root: &Path, _expected_head: Option<&str>, _paths: &[&str], _message: &str,) -> ModelResult<Option<GitScopedCommitResult>>);
    forward!(commit_gwz_paths_checked(_root: &Path, _expected_head: Option<&str>, _candidate_files: &[GitCandidateFile], _message: &str,) -> ModelResult<GitScopedCommitResult>);
    forward!(verify_gwz_paths_commit(_root: &Path, _commit: &str, _expected_parent: Option<&str>, _candidate_files: &[GitCandidateFile], _message: &str,) -> ModelResult<GitScopedCommitResult>);
    forward!(rollback_gwz_paths_commit_checked(_root: &Path, _branch: &str, _commit: &str, _expected_parent: Option<&str>, _candidate_files: &[GitCandidateFile], _message: &str,) -> ModelResult<()>);
    forward!(rebase_onto(path: &Path, branch: &str, upstream_ref: &str,) -> ModelResult<GitIntegrateResult>);
    forward!(reset_hard(path: &Path, branch: &str, upstream_ref: &str,) -> ModelResult<GitUpdateResult>);
    forward!(checkout_commit(path: &Path, commit: &str) -> ModelResult<GitUpdateResult>);
    forward!(checkout_branch(path: &Path, branch: &str, commit: &str,) -> ModelResult<GitUpdateResult>);
    forward!(branch_list(_path: &Path) -> ModelResult<Vec<GitBranch>>);
    forward!(branch_create(_path: &Path, _branch: &str, _start_ref: &str,) -> ModelResult<GitBranchCreateResult>);
    forward!(branch_delete(_path: &Path, _branch: &str) -> ModelResult<()>);
    forward!(switch_branch(_path: &Path, _branch: &str) -> ModelResult<GitUpdateResult>);
    forward!(stash_push(_path: &Path, _message: &str, _options: GitStashPushOptions,) -> ModelResult<GitStashPushResult>);
    forward!(stash_list(_path: &Path) -> ModelResult<Vec<GitStashEntry>>);
    forward!(stash_apply(_path: &Path, _target: &GitStashTarget, _options: GitStashRestoreOptions,) -> ModelResult<()>);
    forward!(stash_pop(_path: &Path, _target: &GitStashTarget, _options: GitStashRestoreOptions,) -> ModelResult<()>);
    forward!(stash_drop(_path: &Path, _target: &GitStashTarget) -> ModelResult<()>);
    forward!(status(path: &Path) -> ModelResult<GitStatus>);
    forward!(status_with_options(path: &Path, _options: GitStatusOptions,) -> ModelResult<GitStatus>);
    forward!(head(path: &Path) -> ModelResult<GitHeadState>);
    forward!(remotes(path: &Path) -> ModelResult<Vec<GitRemote>>);
    forward!(add_remote(path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult>);
    forward!(push(path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult>);
    forward!(prepare_push(_path: &Path, _remote: &str, _refspec: &str,) -> ModelResult<GitPreparedPush>);
    forward!(push_prepared(_path: &Path, _plan: &GitPreparedPush) -> ModelResult<GitPushResult>);
    forward!(fetch_anonymous(path: &Path, url: &str, refspecs: &[&str],) -> ModelResult<GitFetchResult>);
    forward!(push_anonymous(path: &Path, url: &str, refspec: &str) -> ModelResult<GitPushResult>);
    forward!(read_ref(path: &Path, ref_spec: &str) -> ModelResult<Option<String>>);
    forward!(is_ancestor(path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool>);
    forward!(merge_base(_path: &Path, _left: &str, _right: &str) -> ModelResult<Option<String>>);
    forward!(changed_paths_between(_path: &Path, _old_commit: &str, _new_commit: &str,) -> ModelResult<Vec<String>>);
    forward!(diff_manifest(_path: &Path, _comparison: &crate::diff::RepoDiffComparison, _options: &crate::diff::RepoDiffOptions,) -> ModelResult<crate::diff::RepoDiffManifest>);
    forward!(resolve_comparison(_path: &Path, _spec: &crate::diff::ComparisonSpec,) -> ModelResult<crate::diff::RepoDiffComparison>);
    forward!(stage_paths(path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult>);
    forward!(stage_paths_allowing_other_conflicts(path: &Path, pathspecs: &[&str],) -> ModelResult<GitStageResult>);
    forward!(commit(path: &Path, message: &str, all: bool) -> ModelResult<GitCommitResult>);
    forward!(commit_merge_resolution(path: &Path, message: &str) -> ModelResult<GitCommitResult>);
    forward!(commit_merge_resolution_checked(_path: &Path, _target_branch: &str, _expected_before: &str, _expected_merge_head: &str, _message: &str, _attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitCommitResult>);
    forward!(prepare_merge_resolution_checked(_path: &Path, _target_branch: &str, _expected_before: &str, _expected_merge_head: &str, _attribution: Option<&crate::model::OperationAttribution>,) -> ModelResult<GitPreparedCommit>);
    forward!(commit_prepared_merge_resolution_checked(_path: &Path, _target_branch: &str, _expected_before: &str, _expected_merge_head: &str, _message: &str, _prepared: &GitPreparedCommit,) -> ModelResult<GitCommitResult>);
    forward!(tag_create(path: &Path, name: &str, message: Option<&str>, signed: bool,) -> ModelResult<GitTagResult>);
    forward!(tag_list(path: &Path) -> ModelResult<Vec<String>>);
    forward!(tag_delete(path: &Path, name: &str) -> ModelResult<()>);
    forward!(tag_fetch(path: &Path, remote: &str) -> ModelResult<GitFetchResult>);
}
