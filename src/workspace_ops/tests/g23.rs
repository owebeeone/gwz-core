use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::artifact::read_lock;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::runtime::clock::{FixedClock, TimestampMs};
use crate::runtime::ids::SequentialIdProvider;
use crate::workspace_ops::merge::{
    FileMergeStore, MergeDependencies, MergeOperationRecord, MergeStore, OperationState,
    PublicationStep, handle_merge_with_dependencies,
};
use sha2::{Digest, Sha256};

use super::*;

fn request(dry_run: bool) -> crate::MergeRequest {
    let mut meta = request_meta();
    meta.dry_run = dry_run.then_some(true);
    crate::MergeRequest {
        meta,
        op: crate::MergeOp::Start,
        source_ref: Some("feature/source".to_owned()),
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug)]
enum FinalizationFault {
    AfterEnteringFinalizing,
    BeforeCandidateCreation,
    AfterCandidatePersistence,
    AfterEvidenceCommit,
    AfterLockPublication,
    BeforeArchive,
}

struct FaultingMergeStore {
    fault: FinalizationFault,
    fired: Cell<bool>,
}

impl FaultingMergeStore {
    fn new(fault: FinalizationFault) -> Self {
        Self {
            fault,
            fired: Cell::new(false),
        }
    }

    fn should_fail_write(&self, root: &Path, record: &MergeOperationRecord) -> bool {
        let Some(publication) = record.publication.as_ref() else {
            return false;
        };
        match self.fault {
            FinalizationFault::AfterEnteringFinalizing => {
                publication.step == PublicationStep::NotStarted
            }
            FinalizationFault::BeforeCandidateCreation => {
                publication.step == PublicationStep::PreparingCandidate
                    && publication.candidate.is_none()
            }
            FinalizationFault::AfterCandidatePersistence => {
                publication.step == PublicationStep::CommittingEvidence
                    && publication.candidate.is_some()
                    && publication.composition_commit.is_none()
            }
            FinalizationFault::AfterEvidenceCommit => publication.composition_commit.is_some(),
            FinalizationFault::AfterLockPublication => {
                let actual = fs::read(root.join(crate::artifact::LOCK_PATH))
                    .ok()
                    .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
                publication.step == PublicationStep::PublishingCandidate
                    && publication.composition_commit.is_some()
                    && actual.as_deref() == publication.candidate_lock_sha256.as_deref()
            }
            FinalizationFault::BeforeArchive => false,
        }
    }

    fn inject(&self) -> ModelResult<()> {
        self.fired.set(true);
        Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!("injected {:?} failure", self.fault),
        ))
    }
}

impl MergeStore for FaultingMergeStore {
    fn discover_open(&self, root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        FileMergeStore.discover_open(root)
    }

    fn load(&self, root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
        FileMergeStore.load(root, merge_id)
    }

    fn write_open(&self, root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
        if !self.fired.get() && self.should_fail_write(root, record) {
            return self.inject();
        }
        FileMergeStore.write_open(root, record)
    }

    fn archive(&self, root: &Path, merge_id: &str) -> ModelResult<()> {
        if !self.fired.get() && matches!(self.fault, FinalizationFault::BeforeArchive) {
            return self.inject();
        }
        FileMergeStore.archive(root, merge_id)
    }
}

fn invoke_with_store(
    backend: &crate::git::Git2Backend,
    store: &FaultingMergeStore,
    root: &Path,
    request: crate::MergeRequest,
    operation_id: &str,
) -> ModelResult<crate::MergeResponse> {
    let clock = FixedClock::new(TimestampMs(1_700_000_000_000));
    let mut ids = SequentialIdProvider::new();
    handle_merge_with_dependencies(
        MergeDependencies {
            backend,
            store,
            clock: &clock,
            ids: &mut ids,
            events: &crate::operation::NullSink,
        },
        root,
        request,
        operation_id,
    )
}

