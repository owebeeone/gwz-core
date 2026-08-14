use super::*;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::stash::StashBundle;
use crate::workspace_ops::merge::model::v1::{
    PendingPreservationActionV1, PreservationRefResetPhaseV1 as R, PreservationStashPhaseV1 as S,
};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    V1LifecycleRequest, V1ResponseDisposition,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::{
    ExactObserver, PhysicalExecutor, run_test as run,
};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[test]
fn full_service_preserves_dirty_work_then_rolls_back_the_integrated_ref() {
    let fixture = dirty_integrated_fixture("v1-preservation-real-git-service");
    fixture.seed_open();
    let context = fixture.context();
    let mut runtime = RecordingRuntime {
        inner: ReverseRuntime::new(&fixture.backend, &context),
        preservation_actions: Vec::new(),
        saw_backup: false,
        stash_phases: Vec::new(),
        reset_phases: Vec::new(),
    };

    let response = run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    )
    .unwrap();

    assert_eq!(
        response.disposition(),
        V1ResponseDisposition::Terminal(OperationState::Aborted),
        "stash={:?} reset={:?} pending={:?} recovery={:?}",
        runtime.stash_phases,
        runtime.reset_phases,
        response.current().record().pending_preservation,
        response.current().record().recovery_context,
    );
    assert_eq!(
        fixture
            .backend
            .head(&fixture.member)
            .unwrap()
            .commit
            .as_deref(),
        Some(fixture.before.as_str())
    );
    let record = response.current().record();
    let evidence = &record.participants["mem_a"].preservation;
    assert_eq!(evidence.len(), 1);
    assert_eq!(
        evidence[0].backup_commit.as_deref(),
        Some(fixture.protected.as_str())
    );
    assert!(evidence[0].stash_object_id.is_some());

    let stash = fixture
        .backend
        .preservation_stashes(&fixture.member, &fixture.model.merge_id)
        .unwrap()
        .pop()
        .unwrap();
    assert!(stash.image.dirty.staged);
    assert!(stash.image.dirty.unstaged);
    assert!(stash.image.dirty.untracked);

    let bundle_path = crate::stash::bundle_path(
        &fixture.root.path,
        &format!("stash_{}", fixture.model.merge_id),
    );
    let bundle = StashBundle::from_yaml(&fs::read_to_string(bundle_path).unwrap()).unwrap();
    assert_eq!(bundle.selected_members, vec!["mem_a"]);
    assert_eq!(
        bundle.members[0].native_stash_object_id,
        Some(stash.object_id)
    );
    assert!(bundle.members[0].dirty_summary.staged);
    assert!(bundle.members[0].dirty_summary.unstaged);
    assert!(bundle.members[0].dirty_summary.untracked);

    let backup = format!("refs/gwz/merge/{}/mem_a/head", fixture.model.merge_id);
    assert_eq!(
        fixture.backend.read_ref(&fixture.member, &backup).unwrap(),
        Some(fixture.protected.clone())
    );
    assert!(matches!(
        runtime.preservation_actions.as_slice(),
        [
            PendingPreservationActionV1::BackupRef { .. },
            PendingPreservationActionV1::Stash {
                phase: S::CreateStash,
                ..
            },
            PendingPreservationActionV1::Stash {
                phase: S::WriteBundle,
                ..
            },
            PendingPreservationActionV1::ResetAttachedRef {
                phase: R::ResetRef,
                ..
            }
        ]
    ));
}

#[test]
fn publication_root_visits_the_complete_handoff_state_machine() {
    let fixture = dirty_root_handoff_fixture("v1-preservation-root-handoff");
    let current = fixture.base.current();
    let plans = crate::workspace_ops::merge::preserve::v1_preservation_owners(
        &fixture.base.backend,
        &fixture.base.root.path,
        current.record(),
    )
    .unwrap();
    let root_plan = plans.last().unwrap();
    let spec = crate::workspace_ops::merge::preserve::v1_root_preservation_spec(
        &fixture.base.backend,
        current.record(),
        root_plan,
        &root_plan.protected_commit,
    )
    .unwrap()
    .unwrap();
    fixture
        .base
        .backend
        .prepare_root_preservation_stash(&fixture.base.root.path, &spec)
        .unwrap();
    fixture.base.seed_open();
    let context = fixture.base.context();
    let mut runtime = RecordingRuntime {
        inner: ReverseRuntime::new(&fixture.base.backend, &context),
        preservation_actions: Vec::new(),
        saw_backup: false,
        stash_phases: Vec::new(),
        reset_phases: Vec::new(),
    };

    let response = run(
        &CheckedV1Store::default(),
        &fixture.base.root.path,
        &fixture.base.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    )
    .unwrap();

    assert_eq!(
        response.disposition(),
        V1ResponseDisposition::Terminal(OperationState::Aborted),
        "stash={:?} reset={:?} pending={:?} recovery={:?}",
        runtime.stash_phases,
        runtime.reset_phases,
        response.current().record().pending_preservation,
        response.current().record().recovery_context,
    );
    assert!(runtime.saw_backup);
    assert_eq!(
        runtime.stash_phases,
        vec![
            S::NormalizeParent,
            S::NormalizeMarker,
            S::NormalizeLock,
            S::NormalizeIndex,
            S::CreateStash,
            S::RestoreIndex,
            S::RestoreLock,
            S::RestoreParent,
            S::RestoreMarker,
            S::WriteBundle,
            S::Complete,
        ]
    );
    assert_eq!(
        runtime.reset_phases,
        vec![
            R::PrepareParent,
            R::PrepareMarker,
            R::PrepareLock,
            R::PrepareIndex,
            R::ResetRef,
            R::RestoreIndex,
            R::RestoreLock,
            R::RestoreParent,
            R::RestoreMarker,
            R::Complete,
        ]
    );
    let publication = response.current().record().publication.as_ref().unwrap();
    assert_eq!(publication.root_preservation.len(), 1);
    assert_eq!(
        publication.root_preservation[0].backup_commit.as_deref(),
        Some(fixture.protected.as_str())
    );
    assert!(publication.root_preservation[0].stash_object_id.is_some());
    assert_eq!(
        fixture
            .base
            .backend
            .head(&fixture.base.root.path)
            .unwrap()
            .commit
            .as_deref(),
        fixture.base.model.baseline.root_head.as_deref()
    );
    assert_eq!(
        fixture
            .base
            .backend
            .preservation_stashes(&fixture.base.root.path, &fixture.base.model.merge_id,)
            .unwrap()
            .len(),
        1
    );
    assert_ne!(fixture.anchor, fixture.protected);
}

