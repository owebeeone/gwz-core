use super::*;
use crate::git::GitRootManagedIndexFact;
use crate::model::{ErrorCode, ModelResult};
use crate::workspace_ops::merge::model::v1::{
    PendingPreservationActionV1, PreservationOwnerV1, PreservationPublicationHandoffV1,
    PublicationIndexFormV1 as I, PublicationPrefixV1 as P,
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
fn clean_owner_at_anchor_creates_no_fake_preservation_artifact() {
    let fixture = anchor_fixture("v1-preservation-clean-anchor");
    fixture.seed_open();
    let context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);

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
        V1ResponseDisposition::Terminal(OperationState::Aborted)
    );
    assert!(
        response.current().record().participants["mem_a"]
            .preservation
            .is_empty()
    );
    assert!(
        fixture
            .backend
            .preservation_stashes(&fixture.member, &fixture.model.merge_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .backend
            .read_ref(
                &fixture.member,
                &format!("refs/gwz/merge/{}/mem_a/head", fixture.model.merge_id),
            )
            .unwrap()
            .is_none()
    );
    assert!(
        !crate::stash::bundle_path(
            &fixture.root.path,
            &format!("stash_{}", fixture.model.merge_id),
        )
        .exists()
    );
}

#[test]
fn changed_work_after_durable_stash_intent_is_rejected_without_stashing() {
    let fixture = dirty_anchor_fixture("v1-preservation-stale-preimage");
    fixture.seed_open();
    let context = fixture.context();
    let mut failing = FailFirstPreservationExecution {
        inner: ReverseRuntime::new(&fixture.backend, &context),
        failed: false,
    };

    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut failing,
    ) {
        Ok(_) => panic!("failed stash execution unexpectedly completed"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    let pending = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap();
    assert!(matches!(
        pending.record().pending_preservation,
        Some(PendingPreservationActionV1::Stash { .. })
    ));

    fs::write(fixture.member.join("README.md"), "changed after intent\n").unwrap();
    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let resume_context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &resume_context);
    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ) {
        Ok(_) => panic!("changed stash preimage unexpectedly advanced"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::PreservationEvidenceMismatch);
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert!(
        fixture
            .backend
            .preservation_stashes(&fixture.member, &fixture.model.merge_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn advanced_head_after_durable_stash_intent_is_rejected_without_stashing() {
    let fixture = dirty_anchor_fixture("v1-preservation-stale-stash-head");
    fixture.seed_open();
    let context = fixture.context();
    let mut failing = FailFirstPreservationExecution {
        inner: ReverseRuntime::new(&fixture.backend, &context),
        failed: false,
    };

    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut failing,
    ) {
        Ok(_) => panic!("failed stash execution unexpectedly completed"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    let pending = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap();
    assert!(matches!(
        pending.record().pending_preservation,
        Some(PendingPreservationActionV1::Stash { .. })
    ));

    let advanced = commit_file(
        &fixture.member,
        "advanced-after-intent.txt",
        "advanced after stash intent\n",
        "advance after stash intent",
        &[fixture.result.parse().unwrap()],
    )
    .unwrap();
    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let resume_context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &resume_context);
    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ) {
        Ok(_) => panic!("advanced stash HEAD unexpectedly completed"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::PreservationEvidenceMismatch);
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert_eq!(
        fixture
            .backend
            .head(&fixture.member)
            .unwrap()
            .commit
            .as_deref(),
        Some(advanced.as_str())
    );
    assert!(
        fixture
            .backend
            .preservation_stashes(&fixture.member, &fixture.model.merge_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn root_handoff_mapping_pins_every_non_degenerate_publishing_pair() {
    let mut fixture = dirty_root_handoff_fixture("v1-preservation-root-handoff-map");
    let publication = fixture.base.model.publication.as_ref().unwrap();
    let candidate = publication.candidate.as_ref().unwrap().clone();
    let marker_path = publication.candidate_marker_path.as_ref().unwrap().clone();
    let repository = git2::Repository::open(&fixture.base.root.path).unwrap();
    let baseline_lock_oid = git2::Oid::hash_object_ext(
        git2::ObjectType::Blob,
        candidate.baseline_lock_yaml.as_bytes(),
        repository.object_format(),
    )
    .unwrap()
    .to_string();
    let candidate_lock_oid = git2::Oid::hash_object_ext(
        git2::ObjectType::Blob,
        candidate.lock_yaml.as_bytes(),
        repository.object_format(),
    )
    .unwrap()
    .to_string();

    for (prefix, index) in [
        (P::Baseline, I::Pre),
        (P::Marker, I::Pre),
        (P::Lock, I::Pre),
        (P::Boundary, I::Pre),
        (P::Boundary, I::Staged),
    ] {
        fixture.base.model.preservation_publication_handoff =
            Some(PreservationPublicationHandoffV1::Candidate { prefix, index });
        let current = fixture.base.current();
        let plans = crate::workspace_ops::merge::preserve::v1_preservation_owners(
            &fixture.base.backend,
            &fixture.base.root.path,
            current.record(),
        )
        .unwrap();
        let plan = plans.last().unwrap();
        assert_eq!(plan.owner, PreservationOwnerV1::PublicationRoot);
        let spec = crate::workspace_ops::merge::preserve::v1_root_preservation_spec(
            &fixture.base.backend,
            current.record(),
            plan,
            &plan.protected_commit,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            spec.handoff_form
                .marker
                .as_ref()
                .map(|file| file.bytes.as_slice()),
            (prefix != P::Baseline).then_some(candidate.marker_yaml.as_bytes()),
            "{prefix:?}/{index:?} marker",
        );
        let expected_lock = if matches!(prefix, P::Lock | P::Boundary) {
            candidate.lock_yaml.as_bytes()
        } else {
            candidate.baseline_lock_yaml.as_bytes()
        };
        assert_eq!(spec.handoff_form.lock.bytes, expected_lock);
        assert_eq!(
            spec.handoff_boundary,
            if prefix == P::Boundary {
                candidate.boundary_text.as_bytes()
            } else {
                candidate.baseline_boundary_text.as_bytes()
            },
        );
        match index {
            I::Pre => {
                assert!(matches!(
                    &spec.handoff_form.index.marker,
                    GitRootManagedIndexFact::Absent { path }
                        if path == marker_path.as_bytes()
                ));
                assert_eq!(
                    index_oid(&spec.handoff_form.index.lock),
                    baseline_lock_oid,
                    "{prefix:?}/{index:?} lock index",
                );
            }
            I::Staged => {
                assert!(matches!(
                    &spec.handoff_form.index.marker,
                    GitRootManagedIndexFact::Present(entry)
                        if entry.path == marker_path.as_bytes()
                ));
                assert_eq!(
                    index_oid(&spec.handoff_form.index.lock),
                    candidate_lock_oid,
                    "{prefix:?}/{index:?} lock index",
                );
            }
        }
    }
}

#[test]
fn selected_root_is_the_only_root_preservation_owner() {
    let fixture = dirty_selected_root_handoff_fixture("v1-preservation-selected-root-owner");
    let current = fixture.base.current();
    let plans = crate::workspace_ops::merge::preserve::v1_preservation_owners(
        &fixture.base.backend,
        &fixture.base.root.path,
        current.record(),
    )
    .unwrap();
    assert!(plans.iter().any(|plan| {
        plan.owner
            == PreservationOwnerV1::Participant {
                member_id: "@root".into(),
            }
            && plan.root_handoff.is_some()
    }));
    assert!(
        plans
            .iter()
            .all(|plan| plan.owner != PreservationOwnerV1::PublicationRoot)
    );
}

fn index_oid(fact: &GitRootManagedIndexFact) -> &str {
    match fact {
        GitRootManagedIndexFact::Present(entry) => &entry.object_id,
        GitRootManagedIndexFact::Absent { .. } => panic!("expected a present index fact"),
    }
}

fn anchor_fixture(name: &str) -> PreservationFixture {
    let fixture = integrated_fixture(name);
    fixture
        .backend
        .set_branch_target_checked(&fixture.member, "main", &fixture.protected, &fixture.result)
        .unwrap();
    fixture
}

fn dirty_anchor_fixture(name: &str) -> PreservationFixture {
    let fixture = anchor_fixture(name);
    fs::write(fixture.member.join("README.md"), "dirty at anchor\n").unwrap();
    fs::write(fixture.member.join("staged.txt"), "staged at anchor\n").unwrap();
    fixture
        .backend
        .stage_paths(&fixture.member, &["staged.txt"])
        .unwrap();
    fs::write(
        fixture.member.join("untracked.txt"),
        "untracked at anchor\n",
    )
    .unwrap();
    fixture
}

struct FailFirstPreservationExecution<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    failed: bool,
}

impl ExactObserver for FailFirstPreservationExecution<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for FailFirstPreservationExecution<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if matches!(action, PhysicalActionKind::Preservation(_)) && !self.failed {
            self.failed = true;
            return ExecutionDiagnostic::Failed {
                code: ErrorCode::GitCommandFailed,
                message: "injected preservation execution failure".into(),
                detail: None,
            };
        }
        self.inner.execute(lease, current, action)
    }
}
