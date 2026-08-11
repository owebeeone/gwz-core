use super::*;
use crate::git::GitStashPushOptions;
use crate::model::ModelResult;
use crate::workspace_ops::merge::model::v1::{
    PendingPreservationActionV1, PreservationOwnerV1, PreservationStashPhaseV1 as S,
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
fn restart_after_each_non_root_physical_boundary_never_repeats_or_loses_work() {
    for crash_after in 1..=4 {
        let fixture =
            dirty_integrated_fixture(&format!("v1-preservation-crash-after-{crash_after}"));
        fixture.seed_open();
        let context = fixture.context();
        let mut crashing = CrashAfterPreservationAction {
            inner: ReverseRuntime::new(&fixture.backend, &context),
            crash_after,
            executions: 0,
        };

        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = run(
                &CheckedV1Store::default(),
                &fixture.root.path,
                &fixture.model.merge_id,
                V1LifecycleRequest::Preserve,
                &mut crashing,
            );
        }));
        assert!(crashed.is_err(), "boundary {crash_after} did not crash");

        let interrupted = CheckedV1Store::default()
            .load_open(&fixture.root.path, &fixture.model.merge_id)
            .unwrap();
        assert_eq!(interrupted.record().state, OperationState::Preserving);
        assert!(interrupted.record().pending_preservation.is_some());

        let resume_context = fixture.context();
        let mut resumed = ReverseRuntime::new(&fixture.backend, &resume_context);
        let response = run(
            &CheckedV1Store::default(),
            &fixture.root.path,
            &fixture.model.merge_id,
            V1LifecycleRequest::Preserve,
            &mut resumed,
        )
        .unwrap();
        assert_eq!(
            response.disposition(),
            V1ResponseDisposition::Terminal(OperationState::Aborted),
            "boundary {crash_after}",
        );
        assert_eq!(
            fixture
                .backend
                .head(&fixture.member)
                .unwrap()
                .commit
                .as_deref(),
            Some(fixture.before.as_str()),
            "boundary {crash_after}",
        );
        assert_eq!(
            fixture
                .backend
                .preservation_stashes(&fixture.member, &fixture.model.merge_id,)
                .unwrap()
                .len(),
            1,
            "boundary {crash_after}",
        );
    }
}

#[test]
fn later_pending_owner_cannot_advance_after_completed_prefix_regresses() {
    let mut fixture = dirty_integrated_fixture("v1-preservation-prefix-regression");
    let (later, _, _, later_protected) = add_integrated_member(&mut fixture, "mem_b", "members/b");
    fixture.seed_open();
    let context = fixture.context();
    let mut stopping = FailOwnerPreservationExecution {
        inner: ReverseRuntime::new(&fixture.backend, &context),
        member_id: "mem_b",
    };

    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut stopping,
    ) {
        Ok(_) => panic!("later-owner stop unexpectedly completed"),
        Err(error) => error,
    };
    assert_eq!(error.code, crate::model::ErrorCode::GitCommandFailed);
    let pending = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap();
    assert!(matches!(
        pending.record().pending_preservation,
        Some(PendingPreservationActionV1::BackupRef {
            owner: PreservationOwnerV1::Participant { ref member_id },
            ..
        }) if member_id == "mem_b"
    ));

    fs::write(
        fixture.member.join("new-earlier-work.txt"),
        "work created after mem_a completed\n",
    )
    .unwrap();
    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let later_head_before = fixture.backend.head(&later).unwrap();
    let later_image_before = fixture.backend.preservation_image(&later, true).unwrap();
    let resume_context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &resume_context);

    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ) {
        Ok(_) => panic!("regressed cursor prefix unexpectedly advanced"),
        Err(error) => error,
    };
    assert_eq!(
        error.code,
        crate::model::ErrorCode::PreservationEvidenceMismatch
    );
    assert_eq!(error.member_id.as_deref(), Some("mem_a"));
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert_eq!(fixture.backend.head(&later).unwrap(), later_head_before);
    assert_eq!(
        fixture.backend.preservation_image(&later, true).unwrap(),
        later_image_before
    );
    assert!(
        fixture
            .backend
            .read_ref(
                &later,
                &format!("refs/gwz/merge/{}/mem_b/head", fixture.model.merge_id),
            )
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .backend
            .preservation_stashes(&later, &fixture.model.merge_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        later_head_before.commit.as_deref(),
        Some(later_protected.as_str())
    );
}

