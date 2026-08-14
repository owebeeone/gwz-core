use std::fs;

use super::*;
use crate::artifact::LOCK_PATH;
use crate::git::{GitBackend, GitHeadState};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::OperationState;
use crate::workspace_ops::merge::model::v1::AcceptedRootBaseV1;

use super::tests::{CrashAfterRuntime, context, fixture, seed_open};

#[test]
fn detached_unchanged_root_completes_without_publication_authority() {
    let (root, backend, mut model) = fixture("merge-v1-finalization-detached-no-change", false);
    let root_commit = model.baseline.root_head.clone().unwrap();
    backend.checkout_commit(&root.path, &root_commit).unwrap();
    model.baseline.root_branch = None;
    seed_open(&root, &model);
    let context = context();
    let mut runtime = RecordingRuntime::new(&backend, &context);

    let response = super::super::service::run_test(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();

    let record = response.current().record();
    assert_eq!(record.state, OperationState::Completed);
    assert!(matches!(
        record.accepted_workspace.as_ref().unwrap().root.base,
        AcceptedRootBaseV1::BornDetached { ref commit } if commit == &root_commit
    ));
    assert!(record.publication.as_ref().unwrap().candidate.is_none());
    assert!(runtime.actions.is_empty());
    assert_eq!(
        backend.head(&root.path).unwrap(),
        GitHeadState {
            commit: Some(root_commit),
            branch: None,
            is_detached: true,
        }
    );
}

#[test]
fn restart_after_acceptance_rejects_every_frozen_metadata_tamper() {
    for tamper in [
        "manifest_worktree",
        "lock_worktree",
        "manifest_index",
        "lock_index",
        "manifest_flags",
        "lock_flags",
    ] {
        accepted_metadata_tamper(tamper);
    }
}

#[test]
fn pre_acceptance_noncanonical_index_flags_do_not_receive_authority() {
    for path in [WORKSPACE_MANIFEST, LOCK_PATH] {
        let (root, backend, model) = fixture(
            &format!(
                "merge-v1-finalization-pre-acceptance-flags-{}",
                path.replace('/', "-")
            ),
            true,
        );
        seed_open(&root, &model);
        set_assume_valid(&root.path, path);
        let head_before = backend.head(&root.path).unwrap();
        let context = context();
        let mut runtime = FinalizationRuntime::new(&backend, &context);

        let result = super::super::service::run_test(
            &super::super::store::CheckedV1Store::default(),
            &root.path,
            &model.merge_id,
            super::super::authority::V1LifecycleRequest::Continue,
            &mut runtime,
        );
        let Err(error) = result else {
            panic!("noncanonical index entry was accepted for {path}")
        };

        assert_eq!(error.code, crate::model::ErrorCode::MergeDrift, "{path}");
        let current = super::super::store::CheckedV1Store::default()
            .load_open(&root.path, &model.merge_id)
            .unwrap();
        assert!(current.record().accepted_workspace.is_none(), "{path}");
        assert!(current.record().publication.is_none(), "{path}");
        assert_eq!(backend.head(&root.path).unwrap(), head_before, "{path}");
    }
}

#[test]
fn post_evidence_manifest_tamper_enters_finalizing_recovery() {
    let (root, backend, model) = fixture("merge-v1-finalization-evidence-manifest-tamper", true);
    seed_open(&root, &model);
    let context = context();
    let mut crashing = CrashAfterRuntime::new(
        &backend,
        &context,
        PublicationPhysicalAction::EvidenceCommit,
    );
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::service::run_test(
            &super::super::store::CheckedV1Store::default(),
            &root.path,
            &model.merge_id,
            super::super::authority::V1LifecycleRequest::Continue,
            &mut crashing,
        )
    }));
    assert!(crashed.is_err());

    fs::write(root.path.join(WORKSPACE_MANIFEST), "tampered: true\n").unwrap();
    let head_before = backend.head(&root.path).unwrap();
    let mut resumed = FinalizationRuntime::new(&backend, &context);
    let response = super::super::service::run_test(
        &super::super::store::CheckedV1Store::default(),
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
        response
            .current()
            .record()
            .recovery_context
            .as_ref()
            .unwrap()
            .origin_state,
        crate::workspace_ops::merge::model::v1::RecoveryOriginStateV1::Finalizing
    );
    assert_eq!(backend.head(&root.path).unwrap(), head_before);
    assert_eq!(
        fs::read_to_string(root.path.join(WORKSPACE_MANIFEST)).unwrap(),
        "tampered: true\n"
    );
}

