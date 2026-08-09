use std::fs;

use super::*;
use crate::artifact::{LOCK_PATH, MarkerArtifact};
use crate::git::{Git2Backend, GitBackend};
use crate::operation::{ActionKind, OperationContext};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::model::v1::test_record;
use crate::workspace_ops::merge::{OperationState, ParticipantState};
use crate::workspace_ops::tests::{TempDir, commit_file};

#[test]
fn concrete_finalizer_freezes_acceptance_and_publishes_exact_candidate() {
    let (root, backend, model) = fixture("merge-v1-finalization-happy", true);
    seed_open(&root, &model);
    fs::write(root.path.join("user-staged.txt"), "preserve staged\n").unwrap();
    backend
        .stage_paths(&root.path, &["user-staged.txt"])
        .unwrap();
    fs::write(root.path.join("user-untracked.txt"), "preserve untracked\n").unwrap();
    let context = context();
    let mut runtime = RecordingRuntime::new(&backend, &context);

    let response = super::super::service::run(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap_or_else(|error| {
        panic!(
            "finalization failed: {error:?}; observations={:?}; actions={:?}",
            runtime.observations, runtime.actions
        )
    });

    assert_eq!(response.current().record().state, OperationState::Completed);
    let record = response.current().record();
    let accepted = record.accepted_workspace.as_ref().unwrap();
    let publication = record.publication.as_ref().unwrap();
    let candidate = publication.candidate.as_ref().unwrap();
    assert_eq!(
        fs::read_to_string(root.path.join(LOCK_PATH)).unwrap(),
        accepted.lock.exact_yaml
    );
    let marker = MarkerArtifact::from_yaml(
        &fs::read_to_string(
            root.path
                .join(publication.candidate_marker_path.as_ref().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(marker.gwz_commit_id, candidate.marker_id);
    assert_eq!(
        backend.head(&root.path).unwrap().commit.as_deref(),
        publication.composition_commit.as_deref()
    );
    assert!(
        backend
            .index_matches_candidate_files(
                &root.path,
                &crate::workspace_ops::merge::acceptance::v1_candidate_files(record).unwrap(),
                &[],
            )
            .unwrap()
    );
    let status = backend.status(&root.path).unwrap();
    assert!(status.files.iter().any(|file| {
        file.path == "user-staged.txt" && file.index_status == "A" && file.worktree_status == " "
    }));
    assert!(status.files.iter().any(|file| {
        file.path == "user-untracked.txt" && file.index_status == " " && file.worktree_status == "?"
    }));
    assert_eq!(
        runtime.actions,
        [
            PublicationPhysicalAction::EvidenceCommit,
            PublicationPhysicalAction::WriteMarker,
            PublicationPhysicalAction::WriteLock,
            PublicationPhysicalAction::WriteBoundary,
            PublicationPhysicalAction::StageIndex,
        ]
        .into_iter()
        .map(PhysicalActionKind::Publication)
        .collect::<Vec<_>>()
    );
}

#[test]
fn no_change_finalization_freezes_acceptance_without_physical_publication() {
    let (root, backend, model) = fixture("merge-v1-finalization-no-change", false);
    seed_open(&root, &model);
    let context = context();
    let mut runtime = RecordingRuntime::new(&backend, &context);

    let response = super::super::service::run(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();

    let record = response.current().record();
    assert_eq!(record.state, OperationState::Completed);
    assert!(record.accepted_workspace.is_some());
    let publication = record.publication.as_ref().unwrap();
    assert_eq!(
        publication.step,
        crate::workspace_ops::merge::PublicationStep::Complete
    );
    assert!(publication.candidate.is_none());
    assert!(runtime.actions.is_empty());
    assert_eq!(
        backend.head(&root.path).unwrap().commit,
        model.baseline.root_head
    );
}

#[test]
fn participant_drift_rejects_before_acceptance_or_root_mutation() {
    let (root, backend, model) = fixture("merge-v1-finalization-participant-drift", true);
    seed_open(&root, &model);
    fs::write(root.path.join("members/a/README.md"), "drift\n").unwrap();
    let context = context();
    let mut runtime = FinalizationRuntime::new(&backend, &context);

    let result = super::super::service::run(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut runtime,
    );
    let Err(error) = result else {
        panic!("participant drift unexpectedly finalized")
    };

    assert_eq!(error.code, crate::model::ErrorCode::MergeDrift);
    let current = super::super::store::CheckedV1Store::default()
        .load_open(&root.path, &model.merge_id)
        .unwrap();
    assert_eq!(current.record().state, OperationState::Executing);
    assert!(current.record().accepted_workspace.is_none());
    assert_eq!(
        backend.head(&root.path).unwrap().commit,
        model.baseline.root_head
    );
}

#[test]
fn restart_reconciles_every_owned_publication_mutation_prefix() {
    for action in [
        PublicationPhysicalAction::EvidenceCommit,
        PublicationPhysicalAction::WriteMarker,
        PublicationPhysicalAction::WriteLock,
        PublicationPhysicalAction::WriteBoundary,
        PublicationPhysicalAction::StageIndex,
    ] {
        restart_after(action);
    }
}

#[test]
fn tampered_owned_publication_prefix_enters_recovery_without_overwrite() {
    let (root, backend, model) = fixture("merge-v1-finalization-tampered-prefix", true);
    seed_open(&root, &model);
    let context = context();
    let mut crashing =
        CrashAfterRuntime::new(&backend, &context, PublicationPhysicalAction::WriteMarker);
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::service::run(
            &super::super::store::CheckedV1Store::default(),
            &root.path,
            &model.merge_id,
            super::super::authority::V1LifecycleRequest::Continue,
            &mut crashing,
        )
    }));
    assert!(crashed.is_err());

    let store = super::super::store::CheckedV1Store::default();
    let interrupted = store.load_open(&root.path, &model.merge_id).unwrap();
    let marker_path = interrupted
        .record()
        .publication
        .as_ref()
        .unwrap()
        .candidate_marker_path
        .as_ref()
        .unwrap()
        .clone();
    fs::write(root.path.join(&marker_path), "tampered: true\n").unwrap();
    let head_before = backend.head(&root.path).unwrap();
    let mut resumed = FinalizationRuntime::new(&backend, &context);
    let response = super::super::service::run(
        &store,
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut resumed,
    )
    .unwrap();

    assert_eq!(
        response.current().record().state,
        OperationState::RecoveryRequired
    );
    assert_eq!(
        fs::read_to_string(root.path.join(&marker_path)).unwrap(),
        "tampered: true\n"
    );
    assert_eq!(backend.head(&root.path).unwrap(), head_before);
}

fn restart_after(target: PublicationPhysicalAction) {
    let (root, backend, model) = fixture(&format!("merge-v1-finalization-{target:?}"), true);
    seed_open(&root, &model);
    let context = context();
    let mut crashing = CrashAfterRuntime::new(&backend, &context, target);

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::service::run(
            &super::super::store::CheckedV1Store::default(),
            &root.path,
            &model.merge_id,
            super::super::authority::V1LifecycleRequest::Continue,
            &mut crashing,
        )
    }));
    assert!(crashed.is_err(), "{target:?} fault was not reached");
    assert!(crashing.hit);

    let mut resumed = RecordingRuntime::new(&backend, &context);
    let response = super::super::service::run(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut resumed,
    )
    .unwrap_or_else(|error| panic!("restart after {target:?} failed: {error:?}"));
    assert_eq!(response.current().record().state, OperationState::Completed);
    let sequence = [
        PublicationPhysicalAction::EvidenceCommit,
        PublicationPhysicalAction::WriteMarker,
        PublicationPhysicalAction::WriteLock,
        PublicationPhysicalAction::WriteBoundary,
        PublicationPhysicalAction::StageIndex,
    ];
    let expected = sequence
        .iter()
        .skip_while(|action| **action != target)
        .skip(1)
        .copied()
        .map(PhysicalActionKind::Publication)
        .collect::<Vec<_>>();
    assert_eq!(resumed.actions, expected, "restart suffix after {target:?}");
}