fn feature_commit(
    backend: &crate::git::Git2Backend,
    repo: &std::path::Path,
    file: &str,
    content: &str,
) -> (String, String) {
    let base = backend.head(repo).unwrap().commit.unwrap();
    backend
        .branch_create(repo, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(repo, "feature/source").unwrap();
    let source = commit_file(
        repo,
        file,
        content,
        "source",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    backend.switch_branch(repo, "main").unwrap();
    (base, source)
}

fn init_two_member_workspace(
    root: &std::path::Path,
    backend: &crate::git::Git2Backend,
) -> (RemoteFixture, RemoteFixture) {
    let app = RemoteFixture::new("merge-start-app");
    let lib = RemoteFixture::new("merge-start-lib");
    app.commit_and_push("README.md", "base\n", "initial", backend);
    lib.commit_and_push("README.md", "base\n", "initial", backend);
    handle_init_from_sources(
        backend,
        root,
        crate::InitFromSourcesRequest {
            meta: request_meta(),
            workspace_root: root.to_string_lossy().into_owned(),
            sources: vec![
                crate::SourceUrl {
                    url: app.remote_url().to_owned(),
                    path: Some("app".to_owned()),
                    remote_name: None,
                    branch: None,
                },
                crate::SourceUrl {
                    url: lib.remote_url().to_owned(),
                    path: Some("lib".to_owned()),
                    remote_name: None,
                    branch: None,
                },
            ],
            target: None,
            workspace_id: Some("ws_ops".to_owned()),
        },
        "op_init",
        &CollectingSink::default(),
    )
    .unwrap();
    (app, lib)
}

struct MixedMergeFixture {
    _remotes: [RemoteFixture; 3],
    app_before: String,
    lib_before: String,
    docs_before: String,
    docs_source: String,
}

fn init_mixed_merge_workspace(
    root: &std::path::Path,
    backend: &crate::git::Git2Backend,
) -> MixedMergeFixture {
    let app = RemoteFixture::new("merge-mixed-app");
    let lib = RemoteFixture::new("merge-mixed-lib");
    let docs = RemoteFixture::new("merge-mixed-docs");
    for fixture in [&app, &lib, &docs] {
        fixture.commit_and_push("README.md", "base\n", "initial", backend);
    }
    handle_init_from_sources(
        backend,
        root,
        crate::InitFromSourcesRequest {
            meta: request_meta(),
            workspace_root: root.to_string_lossy().into_owned(),
            sources: [(&app, "app"), (&lib, "lib"), (&docs, "docs")]
                .into_iter()
                .map(|(fixture, path)| crate::SourceUrl {
                    url: fixture.remote_url().to_owned(),
                    path: Some(path.to_owned()),
                    remote_name: None,
                    branch: None,
                })
                .collect(),
            target: None,
            workspace_id: Some("ws_ops".to_owned()),
        },
        "op_init",
        &CollectingSink::default(),
    )
    .unwrap();

    let app_path = root.join("app");
    let lib_path = root.join("lib");
    let docs_path = root.join("docs");
    let app_before = backend.head(&app_path).unwrap().commit.unwrap();
    backend
        .branch_create(&app_path, "feature/source", "HEAD")
        .unwrap();

    let (lib_base, _) = feature_commit(backend, &lib_path, "source.txt", "source\n");
    let lib_before = commit_file(
        &lib_path,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&lib_base).unwrap()],
    )
    .unwrap();
    let (docs_base, docs_source) = feature_commit(backend, &docs_path, "README.md", "source\n");
    let docs_before = commit_file(
        &docs_path,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&docs_base).unwrap()],
    )
    .unwrap();
    MixedMergeFixture {
        _remotes: [app, lib, docs],
        app_before,
        lib_before,
        docs_before,
        docs_source,
    }
}

fn recovery_request(op: crate::MergeOp, merge_id: Option<String>) -> crate::MergeRequest {
    crate::MergeRequest {
        meta: request_meta(),
        op,
        merge_id,
        ..Default::default()
    }
}

fn merge_repo<'a>(
    response: &'a crate::MergeResponse,
    target_id: &str,
) -> &'a crate::MergeRepoSummary {
    response
        .repos
        .iter()
        .find(|repo| repo.target_id == target_id)
        .unwrap()
}

fn workspace_file_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            let path = entry.path();
            if file_type.is_dir() {
                visit(root, &path, files);
            } else if file_type.is_file() {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn assert_open_merge_blocks_all_starts_without_mutation(
    root: &Path,
    backend: &crate::git::Git2Backend,
    merge_id: &str,
) {
    let unrelated = TempDir::new("merge-gate-unrelated-cwd");
    let before = workspace_file_snapshot(root);

    for dry_run in [true, false] {
        let mut rejected = request(dry_run);
        rejected.meta.workspace = Some(crate::WorkspaceRef {
            root: Some(root.to_string_lossy().into_owned()),
            workspace_id: None,
        });

        let error = handle_merge(
            backend,
            unrelated.path(),
            rejected,
            if dry_run {
                "op_rejected_dry_run"
            } else {
                "op_rejected_real"
            },
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::OpenOperation);
        assert!(error.message.contains(merge_id));
        assert_eq!(workspace_file_snapshot(root), before);
    }
}

fn force_open_merge_state(root: &Path, state: OperationState) -> String {
    let store = FileMergeStore;
    let mut record = store.discover_open(root).unwrap().unwrap();
    record.state = state;
    let merge_id = record.merge_id.clone();
    store.write_open(root, &record).unwrap();
    merge_id
}

#[test]
fn first_class_merge_fast_forwards_and_publishes_durable_composition() {
    let temp = TempDir::new("merge-start-ff");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-start-ff-source");
    let member = temp.path().join("remote");
    let (base, source) = feature_commit(&backend, &member, "README.md", "source\n");

    let response = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();

    assert_eq!(response.response.meta.action, crate::ActionKind::Merge);
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(response.repos[0].source_ref, "feature/source");
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert!(!response.open);
    assert_eq!(
        response.publication_step,
        Some(crate::MergePublicationStep::Complete)
    );
    assert_eq!(response.merge_id.as_deref(), Some("merge_op_merge_0001"));
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(source.as_str())
    );
    assert!(
        temp.path()
            .join(".gwz/merge/done/merge_op_merge_0001.yaml")
            .is_file()
    );
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_remote"]
            .commit
            .as_deref(),
        Some(source.as_str())
    );
    assert_ne!(base, source);
    let markers = crate::artifact::list_markers(temp.path()).unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(
        markers[0].merge.as_ref().unwrap().participants["mem_remote"].resulting_commit,
        source
    );
}

