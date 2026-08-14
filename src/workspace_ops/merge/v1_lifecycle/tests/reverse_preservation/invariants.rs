use super::*;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{PendingPreservationActionV1, PreservationOwnerV1};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    V1LifecycleRequest,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::{
    ExactObserver, PhysicalExecutor, run_test as run,
};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[test]
fn absent_backup_ref_is_not_before_after_the_persisted_head_advances() {
    let fixture = dirty_integrated_fixture("v1-preservation-backup-stale-head");
    fixture.seed_open();
    let context = fixture.context();
    let mut stopping = StopBeforeBackup {
        inner: ReverseRuntime::new(&fixture.backend, &context),
        stopped: false,
    };
    assert_eq!(
        expect_error(run(
            &CheckedV1Store::default(),
            &fixture.root.path,
            &fixture.model.merge_id,
            V1LifecycleRequest::Preserve,
            &mut stopping,
        ))
        .code,
        ErrorCode::GitCommandFailed
    );
    let stored = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap();
    assert!(matches!(
        stored.record().pending_preservation,
        Some(PendingPreservationActionV1::BackupRef { .. })
    ));
    let advanced = commit_file(
        &fixture.member,
        "advanced.txt",
        "advanced after durable intent\n",
        "advance after intent",
        &[fixture.protected.parse().unwrap()],
    )
    .unwrap();
    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);
    let error = expect_error(run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ));
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
            .read_ref(
                &fixture.member,
                &format!("refs/gwz/merge/{}/mem_a/head", fixture.model.merge_id),
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn new_work_in_an_earlier_skipped_owner_is_rejected_before_later_evidence_mutates() {
    let mut fixture = integrated_fixture("v1-preservation-later-durable-prefix");
    fixture
        .backend
        .set_branch_target_checked(&fixture.member, "main", &fixture.protected, &fixture.result)
        .unwrap();
    let (later, _, _, _) = add_integrated_member(&mut fixture, "mem_b", "members/b");
    fs::write(later.join("README.md"), "later dirty work\n").unwrap();
    fs::write(later.join("untracked.txt"), "later untracked\n").unwrap();
    fixture.seed_open();
    let context = fixture.context();
    let mut stopping = StopAfterLaterBundle {
        inner: ReverseRuntime::new(&fixture.backend, &context),
        stopped: false,
    };
    assert_eq!(
        expect_error(run(
            &CheckedV1Store::default(),
            &fixture.root.path,
            &fixture.model.merge_id,
            V1LifecycleRequest::Preserve,
            &mut stopping,
        ))
        .code,
        ErrorCode::GitCommandFailed
    );
    let stored = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap();
    assert!(stored.record().pending_preservation.is_none());
    assert!(
        stored.record().participants["mem_b"]
            .preservation
            .iter()
            .any(|row| row.stash_id.is_some())
    );
    fs::write(
        fixture.member.join("new-earlier-work.txt"),
        "new after earlier owner was skipped\n",
    )
    .unwrap();
    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let later_head = fixture.backend.head(&later).unwrap();
    let context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);
    let error = expect_error(run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ));
    assert_eq!(error.code, ErrorCode::PreservationEvidenceMismatch);
    assert_eq!(error.member_id.as_deref(), Some("mem_a"));
    assert_eq!(fs::read(record_path).unwrap(), record_before);
    assert_eq!(fixture.backend.head(&later).unwrap(), later_head);
    assert!(
        fixture
            .backend
            .preservation_stashes(&fixture.member, &fixture.model.merge_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fixture
            .backend
            .preservation_stashes(&later, &fixture.model.merge_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn bundle_rows_are_canonical_when_cursor_order_is_not() {
    let mut fixture = dirty_integrated_fixture("v1-preservation-bundle-canonical-order");
    let (later, _, _, _) = add_integrated_member(&mut fixture, "mem_z", "members/z");
    fs::write(later.join("README.md"), "z dirty work\n").unwrap();
    fs::write(later.join("untracked.txt"), "z untracked\n").unwrap();
    let mut manifest = crate::artifact::ManifestArtifact::from_yaml(
        fixture.model.baseline.manifest_yaml.as_deref().unwrap(),
    )
    .unwrap();
    manifest
        .members
        .sort_by(|left, right| right.id.cmp(&left.id));
    let manifest = manifest.to_yaml().unwrap();
    fixture.model.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest.as_bytes()));
    fixture.model.baseline.manifest_yaml = Some(manifest);
    fixture.model.selected_targets = vec!["mem_z".into(), "mem_a".into()];
    fixture.seed_open();
    let context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);
    run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    )
    .unwrap();
    let bundle = crate::stash::read_bundle(
        &fixture.root.path,
        &format!("stash_{}", fixture.model.merge_id),
    )
    .unwrap();
    assert_eq!(bundle.selected_members, ["mem_a", "mem_z"]);
    assert_eq!(
        bundle
            .members
            .iter()
            .map(|row| row.member_id.as_str())
            .collect::<Vec<_>>(),
        ["mem_a", "mem_z"]
    );
}

