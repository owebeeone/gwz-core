use super::root_successor_matrix::{PhaseTarget, expected_targets};
use super::*;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{RecoveryContextV1, RecoveryOriginStateV1};
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
use crate::workspace_ops::merge::v1_lifecycle::tests::c7_matrix::matrix_spec::REQUESTS;

macro_rules! ambiguity_matrix_test {
    ($name:ident, $owner:expr, $request_index:expr) => {
        #[test]
        fn $name() {
            assert_ambiguous_request($owner, $request_index, REQUESTS[$request_index]);
        }
    };
}

ambiguity_matrix_test!(
    publication_resume_start_rejects_ambiguity,
    RootOwner::Publication,
    0
);
ambiguity_matrix_test!(
    publication_continue_rejects_ambiguity,
    RootOwner::Publication,
    1
);
ambiguity_matrix_test!(
    publication_abort_rejects_ambiguity,
    RootOwner::Publication,
    2
);
ambiguity_matrix_test!(
    publication_preserve_rejects_ambiguity,
    RootOwner::Publication,
    3
);
ambiguity_matrix_test!(
    publication_archive_rejects_ambiguity,
    RootOwner::Publication,
    4
);
ambiguity_matrix_test!(
    selected_resume_start_rejects_ambiguity,
    RootOwner::Selected,
    0
);
ambiguity_matrix_test!(selected_continue_rejects_ambiguity, RootOwner::Selected, 1);
ambiguity_matrix_test!(selected_abort_rejects_ambiguity, RootOwner::Selected, 2);
ambiguity_matrix_test!(selected_preserve_rejects_ambiguity, RootOwner::Selected, 3);
ambiguity_matrix_test!(selected_archive_rejects_ambiguity, RootOwner::Selected, 4);

fn assert_ambiguous_request(owner: RootOwner, request_index: usize, request: V1LifecycleRequest) {
    for (target_index, target) in expected_targets().into_iter().enumerate() {
        let fixture = root_fixture(
            owner,
            &format!("v1-root-ambiguous-{owner:?}-{target_index}-{request_index}"),
        );
        fixture.base.seed_open();
        let operation_context = fixture.base.context();
        let mut stop = StopBeforePhaseObservation {
            inner: ReverseRuntime::new(&fixture.base.backend, &operation_context),
            target,
            stopped: false,
        };
        let error = match run(
            &CheckedV1Store::default(),
            &fixture.base.root.path,
            &fixture.base.model.merge_id,
            V1LifecycleRequest::Preserve,
            &mut stop,
        ) {
            Ok(_) => panic!("{owner:?} {target:?} did not retain its phase journal"),
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::GitCommandFailed);
        assert!(stop.stopped, "{owner:?} {target:?}");

        let store = CheckedV1Store::default();
        let pending = store
            .load_open(&fixture.base.root.path, &fixture.base.model.merge_id)
            .unwrap();
        assert_eq!(
            PhaseTarget::from_record(pending.record()),
            Some(target),
            "{owner:?} {target:?}"
        );
        seed_recovery(&fixture.base.root.path, pending.record());
        let ambiguity = install_ambiguity(&fixture, pending.record(), target_index);

        let resume_context = fixture.base.context();
        let mut runtime = CountingRuntime {
            inner: ReverseRuntime::new(&fixture.base.backend, &resume_context),
            target,
            executions: 0,
        };
        let result = run(
            &store,
            &fixture.base.root.path,
            &fixture.base.model.merge_id,
            request,
            &mut runtime,
        );
        if let Ok(response) = &result {
            assert_eq!(
                response.disposition(),
                V1ResponseDisposition::Stopped(OperationState::RecoveryRequired),
                "{owner:?} {target:?} {request:?}"
            );
        }
        assert_eq!(runtime.executions, 0, "{owner:?} {target:?} {request:?}");
        let retained = store
            .load_open(&fixture.base.root.path, &fixture.base.model.merge_id)
            .unwrap();
        assert_eq!(
            PhaseTarget::from_record(retained.record()),
            Some(target),
            "{owner:?} {target:?} {request:?}"
        );

        ambiguity.remove(&fixture.base.backend, &fixture.base.root.path);
        seed_recovery(&fixture.base.root.path, retained.record());
        let mut terminal = run(
            &store,
            &fixture.base.root.path,
            &fixture.base.model.merge_id,
            V1LifecycleRequest::Preserve,
            &mut runtime,
        )
        .unwrap();
        for retry in 1..8 {
            if terminal.disposition()
                != V1ResponseDisposition::Stopped(OperationState::RecoveryRequired)
            {
                break;
            }
            terminal = run(
                &store,
                &fixture.base.root.path,
                &fixture.base.model.merge_id,
                if retry % 2 == 0 {
                    V1LifecycleRequest::Abort
                } else {
                    V1LifecycleRequest::Preserve
                },
                &mut runtime,
            )
            .unwrap();
        }
        assert_eq!(
            terminal.disposition(),
            V1ResponseDisposition::Terminal(OperationState::Aborted),
            "{owner:?} {target:?} {request:?}"
        );
    }
}