#[test]
fn finalization_faults_report_status_and_resume_without_duplicate_evidence() {
    for fault in [
        FinalizationFault::AfterEnteringFinalizing,
        FinalizationFault::BeforeCandidateCreation,
        FinalizationFault::AfterCandidatePersistence,
        FinalizationFault::AfterEvidenceCommit,
        FinalizationFault::AfterLockPublication,
        FinalizationFault::BeforeArchive,
    ] {
        let temp = TempDir::new(&format!("merge-finalize-{fault:?}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture =
            init_one_member_workspace(temp.path(), &backend, &format!("merge-fault-{fault:?}"));
        let member = temp.path().join("remote");
        feature_commit(&backend, &member, "README.md", "source\n");
        let baseline_root = backend.head(temp.path()).unwrap().commit;
        let store = FaultingMergeStore::new(fault);

        let failure = invoke_with_store(&backend, &store, temp.path(), request(false), "op_fault")
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::MergeRecoveryRequired, "{fault:?}");
        assert!(store.fired.get(), "{fault:?}");

        let before_status = store.discover_open(temp.path()).unwrap().unwrap();
        let head_before_status = backend.head(temp.path()).unwrap();
        let status = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Status, None),
            "op_status",
        )
        .unwrap();
        assert!(matches!(
            status.state,
            crate::MergeOperationState::Finalizing | crate::MergeOperationState::Completed
        ));
        assert_eq!(
            store.discover_open(temp.path()).unwrap().unwrap(),
            before_status,
            "status changed the durable record at {fault:?}"
        );
        assert_eq!(
            backend.head(temp.path()).unwrap(),
            head_before_status,
            "status changed root HEAD at {fault:?}"
        );

        let merge_id = before_status.merge_id.clone();
        let completed = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
            "op_resume",
        )
        .unwrap();
        assert_eq!(completed.state, crate::MergeOperationState::Completed);
        assert!(!completed.open);
        assert!(store.discover_open(temp.path()).unwrap().is_none());

        let archived = store.load(temp.path(), &merge_id).unwrap();
        let composition = archived
            .publication
            .as_ref()
            .and_then(|publication| publication.composition_commit.as_deref())
            .unwrap();
        assert_eq!(
            backend.head(temp.path()).unwrap().commit.as_deref(),
            Some(composition)
        );
        let repository = git2::Repository::open(temp.path()).unwrap();
        let commit = repository
            .find_commit(git2::Oid::from_str(composition).unwrap())
            .unwrap();
        match baseline_root {
            Some(baseline) => {
                assert_eq!(commit.parent_count(), 1);
                assert_eq!(commit.parent_id(0).unwrap().to_string(), baseline);
            }
            None => assert_eq!(commit.parent_count(), 0),
        }
    }
}

#[test]
fn candidate_artifact_drift_blocks_continue_and_abort_without_mutation() {
    let temp = TempDir::new("merge-finalize-candidate-drift");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-candidate-drift-source");
    let member = temp.path().join("remote");
    feature_commit(&backend, &member, "README.md", "source\n");
    let store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);
    invoke_with_store(&backend, &store, temp.path(), request(false), "op_fault").unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    let candidate = record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .unwrap();
    let marker = crate::artifact::marker_path(temp.path(), &candidate.marker_id);
    fs::remove_file(&marker).unwrap();
    let root_head = backend.head(temp.path()).unwrap();
    let member_head = backend.head(&member).unwrap();

    let continued = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
        "op_continue",
    )
    .unwrap();
    assert_eq!(continued.state, crate::MergeOperationState::Finalizing);
    assert!(
        continued.operation_drift.iter().any(|drift| {
            drift.kind == crate::MergeOperationDriftKind::RootCandidateStateChanged
        })
    );
    assert!(!marker.exists());

    let aborted = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(record.merge_id)),
        "op_abort",
    )
    .unwrap_err();
    assert_eq!(aborted.code, ErrorCode::MergeDrift);
    assert_eq!(backend.head(temp.path()).unwrap(), root_head);
    assert_eq!(backend.head(&member).unwrap(), member_head);
}

#[test]
fn root_branch_switch_at_the_same_commit_blocks_finalization() {
    let temp = TempDir::new("merge-finalize-root-branch-drift");
    let backend = crate::git::Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "merge-root-branch-drift-source");
    commit_file(temp.path(), "root.txt", "root\n", "root baseline", &[]).unwrap();
    let member = temp.path().join("remote");
    feature_commit(&backend, &member, "README.md", "source\n");
    let store = FaultingMergeStore::new(FinalizationFault::AfterEnteringFinalizing);
    invoke_with_store(&backend, &store, temp.path(), request(false), "op_fault").unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    let baseline_root = backend.head(temp.path()).unwrap().commit.unwrap();
    backend.branch_create(temp.path(), "other", "HEAD").unwrap();
    backend.switch_branch(temp.path(), "other").unwrap();

    let continued = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id)),
        "op_continue",
    )
    .unwrap();

    assert_eq!(continued.state, crate::MergeOperationState::Finalizing);
    assert!(
        continued.operation_drift.iter().any(|drift| {
            drift.kind == crate::MergeOperationDriftKind::RootCandidateStateChanged
        })
    );
    let root_head = backend.head(temp.path()).unwrap();
    assert_eq!(root_head.branch.as_deref(), Some("other"));
    assert_eq!(root_head.commit.as_deref(), Some(baseline_root.as_str()));
    assert!(
        store
            .discover_open(temp.path())
            .unwrap()
            .unwrap()
            .publication
            .unwrap()
            .composition_commit
            .is_none()
    );
}

