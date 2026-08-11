use super::{support::*, *};

#[test]
fn restore_parent_rejects_a_path_replacement_after_observation() {
    let fixture = fixture();
    let guard = guard(&prepare(&fixture));
    normalize(&fixture, &guard);
    let restores = restore_steps();
    for step in &restores[..2] {
        fixture
            .backend
            .execute_root_preservation_step_checked(
                &fixture.root,
                &fixture.spec,
                step,
                &GitRootPreservationGuard::OtherwiseClean,
            )
            .unwrap();
    }
    fs::remove_dir(fixture.root.join(crate::artifact::MARKER_DIR)).unwrap();
    let config = fixture.root.join("gwz.conf");
    let displaced = fixture.root.join("gwz.conf.displaced");
    let replacement = config.clone();
    run_next_at(FaultBoundary::Before, move || {
        fs::rename(config, displaced).unwrap();
        fs::create_dir(replacement).unwrap();
    });
    let error = fixture
        .backend
        .execute_root_preservation_step_checked(
            &fixture.root,
            &fixture.spec,
            &restores[2],
            &GitRootPreservationGuard::OtherwiseClean,
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PreservationEvidenceMismatch);
    assert!(!fixture.root.join(crate::artifact::MARKER_DIR).exists());
}

#[test]
fn restore_parent_namespace_states_are_closed_and_deterministic() {
    #[derive(Clone, Copy, Debug)]
    enum Shape {
        Missing,
        StageOnly,
        FinalOnly,
        Both,
        StageChild,
        StageFile,
        ForeignStage,
        DuplicateStage,
    }
    let cases = [
        (Shape::Missing, GitRootPreservationStepObservation::Before),
        (Shape::StageOnly, GitRootPreservationStepObservation::Before),
        (
            Shape::FinalOnly,
            GitRootPreservationStepObservation::AfterNeedsDurability,
        ),
        (Shape::Both, GitRootPreservationStepObservation::Ambiguous),
        (
            Shape::StageChild,
            GitRootPreservationStepObservation::Ambiguous,
        ),
        (
            Shape::StageFile,
            GitRootPreservationStepObservation::Ambiguous,
        ),
        (
            Shape::ForeignStage,
            GitRootPreservationStepObservation::Ambiguous,
        ),
        (
            Shape::DuplicateStage,
            GitRootPreservationStepObservation::Ambiguous,
        ),
    ];
    for (shape, expected) in cases {
        let (fixture, step) = parent_fixture();
        let stage = create_exact_stage(&fixture, &step);
        let final_path = fixture.root.join(crate::artifact::MARKER_DIR);
        match shape {
            Shape::Missing => fs::remove_dir(&stage).unwrap(),
            Shape::StageOnly => {}
            Shape::FinalOnly => fs::rename(&stage, &final_path).unwrap(),
            Shape::Both => fs::create_dir(&final_path).unwrap(),
            Shape::StageChild => fs::write(stage.join("foreign"), b"foreign\n").unwrap(),
            Shape::StageFile => {
                fs::remove_dir(&stage).unwrap();
                fs::write(&stage, b"not a directory\n").unwrap();
            }
            Shape::ForeignStage => {
                fs::remove_dir(&stage).unwrap();
                fs::create_dir(fixture.root.join("gwz.conf/.gwz-markers-foreign.stage")).unwrap();
            }
            Shape::DuplicateStage => {
                fs::create_dir(fixture.root.join("gwz.conf/.gwz-markers-foreign.stage")).unwrap();
            }
        }
        assert_eq!(observe_parent(&fixture, &step), expected, "{shape:?}");
        if expected == GitRootPreservationStepObservation::Ambiguous {
            assert_eq!(
                execute_parent(&fixture, &step).unwrap_err().code,
                ErrorCode::PreservationEvidenceMismatch,
                "{shape:?}"
            );
        }
    }
}