#[test]
fn selected_root_owns_the_same_complete_handoff_state_machine_without_collision() {
    let fixture = dirty_selected_root_handoff_fixture("v1-preservation-selected-root-handoff");
    fixture.base.seed_open();
    let context = fixture.base.context();
    let mut runtime = SelectedRootRecordingRuntime {
        inner: ReverseRuntime::new(&fixture.base.backend, &context),
        backend: &fixture.base.backend,
        root: &fixture.base.root.path,
        anchor: &fixture.anchor,
        saw_backup: false,
        stash_phases: Vec::new(),
        reset_phases: Vec::new(),
    };

    let error = match run(
        &CheckedV1Store::default(),
        &fixture.base.root.path,
        &fixture.base.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ) {
        Ok(_) => panic!("selected-root preservation unexpectedly crossed into rollback"),
        Err(error) => error,
    };

    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert!(runtime.saw_backup);
    assert_eq!(
        runtime.stash_phases,
        vec![
            S::NormalizeParent,
            S::NormalizeMarker,
            S::NormalizeLock,
            S::NormalizeIndex,
            S::CreateStash,
            S::RestoreIndex,
            S::RestoreLock,
            S::RestoreParent,
            S::RestoreMarker,
            S::WriteBundle,
            S::Complete,
        ]
    );
    assert_eq!(
        runtime.reset_phases,
        vec![
            R::PrepareParent,
            R::PrepareMarker,
            R::PrepareLock,
            R::PrepareIndex,
            R::ResetRef,
            R::RestoreIndex,
            R::RestoreLock,
            R::RestoreParent,
            R::RestoreMarker,
            R::Complete,
        ]
    );
    let current = CheckedV1Store::default()
        .load_open(&fixture.base.root.path, &fixture.base.model.merge_id)
        .unwrap();
    let record = current.record();
    assert_eq!(record.state, OperationState::Preserving);
    assert!(record.pending_preservation.is_none());
    assert_eq!(record.participants["@root"].preservation.len(), 1);
    assert!(
        record
            .publication
            .as_ref()
            .unwrap()
            .root_preservation
            .is_empty()
    );
    assert_eq!(
        fixture
            .base
            .backend
            .preservation_stashes(&fixture.base.root.path, &fixture.base.model.merge_id)
            .unwrap()
            .len(),
        1
    );
}

struct SelectedRootRecordingRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    backend: &'a Git2Backend,
    root: &'a std::path::Path,
    anchor: &'a str,
    saw_backup: bool,
    stash_phases: Vec<S>,
    reset_phases: Vec<R>,
}

impl ExactObserver for SelectedRootRecordingRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let preservation_complete = current.record().state == OperationState::Preserving
            && current.record().pending_preservation.is_none()
            && current.record().participants["@root"]
                .preservation
                .first()
                .is_some_and(|row| row.stash_object_id.is_some())
            && self.backend.head(self.root)?.commit.as_deref() == Some(self.anchor);
        if preservation_complete {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "test stop after selected-root preservation exhaustion",
            ));
        }
        match current.record().pending_preservation.as_ref() {
            Some(PendingPreservationActionV1::BackupRef { .. }) => self.saw_backup = true,
            Some(PendingPreservationActionV1::Stash { phase, .. })
                if self.stash_phases.last() != Some(phase) =>
            {
                self.stash_phases.push(*phase);
            }
            Some(PendingPreservationActionV1::ResetAttachedRef { phase, .. })
                if self.reset_phases.last() != Some(phase) =>
            {
                self.reset_phases.push(*phase);
            }
            Some(_) => {}
            None => {}
        }
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for SelectedRootRecordingRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.inner.execute(lease, current, action)
    }
}

struct RecordingRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    preservation_actions: Vec<PendingPreservationActionV1>,
    saw_backup: bool,
    stash_phases: Vec<S>,
    reset_phases: Vec<R>,
}

impl ExactObserver for RecordingRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        match current.record().pending_preservation.as_ref() {
            Some(PendingPreservationActionV1::BackupRef { .. }) => {
                self.saw_backup = true;
            }
            Some(PendingPreservationActionV1::Stash { phase, .. })
                if self.stash_phases.last() != Some(phase) =>
            {
                self.stash_phases.push(*phase);
            }
            Some(PendingPreservationActionV1::ResetAttachedRef { phase, .. })
                if self.reset_phases.last() != Some(phase) =>
            {
                self.reset_phases.push(*phase);
            }
            Some(_) => {}
            None => {}
        }
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for RecordingRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if let PhysicalActionKind::Preservation(action) = action {
            self.preservation_actions.push(action.clone());
        }
        self.inner.execute(lease, current, action)
    }
}