#[test]
fn all_up_to_date_merge_completes_without_root_evidence() {
    let temp = TempDir::new("merge-all-up-to-date");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-no-op-source");
    let member = temp.path().join("remote");
    backend
        .branch_create(&member, "feature/source", "HEAD")
        .unwrap();
    let root_before = backend.head(temp.path()).unwrap();

    let response = handle_merge(&backend, temp.path(), request(false), "op_noop").unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::UpToDate
    );
    assert_eq!(backend.head(temp.path()).unwrap(), root_before);
    assert!(
        crate::artifact::list_markers(temp.path())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn finalization_preserves_unrelated_root_staged_dirty_and_untracked_work() {
    let temp = TempDir::new("merge-root-local-work");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-work-source");
    let member = temp.path().join("remote");
    feature_commit(&backend, &member, "README.md", "source\n");

    commit_file(
        temp.path(),
        "root-note.txt",
        "accepted\n",
        "root baseline",
        &[],
    )
    .unwrap();
    fs::write(temp.path().join("root-note.txt"), "dirty\n").unwrap();
    fs::write(temp.path().join("staged-local.txt"), "staged\n").unwrap();
    backend
        .stage_paths(temp.path(), &["staged-local.txt"])
        .unwrap();
    fs::write(temp.path().join("untracked-local.txt"), "untracked\n").unwrap();
    let repository = git2::Repository::open(temp.path()).unwrap();
    let staged_before = repository
        .index()
        .unwrap()
        .get_path(Path::new("staged-local.txt"), 0)
        .unwrap()
        .id;

    let response = handle_merge(&backend, temp.path(), request(false), "op_local_work").unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        fs::read_to_string(temp.path().join("root-note.txt")).unwrap(),
        "dirty\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("untracked-local.txt")).unwrap(),
        "untracked\n"
    );
    let repository = git2::Repository::open(temp.path()).unwrap();
    let staged_after = repository
        .index()
        .unwrap()
        .get_path(Path::new("staged-local.txt"), 0)
        .unwrap()
        .id;
    assert_eq!(staged_after, staged_before);
    let head_tree = repository.head().unwrap().peel_to_tree().unwrap();
    assert!(head_tree.get_path(Path::new("staged-local.txt")).is_err());
}

#[test]
fn first_class_true_merge_uses_request_git_identities_and_planned_message() {
    let temp = TempDir::new("merge-start-identity");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-identity-source");
    let member = temp.path().join("remote");
    let (base, _) = feature_commit(&backend, &member, "source.txt", "source\n");
    commit_file(
        &member,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    let mut request = request(false);
    request.meta.attribution = Some(crate::OperationAttribution {
        actor: None,
        git_author: Some(crate::GitObjectIdentity {
            name: "Merge Author".to_owned(),
            email: "author@example.invalid".to_owned(),
            time_ms: Some(1_700_000_000_000),
            timezone_offset_minutes: Some(600),
        }),
        git_committer: Some(crate::GitObjectIdentity {
            name: "Merge Committer".to_owned(),
            email: "committer@example.invalid".to_owned(),
            time_ms: Some(1_700_000_100_000),
            timezone_offset_minutes: Some(-300),
        }),
        credential_ref: None,
    });

    let response = handle_merge(&backend, temp.path(), request, "op_merge").unwrap();
    let oid = git2::Oid::from_str(response.repos[0].resulting_commit.as_deref().unwrap()).unwrap();
    let repo = git2::Repository::open(&member).unwrap();
    let commit = repo.find_commit(oid).unwrap();

    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::Merged
    );
    assert_eq!(
        commit.message(),
        Ok(
            "Merge 'feature/source' into 'main'\n\nGWZ-Merge-ID: merge_op_merge_0001\nGWZ-Operation-ID: op_merge"
        )
    );
    assert_eq!(commit.author().name(), Ok("Merge Author"));
    assert_eq!(commit.author().when().offset_minutes(), 600);
    assert_eq!(commit.committer().name(), Ok("Merge Committer"));
    assert_eq!(commit.committer().when().offset_minutes(), -300);
}

#[test]
fn invalid_identity_rejects_mixed_batch_before_fast_forward_mutation() {
    let temp = TempDir::new("merge-start-invalid-identity");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (app_before, _) = feature_commit(&backend, &app, "source.txt", "source\n");
    let (lib_base, _) = feature_commit(&backend, &lib, "source.txt", "source\n");
    let lib_before = commit_file(
        &lib,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&lib_base).unwrap()],
    )
    .unwrap();
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
    let mut request = request(false);
    request.meta.attribution = Some(crate::OperationAttribution {
        actor: None,
        git_author: Some(crate::GitObjectIdentity {
            name: "Invalid <Author>".to_owned(),
            email: "author@example.invalid".to_owned(),
            time_ms: None,
            timezone_offset_minutes: None,
        }),
        git_committer: None,
        credential_ref: None,
    });

    let error = handle_merge(&backend, temp.path(), request, "op_merge").unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("git_identity.name"));
    assert_eq!(
        backend.head(&app).unwrap().commit.as_deref(),
        Some(app_before.as_str())
    );
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(lib_before.as_str())
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert!(backend.merge_state(&app).unwrap().is_none());
    assert!(backend.merge_state(&lib).unwrap().is_none());
}