#[test]
fn mixed_publication_index_enters_recovery_without_another_write() {
    let (root, backend, model) = fixture("merge-v1-finalization-mixed-index", true);
    seed_open(&root, &model);
    let context = context();
    let mut crashing =
        CrashAfterRuntime::new(&backend, &context, PublicationPhysicalAction::WriteMarker);
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::service::run_test(
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
    let progress = interrupted.record().publication.as_ref().unwrap();
    let marker_path = progress.candidate_marker_path.as_ref().unwrap().clone();
    let marker_bytes = progress.candidate.as_ref().unwrap().marker_yaml.clone();
    let lock_bytes = fs::read_to_string(root.path.join(LOCK_PATH)).unwrap();
    backend.stage_paths(&root.path, &[&marker_path]).unwrap();

    let mut resumed = RecordingRuntime::new(&backend, &context);
    let response = super::super::service::run_test(
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
    assert!(resumed.actions.is_empty());
    assert_eq!(
        fs::read_to_string(root.path.join(&marker_path)).unwrap(),
        marker_bytes
    );
    assert_eq!(
        fs::read_to_string(root.path.join(LOCK_PATH)).unwrap(),
        lock_bytes
    );
}

#[test]
fn staged_candidate_worktree_tamper_enters_recovery_without_overwrite() {
    let (root, backend, model) = fixture("merge-v1-finalization-staged-tamper", true);
    seed_open(&root, &model);
    let context = context();
    let mut crashing =
        CrashAfterRuntime::new(&backend, &context, PublicationPhysicalAction::StageIndex);
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::service::run_test(
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
    let progress = interrupted.record().publication.as_ref().unwrap();
    let marker_path = progress.candidate_marker_path.as_ref().unwrap().clone();
    let candidate_files =
        crate::workspace_ops::merge::acceptance::v1_candidate_files(interrupted.record()).unwrap();
    assert!(
        backend
            .index_entries_match_candidate_files(&root.path, &candidate_files, &[])
            .unwrap()
    );
    fs::write(root.path.join(&marker_path), "tampered: true\n").unwrap();

    let mut resumed = RecordingRuntime::new(&backend, &context);
    let response = super::super::service::run_test(
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
    assert!(resumed.actions.is_empty());
    assert_eq!(
        fs::read_to_string(root.path.join(&marker_path)).unwrap(),
        "tampered: true\n"
    );
    assert!(
        backend
            .index_entries_match_candidate_files(&root.path, &candidate_files, &[])
            .unwrap()
    );
}

fn accepted_metadata_tamper(tamper: &str) {
    let (root, backend, model) = fixture(&format!("merge-v1-finalization-accepted-{tamper}"), true);
    seed_open(&root, &model);
    let context = context();
    let mut crashing = CrashAfterAcceptanceRuntime::new(&backend, &context);
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::service::run_test(
            &super::super::store::CheckedV1Store::default(),
            &root.path,
            &model.merge_id,
            super::super::authority::V1LifecycleRequest::Continue,
            &mut crashing,
        )
    }));
    assert!(
        crashed.is_err(),
        "acceptance crash not reached for {tamper}"
    );
    assert!(crashing.hit);

    let store = super::super::store::CheckedV1Store::default();
    let accepted = store.load_open(&root.path, &model.merge_id).unwrap();
    assert!(accepted.record().accepted_workspace.is_some());
    assert!(accepted.record().publication.is_none());
    let frozen_lock = accepted
        .record()
        .accepted_workspace
        .as_ref()
        .unwrap()
        .metadata_base
        .lock_exact_yaml
        .clone();
    let frozen_manifest = accepted
        .record()
        .accepted_workspace
        .as_ref()
        .unwrap()
        .metadata_base
        .manifest_exact_yaml
        .clone();
    match tamper {
        "manifest_worktree" => {
            fs::write(root.path.join(WORKSPACE_MANIFEST), "tampered: true\n").unwrap();
        }
        "lock_worktree" => {
            fs::write(root.path.join(LOCK_PATH), "tampered: true\n").unwrap();
        }
        "manifest_index" => {
            tamper_index(&backend, &root.path, WORKSPACE_MANIFEST, &frozen_manifest)
        }
        "lock_index" => tamper_index(&backend, &root.path, LOCK_PATH, &frozen_lock),
        "manifest_flags" => set_assume_valid(&root.path, WORKSPACE_MANIFEST),
        "lock_flags" => set_assume_valid(&root.path, LOCK_PATH),
        _ => unreachable!(),
    }
    let head_before = backend.head(&root.path).unwrap();
    let mut resumed = FinalizationRuntime::new(&backend, &context);
    let result = super::super::service::run_test(
        &store,
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut resumed,
    );
    let Err(error) = result else {
        panic!("accepted metadata tamper unexpectedly finalized: {tamper}")
    };

    assert_eq!(error.code, crate::model::ErrorCode::MergeDrift, "{tamper}");
    let unchanged = store.load_open(&root.path, &model.merge_id).unwrap();
    assert!(unchanged.record().publication.is_none(), "{tamper}");
    assert_eq!(backend.head(&root.path).unwrap(), head_before, "{tamper}");
}

fn tamper_index(
    backend: &crate::git::Git2Backend,
    root: &std::path::Path,
    path: &str,
    frozen: &str,
) {
    fs::write(root.join(path), "tampered: true\n").unwrap();
    backend.stage_paths(root, &[path]).unwrap();
    fs::write(root.join(path), frozen).unwrap();
}

fn set_assume_valid(root: &std::path::Path, path: &str) {
    let repo = git2::Repository::open(root).unwrap();
    let mut index = repo.index().unwrap();
    let mut entry = index.get_path(std::path::Path::new(path), 0).unwrap();
    entry.flags |= 0x8000;
    index.add(&entry).unwrap();
    index.write().unwrap();
}

struct RecordingRuntime<'a> {
    inner: FinalizationRuntime<'a, crate::git::Git2Backend>,
    actions: Vec<PhysicalActionKind>,
}