pub(super) fn fixture(
    name: &str,
    changed: bool,
) -> (
    TempDir,
    Git2Backend,
    super::super::super::model::v1::MergeOperationRecordV1,
) {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
    fs::create_dir_all(root.path.join("gwz.conf")).unwrap();
    let mut model = test_record();
    let manifest = model.baseline.manifest_yaml.clone().unwrap();
    let lock = model.baseline.lock_yaml.clone().unwrap();
    let first = commit_file(
        &root.path,
        WORKSPACE_MANIFEST,
        &manifest,
        "workspace manifest",
        &[],
    )
    .unwrap();
    let first = git2::Oid::from_str(&first).unwrap();
    let root_commit =
        commit_file(&root.path, LOCK_PATH, &lock, "workspace lock", &[first]).unwrap();
    let member = root.path.join("members/a");
    backend.create_repo(&member).unwrap();
    let before = commit_file(&member, "README.md", "before\n", "before", &[]).unwrap();
    let before_oid = git2::Oid::from_str(&before).unwrap();
    let after = if changed {
        commit_file(&member, "README.md", "after\n", "after", &[before_oid]).unwrap()
    } else {
        before.clone()
    };
    let root_head = backend.head(&root.path).unwrap();
    model.baseline.root_head = Some(root_commit);
    model.baseline.root_branch = root_head.branch;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.before_commit = before;
    row.source_commit = after.clone();
    row.resulting_commit = Some(after);
    row.state = if changed {
        ParticipantState::FastForwarded
    } else {
        ParticipantState::UpToDate
    };
    (root, backend, model)
}