#[test]
fn first_class_merge_dry_run_does_not_change_head_lock_or_merge_state() {
    let temp = TempDir::new("merge-start-dry");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-start-dry-source");
    let member = temp.path().join("remote");
    let (base, _) = feature_commit(&backend, &member, "README.md", "source\n");
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let response = handle_merge(&backend, temp.path(), request(true), "op_merge_dry").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Accepted
    );
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::Planned
    );
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(base.as_str())
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert!(backend.merge_state(&member).unwrap().is_none());
}

#[test]
fn open_awaiting_resolution_blocks_dry_run_and_real_starts_from_an_explicit_root() {
    let temp = TempDir::new("merge-start-gate-awaiting");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );

    assert_open_merge_blocks_all_starts_without_mutation(
        temp.path(),
        &backend,
        started.merge_id.as_deref().unwrap(),
    );
}

#[test]
fn open_finalizing_blocks_dry_run_and_real_starts_from_an_explicit_root() {
    let temp = TempDir::new("merge-start-gate-finalizing");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-gate-finalizing");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(started.state, crate::MergeOperationState::Completed);
    let merge_id = started.merge_id.clone().unwrap();
    let done = temp.path().join(format!(".gwz/merge/done/{merge_id}.yaml"));
    let mut record: MergeOperationRecord =
        serde_yaml::from_slice(&fs::read(&done).unwrap()).unwrap();
    record.state = OperationState::Finalizing;
    FileMergeStore.write_open(temp.path(), &record).unwrap();

    assert_open_merge_blocks_all_starts_without_mutation(temp.path(), &backend, &merge_id);
}

#[test]
fn open_halted_blocks_dry_run_and_real_starts_from_an_explicit_root() {
    let temp = TempDir::new("merge-start-gate-halted");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );
    let merge_id = force_open_merge_state(temp.path(), OperationState::Halted);

    assert_open_merge_blocks_all_starts_without_mutation(temp.path(), &backend, &merge_id);
}

#[test]
fn open_recovery_required_blocks_dry_run_and_real_starts_from_an_explicit_root() {
    let temp = TempDir::new("merge-start-gate-recovery-required");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );
    let merge_id = force_open_merge_state(temp.path(), OperationState::RecoveryRequired);

    assert_open_merge_blocks_all_starts_without_mutation(temp.path(), &backend, &merge_id);
}

#[test]
fn first_class_merge_rejects_unrelated_history_without_mutation() {
    let temp = TempDir::new("merge-start-unrelated");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-unrelated-source");
    let member = temp.path().join("remote");
    create_orphan_ref(&member, "refs/heads/feature/source", "unrelated source\n");
    let head = backend.head(&member).unwrap();
    let target_ref = backend.read_ref(&member, "refs/heads/main").unwrap();
    let index = fs::read(member.join(".git/index")).unwrap();
    let worktree = fs::read(member.join("README.md")).unwrap();
    let status = backend.status(&member).unwrap();
    let native_state = backend.merge_state(&member).unwrap();
    let lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let error = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap_err();

    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert!(error.message.contains("do not share a merge base"));
    assert_eq!(backend.head(&member).unwrap(), head);
    assert_eq!(
        backend.read_ref(&member, "refs/heads/main").unwrap(),
        target_ref
    );
    assert_eq!(fs::read(member.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(member.join("README.md")).unwrap(), worktree);
    assert_eq!(backend.status(&member).unwrap(), status);
    assert_eq!(backend.merge_state(&member).unwrap(), native_state);
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(!member.join(".git/MERGE_HEAD").exists());
}

#[test]
fn preflight_checks_every_member_before_mutating_an_earlier_member() {
    let temp = TempDir::new("merge-start-preflight");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (app_base, _) = feature_commit(&backend, &app, "README.md", "source\n");
    feature_commit(&backend, &lib, "README.md", "source\n");
    fs::write(lib.join("README.md"), "dirty\n").unwrap();

    let error = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap_err();

    assert_eq!(error.code, ErrorCode::DirtyMember);
    assert_eq!(
        backend.head(&app).unwrap().commit.as_deref(),
        Some(app_base.as_str())
    );
    assert!(backend.merge_state(&app).unwrap().is_none());
}

#[test]
fn conflict_continues_to_later_member_and_status_recovers_with_baseline_lock() {
    let temp = TempDir::new("merge-start-conflict-batch");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (app_base, _) = feature_commit(&backend, &app, "README.md", "source\n");
    let app_local = commit_file(
        &app,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&app_base).unwrap()],
    )
    .unwrap();
    let (lib_base, lib_source) = feature_commit(&backend, &lib, "README.md", "source\n");

    let response = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Conflicted
    );
    assert_eq!(response.participant_counts.conflicted, 1);
    assert_eq!(response.participant_counts.fast_forwarded, 1);
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::Conflicted
    );
    assert_eq!(
        response.repos[1].state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(
        backend.head(&app).unwrap().commit.as_deref(),
        Some(app_local.as_str())
    );
    let merge_state = backend.merge_state(&app).unwrap().unwrap();
    assert_eq!(merge_state.conflict_paths, ["README.md"]);
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(lib_source.as_str())
    );
    let lock = read_lock(temp.path()).unwrap();
    assert_eq!(
        lock.members["mem_app"].commit.as_deref(),
        Some(app_base.as_str())
    );
    assert_eq!(
        lock.members["mem_lib"].commit.as_deref(),
        Some(lib_base.as_str())
    );

    let merge_id = response.merge_id.clone();
    let mut status_request = request(false);
    status_request.op = crate::MergeOp::Status;
    status_request.source_ref = None;
    let status = handle_merge(
        &crate::git::Git2Backend::new(),
        temp.path(),
        status_request.clone(),
        "op_status",
    )
    .unwrap();
    assert_eq!(status.merge_id, merge_id);
    assert_eq!(status.state, crate::MergeOperationState::AwaitingResolution);
    assert!(status.open);
    assert_eq!(status.repos[0].conflict_paths, ["README.md"]);
    assert_eq!(
        status.repos[1].live_commit.as_deref(),
        Some(lib_source.as_str())
    );

    let manifest_path = temp.path().join(crate::workspace::WORKSPACE_MANIFEST);
    fs::OpenOptions::new()
        .append(true)
        .open(&manifest_path)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    let drifted = handle_merge(&backend, temp.path(), status_request, "op_status_drift").unwrap();
    assert_eq!(
        drifted.operation_drift[0].kind,
        crate::MergeOperationDriftKind::BaselineManifestChanged
    );
}