#[test]
fn bundle_collision_after_entry_blocks_the_first_physical_mutation() {
    let fixture = dirty_integrated_fixture("v1-preservation-post-entry-bundle-collision");
    fixture.seed_open();
    let bundle = crate::stash::bundle_path(
        &fixture.root.path,
        &format!("stash_{}", fixture.model.merge_id),
    );
    fs::create_dir_all(bundle.parent().unwrap()).unwrap();
    fs::write(&bundle, "foreign post-entry bundle\n").unwrap();
    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let head_before = fixture.backend.head(&fixture.member).unwrap();
    let image_before = fixture
        .backend
        .preservation_image(&fixture.member, true)
        .unwrap();
    let context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);

    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ) {
        Ok(_) => panic!("post-entry bundle collision unexpectedly advanced"),
        Err(error) => error,
    };
    assert_eq!(
        error.code,
        crate::model::ErrorCode::PreservationEvidenceMismatch
    );
    assert_eq!(error.member_id.as_deref(), Some("mem_a"));
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert_eq!(fixture.backend.head(&fixture.member).unwrap(), head_before);
    assert_eq!(
        fixture
            .backend
            .preservation_image(&fixture.member, true)
            .unwrap(),
        image_before
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
        fixture
            .backend
            .preservation_stashes(&fixture.member, &fixture.model.merge_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn premature_foreign_root_stash_blocks_before_the_next_physical_mutation() {
    let mut fixture = dirty_root_handoff_fixture("v1-preservation-premature-root-stash");
    let current = fixture.base.current();
    let plans = crate::workspace_ops::merge::preserve::v1_preservation_owners(
        &fixture.base.backend,
        &fixture.base.root.path,
        current.record(),
    )
    .unwrap();
    let plan = plans
        .iter()
        .find(|plan| plan.owner == PreservationOwnerV1::PublicationRoot)
        .unwrap();
    let preimage = fixture
        .base
        .backend
        .preservation_image(&fixture.base.root.path, true)
        .unwrap()
        .preimage_sha256;
    let message = format!(
        "gwz:stash_{}: merge preservation",
        fixture.base.model.merge_id
    );
    fixture
        .base
        .backend
        .create_backup_ref(
            &fixture.base.root.path,
            &plan.backup_ref,
            &plan.protected_commit,
        )
        .unwrap();
    fixture
        .base
        .model
        .publication
        .as_mut()
        .unwrap()
        .root_preservation
        .push(crate::workspace_ops::merge::PreservationEvidence {
            backup_ref: Some(plan.backup_ref.clone()),
            backup_commit: Some(plan.protected_commit.clone()),
            stash_id: None,
            stash_object_id: None,
        });
    fixture.base.model.pending_preservation = Some(PendingPreservationActionV1::Stash {
        owner: plan.owner.clone(),
        phase: S::NormalizeParent,
        stash_id: None,
        stash_object_id: None,
        message: message.clone(),
        head_commit: plan.protected_commit.clone(),
        preimage_sha256: preimage,
        root_publication_handoff: plan.root_handoff,
    });

    fs::write(
        fixture.base.root.path.join("foreign-after-entry.txt"),
        "not in the persisted preimage\n",
    )
    .unwrap();
    fixture
        .base
        .backend
        .stash_push(
            &fixture.base.root.path,
            &message,
            GitStashPushOptions::include_untracked(),
        )
        .unwrap();
    assert_eq!(
        fixture
            .base
            .backend
            .preservation_stashes(&fixture.base.root.path, &fixture.base.model.merge_id,)
            .unwrap()
            .len(),
        1
    );
    fixture.base.seed_open();
    let record_path = fixture
        .base
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.base.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let head_before = fixture.base.backend.head(&fixture.base.root.path).unwrap();
    let image_before = fixture
        .base
        .backend
        .preservation_image(&fixture.base.root.path, true)
        .unwrap();
    let context = fixture.base.context();
    let mut runtime = ReverseRuntime::new(&fixture.base.backend, &context);

    let error = match run(
        &CheckedV1Store::default(),
        &fixture.base.root.path,
        &fixture.base.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ) {
        Ok(_) => panic!("premature foreign root stash unexpectedly advanced"),
        Err(error) => error,
    };
    assert_eq!(
        error.code,
        crate::model::ErrorCode::PreservationEvidenceMismatch
    );
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert_eq!(
        fixture.base.backend.head(&fixture.base.root.path).unwrap(),
        head_before
    );
    assert_eq!(
        fixture
            .base
            .backend
            .preservation_image(&fixture.base.root.path, true)
            .unwrap(),
        image_before
    );
}

struct CrashAfterPreservationAction<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    crash_after: usize,
    executions: usize,
}

struct FailOwnerPreservationExecution<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    member_id: &'static str,
}

impl ExactObserver for FailOwnerPreservationExecution<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for FailOwnerPreservationExecution<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        let matches_owner = matches!(
            action,
            PhysicalActionKind::Preservation(
                PendingPreservationActionV1::BackupRef {
                    owner: PreservationOwnerV1::Participant { member_id },
                    ..
                }
                | PendingPreservationActionV1::Stash {
                    owner: PreservationOwnerV1::Participant { member_id },
                    ..
                }
                | PendingPreservationActionV1::ResetAttachedRef {
                    owner: PreservationOwnerV1::Participant { member_id },
                    ..
                }
            ) if member_id == self.member_id
        );
        if matches_owner {
            return ExecutionDiagnostic::Failed {
                code: crate::model::ErrorCode::GitCommandFailed,
                message: "injected later-owner stop".into(),
                detail: None,
            };
        }
        self.inner.execute(lease, current, action)
    }
}

impl ExactObserver for CrashAfterPreservationAction<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for CrashAfterPreservationAction<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        let diagnostic = self.inner.execute(lease, current, action);
        if matches!(action, PhysicalActionKind::Preservation(_)) {
            self.executions += 1;
            if self.executions == self.crash_after {
                assert_eq!(diagnostic, ExecutionDiagnostic::Success);
                panic!("injected preservation crash");
            }
        }
        diagnostic
    }
}