pub(super) fn seed_open(
    root: &TempDir,
    model: &super::super::super::model::v1::MergeOperationRecordV1,
) {
    let merge_root = root.path.join(".gwz/merge");
    fs::create_dir_all(&merge_root).unwrap();
    fs::write(
        merge_root.join(format!("{}.yaml", model.merge_id)),
        serde_yaml::to_string(model).unwrap(),
    )
    .unwrap();
}

pub(super) fn context() -> OperationContext {
    OperationContext {
        operation_id: "op_1".into(),
        request_id: "req_1".into(),
        schema_version: "gwz.protocol/v0".into(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}

pub(super) struct RecordingRuntime<'a> {
    inner: FinalizationRuntime<'a, Git2Backend>,
    observations: Vec<super::super::authority::ObservationKind>,
    pub(super) actions: Vec<PhysicalActionKind>,
}

impl<'a> RecordingRuntime<'a> {
    pub(super) fn new(backend: &'a Git2Backend, context: &'a OperationContext) -> Self {
        Self {
            inner: FinalizationRuntime::new(backend, context),
            observations: Vec::new(),
            actions: Vec::new(),
        }
    }
}

impl super::super::service::ExactObserver for RecordingRuntime<'_> {
    fn observe(
        &mut self,
        current: &super::super::checked::StoredV1Record,
        request: &BoundObservationRequest,
    ) -> crate::model::ModelResult<BoundExactObservation> {
        self.observations.push(request.kind().clone());
        self.inner.observe(current, request)
    }
}

impl super::super::service::PhysicalExecutor for RecordingRuntime<'_> {
    fn execute(
        &mut self,
        lease: &super::super::checked::V1MutationLease,
        current: &super::super::checked::StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.actions.push(action.clone());
        self.inner.execute(lease, current, action)
    }
}

pub(super) struct CrashAfterRuntime<'a> {
    inner: FinalizationRuntime<'a, Git2Backend>,
    target: PublicationPhysicalAction,
    hit: bool,
}

impl<'a> CrashAfterRuntime<'a> {
    pub(super) fn new(
        backend: &'a Git2Backend,
        context: &'a OperationContext,
        target: PublicationPhysicalAction,
    ) -> Self {
        Self {
            inner: FinalizationRuntime::new(backend, context),
            target,
            hit: false,
        }
    }
}

impl super::super::service::ExactObserver for CrashAfterRuntime<'_> {
    fn observe(
        &mut self,
        current: &super::super::checked::StoredV1Record,
        request: &BoundObservationRequest,
    ) -> crate::model::ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl super::super::service::PhysicalExecutor for CrashAfterRuntime<'_> {
    fn execute(
        &mut self,
        lease: &super::super::checked::V1MutationLease,
        current: &super::super::checked::StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        let diagnostic = self.inner.execute(lease, current, action);
        if action == &PhysicalActionKind::Publication(self.target)
            && diagnostic == ExecutionDiagnostic::Success
        {
            self.hit = true;
            panic!("injected crash after {:?}", self.target);
        }
        diagnostic
    }
}