#[test]
fn mixed_merge_continue_resolves_conflict_and_preserves_prior_result() {
    let temp = TempDir::new("merge-mixed-continue");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        merge_repo(&started, "mem_app").state,
        crate::MergeParticipantState::UpToDate
    );
    assert_eq!(
        merge_repo(&started, "mem_lib").state,
        crate::MergeParticipantState::Merged
    );
    assert_eq!(
        merge_repo(&started, "mem_docs").state,
        crate::MergeParticipantState::Conflicted
    );
    let lib_result = backend
        .head(&temp.path().join("lib"))
        .unwrap()
        .commit
        .unwrap();

    let docs = temp.path().join("docs");
    fs::write(docs.join("README.md"), "resolved\n").unwrap();
    backend
        .stage_paths_allowing_other_conflicts(&docs, &["README.md"])
        .unwrap();
    let continued = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, started.merge_id.clone()),
        "op_continue",
    )
    .unwrap();

    assert_eq!(continued.state, crate::MergeOperationState::Completed);
    assert!(!continued.open);
    assert_eq!(
        merge_repo(&continued, "mem_docs").state,
        crate::MergeParticipantState::Continued
    );
    assert_eq!(
        backend.head(&temp.path().join("lib")).unwrap().commit,
        Some(lib_result.clone())
    );
    let docs_result = git2::Oid::from_str(
        merge_repo(&continued, "mem_docs")
            .resulting_commit
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    let repo = git2::Repository::open(&docs).unwrap();
    let commit = repo.find_commit(docs_result).unwrap();
    assert_eq!(
        commit.parent_id(0).unwrap().to_string(),
        fixture.docs_before
    );
    assert_eq!(
        commit.parent_id(1).unwrap().to_string(),
        fixture.docs_source
    );
    assert_ne!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    let published = read_lock(temp.path()).unwrap();
    assert_eq!(
        published.members["mem_lib"].commit.as_deref(),
        Some(lib_result.as_str())
    );
    assert_eq!(
        published.members["mem_docs"].commit.as_deref(),
        merge_repo(&continued, "mem_docs")
            .resulting_commit
            .as_deref()
    );
}

#[test]
fn mixed_merge_abort_restores_exact_baseline_and_archives_operation() {
    let temp = TempDir::new("merge-mixed-abort");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
    let manifest_before = fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    let merge_id = started.merge_id.clone().unwrap();

    let aborted = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_abort",
    )
    .unwrap();

    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert!(!aborted.open);
    for (path, expected) in [
        ("app", fixture.app_before),
        ("lib", fixture.lib_before),
        ("docs", fixture.docs_before),
    ] {
        assert_eq!(
            backend.head(&temp.path().join(path)).unwrap().commit,
            Some(expected)
        );
        assert!(
            backend
                .merge_state(&temp.path().join(path))
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert_eq!(
        fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap(),
        manifest_before
    );
    assert!(
        !temp
            .path()
            .join(format!(".gwz/merge/{merge_id}.yaml"))
            .exists()
    );
    assert!(
        temp.path()
            .join(format!(".gwz/merge/done/{merge_id}.yaml"))
            .is_file()
    );
    let status = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_status",
    )
    .unwrap();
    assert_eq!(status.state, crate::MergeOperationState::Idle);
    assert!(!status.open);
}