#[test]
fn restore_parent_faults_replay_from_their_exact_namespace_form() {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut cases = vec![
        (
            FaultBoundary::BeforeParentStageCreate,
            GitRootPreservationStepObservation::Before,
        ),
        (
            FaultBoundary::AfterParentStageCreate,
            GitRootPreservationStepObservation::Before,
        ),
        (
            FaultBoundary::BeforeParentPublish,
            GitRootPreservationStepObservation::Before,
        ),
        (
            FaultBoundary::AfterParentPublish,
            GitRootPreservationStepObservation::AfterNeedsDurability,
        ),
    ];
    #[cfg(unix)]
    cases.extend([
        (
            FaultBoundary::BeforeUnixParentSync,
            GitRootPreservationStepObservation::AfterNeedsDurability,
        ),
        (
            FaultBoundary::AfterUnixParentSync,
            GitRootPreservationStepObservation::AfterNeedsDurability,
        ),
    ]);
    for (boundary, expected) in cases {
        let (fixture, step) = parent_fixture();
        fail_next_at(boundary);
        assert_eq!(
            execute_parent(&fixture, &step).unwrap_err().code,
            ErrorCode::GitCommandFailed,
            "{boundary:?}"
        );
        assert_eq!(observe_parent(&fixture, &step), expected, "{boundary:?}");
        let replay = execute_parent(&fixture, &step).unwrap();
        assert_eq!(
            replay,
            if expected == GitRootPreservationStepObservation::Before {
                GitCheckedPreservationMutation::Applied
            } else {
                GitCheckedPreservationMutation::AlreadyComplete
            },
            "{boundary:?}"
        );
        assert_eq!(
            observe_parent(&fixture, &step),
            GitRootPreservationStepObservation::AfterNeedsDurability,
            "{boundary:?}"
        );
        assert_eq!(
            execute_parent(&fixture, &step).unwrap(),
            GitCheckedPreservationMutation::AlreadyComplete,
            "{boundary:?}"
        );
    }
}

#[test]
fn forward_parent_faults_replay_before_the_marker_phase() {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut cases = vec![
        (
            FaultBoundary::BeforeParentStageCreate,
            GitRootPreservationStepObservation::Before,
        ),
        (
            FaultBoundary::AfterParentStageCreate,
            GitRootPreservationStepObservation::Before,
        ),
        (
            FaultBoundary::BeforeParentPublish,
            GitRootPreservationStepObservation::Before,
        ),
        (
            FaultBoundary::AfterParentPublish,
            GitRootPreservationStepObservation::AfterNeedsDurability,
        ),
    ];
    #[cfg(unix)]
    cases.extend([
        (
            FaultBoundary::BeforeUnixParentSync,
            GitRootPreservationStepObservation::AfterNeedsDurability,
        ),
        (
            FaultBoundary::AfterUnixParentSync,
            GitRootPreservationStepObservation::AfterNeedsDurability,
        ),
    ]);
    for (boundary, expected) in cases {
        let (fixture, step, guard) = forward_parent_fixture();
        fail_next_at(boundary);
        assert_eq!(
            execute_step(&fixture, &step, &guard).unwrap_err().code,
            ErrorCode::GitCommandFailed,
            "{boundary:?}"
        );
        assert_eq!(
            observe_step(&fixture, &step, &guard),
            expected,
            "{boundary:?}"
        );
        let replay = execute_step(&fixture, &step, &guard).unwrap();
        assert_eq!(
            replay,
            if expected == GitRootPreservationStepObservation::Before {
                GitCheckedPreservationMutation::Applied
            } else {
                GitCheckedPreservationMutation::AlreadyComplete
            },
            "{boundary:?}"
        );
        assert_eq!(
            observe_step(&fixture, &step, &guard),
            GitRootPreservationStepObservation::AfterNeedsDurability
        );
        assert!(!fixture.root.join(MARKER).exists());
    }
}

#[test]
fn publication_collision_is_ambiguous_and_never_replaces_either_name() {
    let (fixture, step) = parent_fixture();
    let final_path = fixture.root.join(crate::artifact::MARKER_DIR);
    let collision = final_path.clone();
    run_next_at(FaultBoundary::BeforeParentPublish, move || {
        fs::create_dir(collision).unwrap();
    });
    assert!(execute_parent(&fixture, &step).is_err());
    assert_eq!(
        observe_parent(&fixture, &step),
        GitRootPreservationStepObservation::Ambiguous
    );
    assert!(final_path.is_dir());
    assert_eq!(stages(&fixture).len(), 1);
}

