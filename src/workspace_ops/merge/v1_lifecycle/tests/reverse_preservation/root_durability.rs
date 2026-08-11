use super::*;
use crate::checked_artifact::{CheckedArtifactFault, fail_next_checked_artifact_at_for};
use crate::model::ModelResult;
use crate::workspace_ops::merge::model::v1::{
    PendingPreservationActionV1, PreservationStashPhaseV1 as S,
};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    V1LifecycleRequest, V1ResponseDisposition,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::{ExactObserver, PhysicalExecutor, run};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[test]
fn root_marker_consumer_rebarriers_visible_goals_before_advancing() {
    for fault in [
        CheckedArtifactFault::BeforeDurability,
        CheckedArtifactFault::AfterDurability,
    ] {
        let mut fixture =
            dirty_root_handoff_fixture(&format!("v1-root-marker-durability-{fault:?}"));
        install_baseline_pre_handoff(&mut fixture);
        exercise_fault_at(
            &fixture.base.root.path,
            &fixture.base.backend,
            &fixture.base.model,
            S::NormalizeMarker,
            fault,
        );
    }
}

fn install_baseline_pre_handoff(fixture: &mut RootPreservationFixture) {
    use crate::workspace_ops::merge::model::v1::{
        PreservationPublicationHandoffV1, PublicationIndexFormV1, PublicationPrefixV1,
    };

    let publication = fixture.base.model.publication.as_ref().unwrap();
    let marker_path = publication.candidate_marker_path.as_ref().unwrap().clone();
    let candidate = publication.candidate.as_ref().unwrap();
    let baseline_lock = candidate.baseline_lock_yaml.clone();
    let mut baseline_boundary = candidate.baseline_boundary_text.clone();
    let manifest = crate::artifact::ManifestArtifact::from_yaml(
        fixture
            .base
            .model
            .baseline
            .manifest_yaml
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    let mut required = vec![".gwz".to_owned(), "gwz.conf/.tmp".to_owned()];
    required.extend(
        manifest
            .members
            .into_iter()
            .filter(|member| member.active)
            .map(|member| member.path),
    );
    for path in required {
        let line = format!("/{}/", path.trim_matches('/'));
        if !baseline_boundary.lines().any(|actual| actual == line) {
            if !baseline_boundary.ends_with('\n') {
                baseline_boundary.push('\n');
            }
            baseline_boundary.push_str(&line);
            baseline_boundary.push('\n');
        }
    }
    let candidate = fixture
        .base
        .model
        .publication
        .as_mut()
        .unwrap()
        .candidate
        .as_mut()
        .unwrap();
    candidate.baseline_boundary_sha256 =
        format!("{:x}", sha2::Sha256::digest(baseline_boundary.as_bytes()));
    candidate.baseline_boundary_text = baseline_boundary.clone();
    let marker = fixture.base.root.path.join(&marker_path);
    std::fs::remove_file(&marker).unwrap();
    std::fs::write(
        fixture.base.root.path.join(crate::artifact::LOCK_PATH),
        baseline_lock,
    )
    .unwrap();
    fixture
        .base
        .backend
        .stage_paths(
            &fixture.base.root.path,
            &[marker_path.as_str(), crate::artifact::LOCK_PATH],
        )
        .unwrap();
    std::fs::remove_dir(marker.parent().unwrap()).unwrap();
    crate::workspace_ops::publish_workspace_exclude_candidate(
        &fixture.base.root.path,
        &baseline_boundary,
    )
    .unwrap();
    fixture.base.model.preservation_publication_handoff =
        Some(PreservationPublicationHandoffV1::Candidate {
            prefix: PublicationPrefixV1::Baseline,
            index: PublicationIndexFormV1::Pre,
        });
    let current = fixture.base.current();
    let plans = crate::workspace_ops::merge::preserve::v1_preservation_owners(
        &fixture.base.backend,
        &fixture.base.root.path,
        current.record(),
    )
    .unwrap();
    let plan = plans.last().unwrap();
    let spec = crate::workspace_ops::merge::preserve::v1_root_preservation_spec(
        &fixture.base.backend,
        current.record(),
        plan,
        &plan.protected_commit,
    )
    .unwrap()
    .unwrap();
    fixture
        .base
        .backend
        .prepare_root_preservation_stash(&fixture.base.root.path, &spec)
        .unwrap();
    assert_eq!(
        fixture
            .base
            .backend
            .observe_root_preservation_step(
                &fixture.base.root.path,
                &spec,
                &crate::workspace_ops::merge::v1_lifecycle::authority::preservation_stash_step(
                    S::NormalizeParent,
                    &fixture.base.model.merge_id,
                )
                .unwrap(),
                &crate::workspace_ops::merge::v1_lifecycle::authority::preservation_stash_guard(
                    S::NormalizeParent,
                    &fixture
                        .base
                        .backend
                        .prepare_root_preservation_stash(&fixture.base.root.path, &spec)
                        .unwrap()
                        .normalized_image
                        .preimage_sha256,
                ),
            )
            .unwrap(),
        crate::git::GitRootPreservationStepObservation::Before,
    );
}

#[test]
fn bundle_consumer_rebarriers_visible_goals_before_advancing() {
    for owner in [RootOwner::Publication, RootOwner::Selected] {
        for fault in [
            CheckedArtifactFault::BeforeDurability,
            CheckedArtifactFault::AfterDurability,
        ] {
            let fixture = root_fixture(
                owner,
                &format!("v1-root-bundle-durability-{owner:?}-{fault:?}"),
            );
            exercise_fault_at(
                &fixture.base.root.path,
                &fixture.base.backend,
                &fixture.base.model,
                S::WriteBundle,
                fault,
            );
            let stash_id = format!("stash_{}", fixture.base.model.merge_id);
            let path = crate::stash::bundle_path(&fixture.base.root.path, &stash_id);
            let bytes = std::fs::read_to_string(path).unwrap();
            assert_eq!(
                bytes,
                crate::stash::StashBundle::from_yaml(&bytes)
                    .unwrap()
                    .to_yaml()
                    .unwrap()
            );
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum RootOwner {
    Publication,
    Selected,
}

fn root_fixture(owner: RootOwner, name: &str) -> RootPreservationFixture {
    match owner {
        RootOwner::Publication => dirty_root_handoff_fixture(name),
        RootOwner::Selected => dirty_selected_root_handoff_fixture(name),
    }
}

fn exercise_fault_at(
    root: &std::path::Path,
    backend: &Git2Backend,
    model: &MergeOperationRecordV1,
    phase: S,
    fault: CheckedArtifactFault,
) {
    seed_open(root, model);
    let store = CheckedV1Store::default();
    let operation_context = context(model);
    let mut interrupted = DurabilityRuntime {
        inner: ReverseRuntime::new(backend, &operation_context),
        phase,
        fault,
        injected: false,
        awaiting_reobservation: false,
        reobserved: false,
        actions: Vec::new(),
    };
    let error = match run(
        &store,
        root,
        &model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut interrupted,
    ) {
        Ok(_) => panic!("a failed causal durability barrier must retain its owner"),
        Err(error) => error,
    };
    assert!(
        error.message.contains("injected failure"),
        "{phase:?} {fault:?}: {error:?}"
    );
    assert!(interrupted.injected, "{phase:?} {fault:?} was not reached");
    assert!(
        interrupted.reobserved && !interrupted.awaiting_reobservation,
        "{phase:?} {fault:?} advanced without reobserving the visible goal"
    );
    let resume_context = context(model);
    let mut resume = ReverseRuntime::new(backend, &resume_context);
    let mut response = run(
        &store,
        root,
        &model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut resume,
    )
    .unwrap_or_else(|error| {
        panic!(
            "{phase:?} {fault:?} fresh-invocation retry failed: {error:?}; status={:?}; head={:?}; {:?}",
            backend.status(root),
            backend.head(root),
            interrupted.actions
        )
    });
    for _ in 0..8 {
        if response.disposition()
            != V1ResponseDisposition::Stopped(OperationState::RecoveryRequired)
        {
            break;
        }
        let paused = (
            response.current().record().state,
            response.current().record().pending_rollback.clone(),
            response.current().record().recovery_context.clone(),
        );
        let resume_context = context(model);
        let mut resume = ReverseRuntime::new(backend, &resume_context);
        response = run(
            &store,
            root,
            &model.merge_id,
            V1LifecycleRequest::Abort,
            &mut resume,
        )
        .unwrap_or_else(|error| {
            panic!(
                "{phase:?} {fault:?} recovery failed from {paused:?}: {error:?}; {:?}",
                interrupted.actions
            )
        });
    }
    assert_eq!(
        response.disposition(),
        V1ResponseDisposition::Terminal(OperationState::Aborted),
        "{phase:?} {fault:?}"
    );
    assert!(response.current().record().pending_preservation.is_none());
}

fn seed_open(root: &std::path::Path, model: &MergeOperationRecordV1) {
    let merge_root = root.join(".gwz/merge");
    std::fs::create_dir_all(&merge_root).unwrap();
    std::fs::write(
        merge_root.join(format!("{}.yaml", model.merge_id)),
        serde_yaml::to_string(model).unwrap(),
    )
    .unwrap();
}

fn context(model: &MergeOperationRecordV1) -> crate::operation::OperationContext {
    crate::operation::OperationContext {
        operation_id: model.operation_id.clone(),
        request_id: format!("req_{}", model.merge_id),
        schema_version: "gwz.protocol/v0".into(),
        action: crate::operation::ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}

struct DurabilityRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    phase: S,
    fault: CheckedArtifactFault,
    injected: bool,
    awaiting_reobservation: bool,
    reobserved: bool,
    actions: Vec<(PhysicalActionKind, ExecutionDiagnostic)>,
}

impl ExactObserver for DurabilityRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        if self.awaiting_reobservation {
            assert!(
                matches!(
                    current.record().pending_preservation.as_ref(),
                    Some(PendingPreservationActionV1::Stash { phase, .. })
                        if *phase == self.phase
                ),
                "durability fault advanced before exact reobservation"
            );
            self.awaiting_reobservation = false;
            self.reobserved = true;
        }
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for DurabilityRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        assert!(
            !self.awaiting_reobservation,
            "physical execution repeated before exact reobservation"
        );
        if !self.injected
            && matches!(
                action,
                PhysicalActionKind::Preservation(PendingPreservationActionV1::Stash {
                    phase,
                    ..
                }) if *phase == self.phase
            )
        {
            fail_next_checked_artifact_at_for(target_label(self.phase), self.fault);
        }
        let diagnostic = self.inner.execute(lease, current, action);
        if matches!(
            &diagnostic,
            ExecutionDiagnostic::Failed { message, .. }
                if message.contains("injected failure")
        ) {
            self.injected = true;
            self.awaiting_reobservation = true;
        }
        self.actions.push((action.clone(), diagnostic.clone()));
        diagnostic
    }
}

fn target_label(phase: S) -> &'static str {
    match phase {
        S::NormalizeMarker => "root preservation artifact",
        S::WriteBundle => "preservation bundle",
        _ => panic!("durability fixture targets an unsupported checked artifact"),
    }
}