#[test]
fn crash_reload_continue_foreign_rejection_and_external_restore_converge_on_abort() {
    let temp = TempDir::new("merge-adversarial-lifecycle");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    let merge_id = started.merge_id.clone().unwrap();
    assert_eq!(
        merge_repo(&started, "mem_lib").state,
        crate::MergeParticipantState::Merged
    );
    assert_eq!(
        merge_repo(&started, "mem_docs").state,
        crate::MergeParticipantState::Conflicted
    );

    // A new backend instance models a fresh process reloading only durable
    // operation state before the conflict is resolved.
    let reloaded = crate::git::Git2Backend::new();
    let status = handle_merge(
        &reloaded,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_status_reload",
    )
    .unwrap();
    assert_eq!(
        merge_repo(&status, "mem_docs").state,
        crate::MergeParticipantState::Conflicted
    );

    let docs = temp.path().join("docs");
    fs::write(docs.join("README.md"), "resolved after reload\n").unwrap();
    reloaded
        .stage_paths_allowing_other_conflicts(&docs, &["README.md"])
        .unwrap();
    let late_drift = temp.path().join("lib/late-finalization.txt");
    let injected_drift = late_drift.clone();
    crate::git::Git2Backend::before_next_scoped_commit_ref_lock(move || {
        fs::write(injected_drift, "late drift\n").unwrap();
    });
    let continued = handle_merge(
        &reloaded,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
        "op_continue_reload",
    )
    .unwrap();
    assert_eq!(continued.state, crate::MergeOperationState::Finalizing);
    assert!(continued.open);
    fs::remove_file(late_drift).unwrap();
    let lib = temp.path().join("lib");
    let lib_result = merge_repo(&continued, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    let docs_result = merge_repo(&continued, "mem_docs")
        .resulting_commit
        .clone()
        .unwrap();

    // Poison a participant that abort would have to roll back. Whole-operation
    // preflight must reject before changing the later docs participant.
    let lib_repo = git2::Repository::open(&lib).unwrap();
    let cherry_pick_head = lib_repo.path().join("CHERRY_PICK_HEAD");
    fs::write(&cherry_pick_head, format!("{lib_result}\n")).unwrap();
    let record_path = temp.path().join(format!(".gwz/merge/{merge_id}.yaml"));
    let record_before_rejection = fs::read(&record_path).unwrap();
    let error = handle_merge(
        &crate::git::Git2Backend::new(),
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_abort_foreign",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(
        reloaded.head(&lib).unwrap().commit.as_deref(),
        Some(lib_result.as_str())
    );
    assert_eq!(
        reloaded.head(&docs).unwrap().commit.as_deref(),
        Some(docs_result.as_str())
    );
    assert_eq!(fs::read(&record_path).unwrap(), record_before_rejection);
    fs::remove_file(cherry_pick_head).unwrap();

    // Simulate an exact external restoration after the interrupted process.
    // Coordinated abort must recognize it as a no-op and roll back only the
    // participant that remains changed.
    reloaded
        .set_branch_target_checked(&docs, "main", &docs_result, &fixture.docs_before)
        .unwrap();
    let aborted = handle_merge(
        &crate::git::Git2Backend::new(),
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_abort_reloaded",
    )
    .unwrap();
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert!(!aborted.open);
    assert_eq!(
        reloaded.head(&lib).unwrap().commit,
        Some(fixture.lib_before)
    );
    assert_eq!(
        reloaded.head(&docs).unwrap().commit,
        Some(fixture.docs_before)
    );
    assert!(
        temp.path()
            .join(format!(".gwz/merge/done/{merge_id}.yaml"))
            .is_file()
    );
}

#[test]
fn post_merge_commit_rejects_abort_before_conflicted_member_changes() {
    let temp = TempDir::new("merge-mixed-abort-drift");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    let lib = temp.path().join("lib");
    let lib_result = backend.head(&lib).unwrap().commit.unwrap();
    let post_merge = commit_file(
        &lib,
        "post-merge.txt",
        "later work\n",
        "later work",
        &[git2::Oid::from_str(&lib_result).unwrap()],
    )
    .unwrap();
    let docs = temp.path().join("docs");
    let docs_state = backend.merge_state(&docs).unwrap().unwrap();

    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        "op_abort",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(backend.head(&lib).unwrap().commit, Some(post_merge));
    assert_eq!(
        backend.head(&docs).unwrap().commit,
        Some(fixture.docs_before)
    );
    assert_eq!(backend.merge_state(&docs).unwrap(), Some(docs_state));
}

#[test]
fn failed_and_unattempted_rows_retry_only_after_whole_operation_preflight() {
    use crate::workspace_ops::merge::{
        FileMergeStore, MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeBaseline,
        MergeOperationRecord, MergeParticipantRecord, MergeStore, MergeTargetKind, OperationState,
        ParticipantState,
    };

    let temp = TempDir::new("merge-retry-recorded-rows");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (app_before, app_source) = feature_commit(&backend, &app, "source.txt", "app\n");
    let (lib_before, lib_source) = feature_commit(&backend, &lib, "source.txt", "lib\n");
    let participant = |path: &str, before: String, source: String, state| MergeParticipantRecord {
        path: path.to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: before,
        source_commit: source,
        commit_message: format!("Retry recorded merge for {path}"),
        state,
        resulting_commit: None,
        expected_merge_head: None,
        conflict_paths: Vec::new(),
        error: None,
        pending_action: None,
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: BTreeMap::new(),
    };
    let digest = |path| format!("{:x}", Sha256::digest(fs::read(path).unwrap()));
    let merge_id = "merge_retry_rows".to_owned();
    let record = MergeOperationRecord {
        schema: MERGE_RECORD_SCHEMA.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION,
        writer_version: crate::VERSION.to_owned(),
        workspace_id: "ws_ops".to_owned(),
        merge_id: merge_id.clone(),
        operation_id: "op_start".to_owned(),
        state: OperationState::Halted,
        source_ref: "feature/source".to_owned(),
        created_at: "now".to_owned(),
        baseline: MergeBaseline {
            lock_sha256: digest(temp.path().join(crate::artifact::LOCK_PATH)),
            manifest_sha256: digest(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)),
            root_head: None,
            root_branch: None,
            extensions: BTreeMap::new(),
        },
        selected_targets: vec!["mem_app".to_owned(), "mem_lib".to_owned()],
        participants: BTreeMap::from([
            (
                "mem_app".to_owned(),
                participant(
                    "app",
                    app_before.clone(),
                    app_source.clone(),
                    ParticipantState::Failed,
                ),
            ),
            (
                "mem_lib".to_owned(),
                participant(
                    "lib",
                    lib_before.clone(),
                    lib_source.clone(),
                    ParticipantState::Unattempted,
                ),
            ),
        ]),
        publication: None,
        operation_drift: Vec::new(),
        extensions: BTreeMap::new(),
    };
    FileMergeStore.write_open(temp.path(), &record).unwrap();

    fs::write(lib.join("untracked.txt"), "blocks whole preflight\n").unwrap();
    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
        "op_continue_blocked",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(backend.head(&app).unwrap().commit, Some(app_before));

    fs::remove_file(lib.join("untracked.txt")).unwrap();
    let response = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id)),
        "op_continue_retry",
    )
    .unwrap();
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        merge_repo(&response, "mem_app").state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(
        merge_repo(&response, "mem_lib").state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(backend.head(&app).unwrap().commit, Some(app_source));
    assert_eq!(backend.head(&lib).unwrap().commit, Some(lib_source));
}