enum AmbiguityMutation {
    BackupRef(String),
    ForeignFile(std::path::PathBuf),
}

impl AmbiguityMutation {
    fn remove(self, backend: &impl GitBackend, root: &std::path::Path) {
        match self {
            Self::BackupRef(name) => backend.test_set_ref(root, &name, None).unwrap(),
            Self::ForeignFile(path) => fs::remove_file(path).unwrap(),
        }
    }
}

fn install_ambiguity(
    fixture: &RootPreservationFixture<GitTestRepository>,
    model: &MergeOperationRecordV1,
    target_index: usize,
) -> AmbiguityMutation {
    match model.pending_preservation.as_ref().unwrap() {
        crate::workspace_ops::merge::model::v1::PendingPreservationActionV1::BackupRef {
            name,
            target_commit,
            ..
        } => {
            assert_ne!(fixture.anchor, *target_commit);
            fixture
                .base
                .backend
                .test_set_ref(
                    &fixture.base.root.path,
                    name,
                    Some(&TestRefTarget::Direct(fixture.anchor.clone())),
                )
                .unwrap();
            AmbiguityMutation::BackupRef(name.clone())
        }
        crate::workspace_ops::merge::model::v1::PendingPreservationActionV1::Stash {
            phase: crate::workspace_ops::merge::model::v1::PreservationStashPhaseV1::WriteBundle,
            ..
        } => {
            let path = crate::stash::bundle_path(
                &fixture.base.root.path,
                &format!("stash_{}", model.merge_id),
            );
            fs::write(&path, "foreign bundle bytes\n").unwrap();
            AmbiguityMutation::ForeignFile(path)
        }
        _ => {
            let path = fixture
                .base
                .root
                .path
                .join(format!("ambiguous-{target_index}.txt"));
            fs::write(&path, "foreign work after durable phase intent\n").unwrap();
            AmbiguityMutation::ForeignFile(path)
        }
    }
}

fn seed_recovery(root: &std::path::Path, model: &MergeOperationRecordV1) {
    let mut recovery = model.clone();
    recovery.state = OperationState::RecoveryRequired;
    recovery.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Preserving,
    });
    fs::write(
        root.join(format!(".gwz/merge/{}.yaml", recovery.merge_id)),
        serde_yaml::to_string(&recovery).unwrap(),
    )
    .unwrap();
}

#[derive(Clone, Copy, Debug)]
enum RootOwner {
    Publication,
    Selected,
}

fn root_fixture(owner: RootOwner, name: &str) -> RootPreservationFixture<GitTestRepository> {
    dirty_root_handoff_fixture_using(
        name,
        matches!(owner, RootOwner::Selected),
        false,
        false,
        make_repository(),
    )
}

struct StopBeforePhaseObservation<'a> {
    inner: ReverseRuntime<'a, GitTestRepository>,
    target: PhaseTarget,
    stopped: bool,
}

impl ExactObserver for StopBeforePhaseObservation<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        if PhaseTarget::from_record(current.record()) == Some(self.target) && !self.stopped {
            self.stopped = true;
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "injected stop before root phase observation",
            ));
        }
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for StopBeforePhaseObservation<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.inner.execute(lease, current, action)
    }
}

struct CountingRuntime<'a> {
    inner: ReverseRuntime<'a, GitTestRepository>,
    target: PhaseTarget,
    executions: usize,
}

impl ExactObserver for CountingRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for CountingRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if self.target.matches_action(action) {
            self.executions += 1;
        }
        self.inner.execute(lease, current, action)
    }
}