#[test]
fn root_inclusive_bundle_has_canonical_owner_order_and_bytes() {
    let fixture = dirty_selected_root_handoff_fixture_with_later_member(
        "v1-preservation-bundle-root-inclusive-order",
    );
    for (path, value) in [
        (fixture.base.member.join("dirty-a.txt"), "dirty a\n"),
        (
            fixture.base.root.path.join("members/z/dirty-z.txt"),
            "dirty z\n",
        ),
    ] {
        fs::write(path, value).unwrap();
    }
    fixture.base.seed_open();
    let context = fixture.base.context();
    let mut runtime = ReverseRuntime::new(&fixture.base.backend, &context);
    run(
        &CheckedV1Store::default(),
        &fixture.base.root.path,
        &fixture.base.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    )
    .unwrap();
    let stash_id = format!("stash_{}", fixture.base.model.merge_id);
    let path = crate::stash::bundle_path(&fixture.base.root.path, &stash_id);
    let bytes = fs::read_to_string(&path).unwrap();
    let bundle = crate::stash::read_bundle(&fixture.base.root.path, &stash_id).unwrap();
    assert_eq!(bundle.selected_members, ["@root", "mem_a", "mem_z"]);
    assert_eq!(
        bundle
            .members
            .iter()
            .map(|row| row.member_id.as_str())
            .collect::<Vec<_>>(),
        ["@root", "mem_a", "mem_z"]
    );
    assert_eq!(bytes, bundle.to_yaml().unwrap());
}

#[test]
fn foreign_bundle_inserted_before_publication_is_not_overwritten() {
    use crate::checked_artifact::{CheckedArtifactFault, run_next_checked_artifact_at};

    let fixture = dirty_integrated_fixture("v1-preservation-bundle-foreign-insert");
    fixture.seed_open();
    let context = fixture.context();
    let mut stopping = StopBeforeBundle {
        inner: ReverseRuntime::new(&fixture.backend, &context),
        stopped: false,
    };
    assert_eq!(
        expect_error(run(
            &CheckedV1Store::default(),
            &fixture.root.path,
            &fixture.model.merge_id,
            V1LifecycleRequest::Preserve,
            &mut stopping,
        ))
        .code,
        ErrorCode::GitCommandFailed
    );
    let bundle = crate::stash::bundle_path(
        &fixture.root.path,
        &format!("stash_{}", fixture.model.merge_id),
    );
    assert!(!bundle.exists());
    let insertion = bundle.clone();
    run_next_checked_artifact_at(CheckedArtifactFault::BeforeFinalCheck, move || {
        fs::write(insertion, "foreign bundle\n").unwrap();
    });
    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let record_before = fs::read(&record_path).unwrap();
    let context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);
    let error = expect_error(run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    ));
    assert_eq!(error.code, ErrorCode::PreservationEvidenceMismatch);
    assert_eq!(fs::read_to_string(bundle).unwrap(), "foreign bundle\n");
    assert_eq!(fs::read(record_path).unwrap(), record_before);
}

struct StopBeforeBackup<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    stopped: bool,
}

impl ExactObserver for StopBeforeBackup<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for StopBeforeBackup<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if !self.stopped
            && matches!(
                action,
                PhysicalActionKind::Preservation(PendingPreservationActionV1::BackupRef {
                    owner: PreservationOwnerV1::Participant { member_id },
                    ..
                }) if member_id == "mem_a"
            )
        {
            self.stopped = true;
            return failure("stop before backup");
        }
        self.inner.execute(lease, current, action)
    }
}

struct StopAfterLaterBundle<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    stopped: bool,
}

struct StopBeforeBundle<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    stopped: bool,
}

impl ExactObserver for StopBeforeBundle<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for StopBeforeBundle<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if !self.stopped
            && matches!(
                action,
                PhysicalActionKind::Preservation(PendingPreservationActionV1::Stash {
                    phase: crate::workspace_ops::merge::model::v1::PreservationStashPhaseV1::WriteBundle,
                    ..
                })
            )
        {
            self.stopped = true;
            return failure("stop before bundle");
        }
        self.inner.execute(lease, current, action)
    }
}

impl ExactObserver for StopAfterLaterBundle<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        if !self.stopped
            && current.record().pending_preservation.is_none()
            && current.record().participants["mem_b"]
                .preservation
                .iter()
                .any(|row| row.stash_id.is_some())
        {
            self.stopped = true;
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "stop after later durable bundle",
            ));
        }
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for StopAfterLaterBundle<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.inner.execute(lease, current, action)
    }
}

fn failure(message: &str) -> ExecutionDiagnostic {
    ExecutionDiagnostic::Failed {
        code: ErrorCode::GitCommandFailed,
        message: message.into(),
        detail: None,
    }
}

fn expect_error<T>(result: ModelResult<T>) -> ModelError {
    match result {
        Ok(_) => panic!("operation unexpectedly completed"),
        Err(error) => error,
    }
}