#[cfg(unix)]
#[test]
fn parent_and_leaf_replacements_are_contained_inside_the_repository() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[derive(Clone, Copy, Debug)]
    enum Shape {
        Real,
        Symlink,
        File,
        Directory,
        Executable,
    }
    for shape in [
        Shape::Real,
        Shape::Symlink,
        Shape::File,
        Shape::Directory,
        Shape::Executable,
    ] {
        let fixture = fixture();
        let guard = guard(&prepare(&fixture));
        let steps = normalize_steps();
        fixture
            .backend
            .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, &steps[0], &guard)
            .unwrap();
        let outside = fixture.root.parent().unwrap().join("outside-leaf");
        fs::create_dir(&outside).unwrap();
        let sentinel = outside.join("sentinel");
        fs::write(&sentinel, b"outside\n").unwrap();
        let leaf = fixture.root.join(crate::artifact::LOCK_PATH);
        let target = sentinel.clone();
        run_next_at(FaultBoundary::Before, move || match shape {
            Shape::Real => {}
            Shape::Symlink => {
                fs::remove_file(&leaf).unwrap();
                symlink(target, leaf).unwrap();
            }
            Shape::File => fs::write(leaf, b"replacement\n").unwrap(),
            Shape::Directory => {
                fs::remove_file(&leaf).unwrap();
                fs::create_dir(leaf).unwrap();
            }
            Shape::Executable => {
                fs::write(&leaf, b"handoff lock\n").unwrap();
                fs::set_permissions(leaf, fs::Permissions::from_mode(0o755)).unwrap();
            }
        });
        let result = fixture.backend.execute_root_preservation_step_checked(
            &fixture.root,
            &fixture.spec,
            &steps[1],
            &guard,
        );
        if matches!(shape, Shape::Real) {
            assert!(result.is_ok(), "{shape:?}");
        } else {
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::PreservationEvidenceMismatch,
                "{shape:?}"
            );
        }
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside\n", "{shape:?}");
    }

    for shape in [Shape::Symlink, Shape::File, Shape::Directory] {
        let fixture = fixture();
        let guard = guard(&prepare(&fixture));
        let outside = fixture.root.parent().unwrap().join("outside-parent");
        fs::create_dir(&outside).unwrap();
        let sentinel = outside.join("sentinel");
        fs::write(&sentinel, b"outside\n").unwrap();
        let parent = fixture.root.join("gwz.conf");
        let displaced = fixture.root.join("gwz.conf.displaced");
        let target = outside.clone();
        run_next_at(FaultBoundary::Before, move || {
            fs::rename(&parent, displaced).unwrap();
            match shape {
                Shape::Symlink => symlink(target, parent).unwrap(),
                Shape::File => fs::write(parent, b"replacement\n").unwrap(),
                Shape::Directory => fs::create_dir(parent).unwrap(),
                _ => unreachable!(),
            }
        });
        assert_eq!(
            fixture
                .backend
                .execute_root_preservation_step_checked(
                    &fixture.root,
                    &fixture.spec,
                    &normalize_steps()[1],
                    &guard,
                )
                .unwrap_err()
                .code,
            ErrorCode::PreservationEvidenceMismatch,
            "{shape:?}"
        );
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside\n", "{shape:?}");
    }
}

#[cfg(windows)]
#[test]
fn windows_round_trip_faults_have_branchless_restart_forms() {
    let cases = [
        (
            FaultBoundary::BeforeWindowsFirstBarrierRename,
            GitRootPreservationStepObservation::AfterNeedsDurability,
        ),
        (
            FaultBoundary::AfterWindowsFirstBarrierRename,
            GitRootPreservationStepObservation::Before,
        ),
        (
            FaultBoundary::BeforeWindowsSecondBarrierRename,
            GitRootPreservationStepObservation::Before,
        ),
        (
            FaultBoundary::AfterWindowsSecondBarrierRename,
            GitRootPreservationStepObservation::AfterNeedsDurability,
        ),
    ];
    for (boundary, expected) in cases {
        let (fixture, step) = parent_fixture();
        assert_eq!(
            execute_parent(&fixture, &step).unwrap(),
            GitCheckedPreservationMutation::Applied
        );
        fail_next_at(boundary);
        assert_eq!(
            execute_parent(&fixture, &step).unwrap_err().code,
            ErrorCode::GitCommandFailed
        );
        assert_eq!(observe_parent(&fixture, &step), expected, "{boundary:?}");
        execute_parent(&fixture, &step).unwrap();
        assert_eq!(
            observe_parent(&fixture, &step),
            GitRootPreservationStepObservation::AfterNeedsDurability
        );
    }
}