#[test]
fn unrelated_staged_conflict_work_blocks_every_resolution_commit() {
    let temp = TempDir::new("merge-conflict-index-preflight");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let make_conflict = |repo: &std::path::Path| {
        let initial = backend.head(repo).unwrap().commit.unwrap();
        let stable = commit_file(
            repo,
            "stable.txt",
            "stable\n",
            "stable",
            &[git2::Oid::from_str(&initial).unwrap()],
        )
        .unwrap();
        let (base, _) = feature_commit(&backend, repo, "README.md", "source\n");
        assert_eq!(base, stable);
        commit_file(
            repo,
            "README.md",
            "local\n",
            "local",
            &[git2::Oid::from_str(&base).unwrap()],
        )
        .unwrap()
    };
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let app_before = make_conflict(&app);
    make_conflict(&lib);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(started.participant_counts.conflicted, 2);

    for repo in [&app, &lib] {
        fs::write(repo.join("README.md"), "resolved\n").unwrap();
        backend
            .stage_paths_allowing_other_conflicts(repo, &["README.md"])
            .unwrap();
    }
    fs::write(lib.join("stable.txt"), "unrelated staged work\n").unwrap();
    backend
        .stage_paths_allowing_other_conflicts(&lib, &["stable.txt"])
        .unwrap();

    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, started.merge_id),
        "op_continue",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(backend.head(&app).unwrap().commit, Some(app_before));
    assert!(backend.merge_state(&app).unwrap().is_some());
}

#[test]
fn direct_core_mutator_cannot_bypass_open_merge_gate() {
    let temp = TempDir::new("merge-direct-core-gate");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert!(started.open);

    let error = handle_branch(
        &backend,
        temp.path(),
        crate::BranchRequest {
            meta: request_meta(),
            op: crate::BranchOp::Create,
            name: Some("blocked-during-merge".to_owned()),
            start_ref: Some("HEAD".to_owned()),
            switch_after_create: None,
        },
        "op_direct_branch",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::OpenOperation);
    assert!(error.message.contains(started.merge_id.as_deref().unwrap()));
    assert!(
        !backend
            .branch_list(&temp.path().join("app"))
            .unwrap()
            .iter()
            .any(|branch| branch.name == "blocked-during-merge")
    );
}

#[test]
fn conditional_stage_allows_only_recorded_conflicted_participants() {
    let temp = TempDir::new("merge-stage-gate");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );

    let stage = |pathspec: &str, operation_id: &str| {
        handle_stage(
            &backend,
            temp.path(),
            crate::StageRequest {
                meta: request_meta(),
                cwd: temp.path().to_string_lossy().into_owned(),
                pathspecs: vec![pathspec.to_owned()],
                all: None,
            },
            operation_id,
        )
    };

    fs::write(temp.path().join("docs/README.md"), "resolved\n").unwrap();
    stage("docs/README.md", "op_stage_conflict").unwrap();
    assert_eq!(
        backend
            .status(&temp.path().join("docs"))
            .unwrap()
            .unresolved,
        0
    );
    let lib_staged = backend.status(&temp.path().join("lib")).unwrap().staged;
    let app_staged = backend.status(&temp.path().join("app")).unwrap().staged;
    let root_staged = backend.status(temp.path()).unwrap().staged;

    for (pathspec, operation_id) in [
        ("lib/new.txt", "op_stage_merged"),
        ("app/new.txt", "op_stage_unaffected"),
        ("root-new.txt", "op_stage_root"),
    ] {
        fs::write(temp.path().join(pathspec), "must remain unstaged\n").unwrap();
        let error = stage(pathspec, operation_id).unwrap_err();
        assert_eq!(error.code, ErrorCode::OpenOperation, "{pathspec}");
    }
    assert_eq!(
        backend.status(&temp.path().join("lib")).unwrap().staged,
        lib_staged
    );
    assert_eq!(
        backend.status(&temp.path().join("app")).unwrap().staged,
        app_staged
    );
    assert_eq!(backend.status(temp.path()).unwrap().staged, root_staged);
}