impl<'a> RecordingRuntime<'a> {
    fn new(
        backend: &'a crate::git::Git2Backend,
        context: &'a crate::operation::OperationContext,
    ) -> Self {
        Self {
            inner: FinalizationRuntime::new(backend, context),
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

struct CrashAfterAcceptanceRuntime<'a> {
    inner: FinalizationRuntime<'a, crate::git::Git2Backend>,
    hit: bool,
}

impl<'a> CrashAfterAcceptanceRuntime<'a> {
    fn new(
        backend: &'a crate::git::Git2Backend,
        context: &'a crate::operation::OperationContext,
    ) -> Self {
        Self {
            inner: FinalizationRuntime::new(backend, context),
            hit: false,
        }
    }
}

impl super::super::service::ExactObserver for CrashAfterAcceptanceRuntime<'_> {
    fn observe(
        &mut self,
        current: &super::super::checked::StoredV1Record,
        request: &BoundObservationRequest,
    ) -> crate::model::ModelResult<BoundExactObservation> {
        if !self.hit
            && request.kind() == &super::super::authority::ObservationKind::Publication
            && current.record().accepted_workspace.is_some()
            && current.record().publication.is_none()
        {
            self.hit = true;
            panic!("injected crash after acceptance");
        }
        self.inner.observe(current, request)
    }
}

impl super::super::service::PhysicalExecutor for CrashAfterAcceptanceRuntime<'_> {
    fn execute(
        &mut self,
        lease: &super::super::checked::V1MutationLease,
        current: &super::super::checked::StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.inner.execute(lease, current, action)
    }
}
