use super::{support::*, *};

#[test]
fn granular_leaf_and_index_faults_have_exact_restart_classification() {
    let leaf_cases = [
        (
            1,
            CheckedArtifactFault::BeforeFinalCheck,
            GitRootPreservationStepObservation::Before,
        ),
        (
            1,
            CheckedArtifactFault::AfterMutation,
            GitRootPreservationStepObservation::After,
        ),
        (
            2,
            CheckedArtifactFault::BeforeFinalCheck,
            GitRootPreservationStepObservation::Before,
        ),
        (
            2,
            CheckedArtifactFault::AfterMutation,
            GitRootPreservationStepObservation::After,
        ),
    ];
    for (step_index, boundary, expected) in leaf_cases {
        let fixture = fixture();
        let guard = guard(&prepare(&fixture));
        let steps = normalize_steps();
        for step in &steps[..step_index] {
            fixture
                .backend
                .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, step, &guard)
                .unwrap();
        }
        fail_next_checked_artifact_at(boundary);
        assert_eq!(
            fixture
                .backend
                .execute_root_preservation_step_checked(
                    &fixture.root,
                    &fixture.spec,
                    &steps[step_index],
                    &guard,
                )
                .unwrap_err()
                .code,
            ErrorCode::PreservationEvidenceMismatch,
            "{boundary:?}"
        );
        assert_eq!(
            fixture
                .backend
                .observe_root_preservation_step(
                    &fixture.root,
                    &fixture.spec,
                    &steps[step_index],
                    &guard,
                )
                .unwrap(),
            expected,
            "{boundary:?}"
        );
        fixture
            .backend
            .execute_root_preservation_step_checked(
                &fixture.root,
                &fixture.spec,
                &steps[step_index],
                &guard,
            )
            .unwrap();
    }

    let index_cases = [
        (
            3,
            FaultBoundary::BeforeIndexCommit,
            GitRootPreservationStepObservation::Before,
        ),
        (
            3,
            FaultBoundary::AfterIndexCommit,
            GitRootPreservationStepObservation::After,
        ),
    ];
    for (step_index, boundary, expected) in index_cases {
        let fixture = fixture();
        let guard = guard(&prepare(&fixture));
        let steps = normalize_steps();
        for step in &steps[..step_index] {
            fixture
                .backend
                .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, step, &guard)
                .unwrap();
        }
        fail_next_at(boundary);
        assert_eq!(
            fixture
                .backend
                .execute_root_preservation_step_checked(
                    &fixture.root,
                    &fixture.spec,
                    &steps[step_index],
                    &guard,
                )
                .unwrap_err()
                .code,
            ErrorCode::GitCommandFailed,
            "{boundary:?}"
        );
        assert_eq!(
            fixture
                .backend
                .observe_root_preservation_step(
                    &fixture.root,
                    &fixture.spec,
                    &steps[step_index],
                    &guard,
                )
                .unwrap(),
            expected,
            "{boundary:?}"
        );
        let replay = fixture
            .backend
            .execute_root_preservation_step_checked(
                &fixture.root,
                &fixture.spec,
                &steps[step_index],
                &guard,
            )
            .unwrap();
        assert_eq!(
            replay,
            if expected == GitRootPreservationStepObservation::Before {
                GitCheckedPreservationMutation::Applied
            } else {
                GitCheckedPreservationMutation::AlreadyComplete
            },
            "{boundary:?}"
        );
    }
}
