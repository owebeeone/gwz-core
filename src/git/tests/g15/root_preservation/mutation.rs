use super::support::*;
use super::*;

use std::process::Command;

fn observe(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
) -> GitRootPreservationStepObservation {
    fixture
        .backend
        .observe_root_preservation_step(&fixture.root, &fixture.spec, step, guard)
        .unwrap()
}

fn execute(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
) -> Result<GitCheckedPreservationMutation, crate::model::ModelError> {
    fixture.backend.execute_root_preservation_step_checked(
        &fixture.root,
        &fixture.spec,
        step,
        guard,
    )
}

fn reset_restore_steps() -> [GitRootPreservationPhysicalStep; 4] {
    [
        GitRootManagedObject::Index,
        GitRootManagedObject::LockWorktree,
        GitRootManagedObject::MarkerParentDirectory,
        GitRootManagedObject::MarkerWorktree,
    ]
    .map(|object| {
        managed_step(
            object,
            GitRootManagedFormName::RestoreClean,
            GitRootManagedFormName::Handoff,
        )
    })
}

fn assert_row(
    fixture: &RootFixture,
    steps: &[GitRootPreservationPhysicalStep],
    at: usize,
    guard: &GitRootPreservationGuard,
) {
    for step in &steps[..at] {
        assert!(matches!(
            execute(fixture, step, guard).unwrap(),
            GitCheckedPreservationMutation::Applied
                | GitCheckedPreservationMutation::AlreadyComplete
        ));
    }
    let step = &steps[at];
    assert_eq!(
        observe(fixture, step, guard),
        GitRootPreservationStepObservation::Before
    );
    assert_eq!(
        execute(fixture, step, guard).unwrap(),
        GitCheckedPreservationMutation::Applied
    );
    let after = if matches!(
        step,
        GitRootPreservationPhysicalStep::Managed(GitRootManagedTransition {
            object: GitRootManagedObject::MarkerParentDirectory,
            ..
        })
    ) {
        GitRootPreservationStepObservation::AfterNeedsDurability
    } else {
        GitRootPreservationStepObservation::After
    };
    assert_eq!(observe(fixture, step, guard), after);
    let completed = exact_snapshot(fixture);
    assert_eq!(
        execute(fixture, step, guard).unwrap(),
        GitCheckedPreservationMutation::AlreadyComplete
    );
    assert_eq!(exact_snapshot(fixture), completed);
    assert_eq!(observe(fixture, step, guard), after);
}

#[test]
fn every_physical_phase_row_is_before_after_and_restart_safe() {
    for format in ["sha1", "sha256"] {
        physical_phase_matrix(format);
    }
}

fn physical_phase_matrix(format: &str) {
    for at in 1..normalize_steps().len() {
        let fixture = fixture_with_format(format);
        let guard = guard(&prepare(&fixture));
        assert_row(&fixture, &normalize_steps(), at, &guard);
    }
    for at in 0..restore_steps().len() {
        let fixture = fixture_with_format(format);
        fs::write(fixture.root.join("user.txt"), b"preserve\n").unwrap();
        let guard = guard(&prepare(&fixture));
        normalize(&fixture, &guard);
        execute(
            &fixture,
            &GitRootPreservationPhysicalStep::CreateStash {
                merge_id: "merge_1".into(),
            },
            &guard,
        )
        .unwrap();
        assert_row(
            &fixture,
            &restore_steps(),
            at,
            &GitRootPreservationGuard::OtherwiseClean,
        );
    }
    for at in 1..normalize_steps().len() {
        let fixture = fixture_with_format(format);
        assert_row(
            &fixture,
            &normalize_steps(),
            at,
            &GitRootPreservationGuard::OtherwiseClean,
        );
    }
    for at in 0..reset_restore_steps().len() {
        let fixture = fixture_with_format(format);
        for step in normalize_steps() {
            execute(&fixture, &step, &GitRootPreservationGuard::OtherwiseClean).unwrap();
        }
        let reset = GitRootPreservationPhysicalStep::ResetAttachedRef;
        assert_eq!(
            observe(&fixture, &reset, &GitRootPreservationGuard::OtherwiseClean),
            GitRootPreservationStepObservation::Before
        );
        assert!(matches!(
            execute(&fixture, &reset, &GitRootPreservationGuard::OtherwiseClean).unwrap(),
            GitCheckedPreservationMutation::RefReset(_)
        ));
        assert_eq!(
            observe(&fixture, &reset, &GitRootPreservationGuard::OtherwiseClean),
            GitRootPreservationStepObservation::After
        );
        let reset_complete = exact_snapshot(&fixture);
        assert!(matches!(
            execute(&fixture, &reset, &GitRootPreservationGuard::OtherwiseClean).unwrap(),
            GitCheckedPreservationMutation::AlreadyComplete
        ));
        assert_eq!(exact_snapshot(&fixture), reset_complete);
        if at >= 2 {
            let parent = fixture.root.join(crate::artifact::MARKER_DIR);
            if parent.is_dir() {
                fs::remove_dir(parent).unwrap();
            }
        }
        assert_row(
            &fixture,
            &reset_restore_steps(),
            at,
            &GitRootPreservationGuard::OtherwiseClean,
        );
    }
}

#[test]
fn missing_handoff_parent_is_published_before_a_present_clean_marker() {
    let fixture = fixture_with_markers("sha1", Some(b"attached marker\n"), None, None);
    let guard = guard(&prepare(&fixture));
    let parent = managed_step(
        GitRootManagedObject::MarkerParentDirectory,
        GitRootManagedFormName::Handoff,
        GitRootManagedFormName::AttachedClean,
    );
    let marker = managed_step(
        GitRootManagedObject::MarkerWorktree,
        GitRootManagedFormName::Handoff,
        GitRootManagedFormName::AttachedClean,
    );
    let marker_path = fixture.root.join(MARKER);
    let parent_path = fixture.root.join(crate::artifact::MARKER_DIR);
    assert!(!parent_path.exists());
    assert_eq!(
        observe(&fixture, &parent, &guard),
        GitRootPreservationStepObservation::Before
    );
    assert_eq!(
        execute(&fixture, &parent, &guard).unwrap(),
        GitCheckedPreservationMutation::Applied
    );
    assert!(parent_path.is_dir());
    assert!(!marker_path.exists());
    assert_eq!(
        observe(&fixture, &parent, &guard),
        GitRootPreservationStepObservation::AfterNeedsDurability
    );
    assert_eq!(
        execute(&fixture, &parent, &guard).unwrap(),
        GitCheckedPreservationMutation::AlreadyComplete
    );
    assert_eq!(
        observe(&fixture, &marker, &guard),
        GitRootPreservationStepObservation::Before
    );
    assert_eq!(
        execute(&fixture, &marker, &guard).unwrap(),
        GitCheckedPreservationMutation::Applied
    );
    assert_eq!(fs::read(marker_path).unwrap(), b"attached marker\n");
}

#[test]
fn every_allowed_source_equals_goal_step_is_a_nonmutating_after() {
    for format in ["sha1", "sha256"] {
        source_equals_goal_matrix(format);
    }
}

fn source_equals_goal_matrix(format: &str) {
    let fixture = fixture_with_format(format);
    let guard = guard(&prepare(&fixture));
    for object in [
        GitRootManagedObject::MarkerWorktree,
        GitRootManagedObject::LockWorktree,
        GitRootManagedObject::Index,
        GitRootManagedObject::MarkerParentDirectory,
    ] {
        let step = managed_step(
            object,
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::Handoff,
        );
        let before = exact_snapshot(&fixture);
        assert_eq!(
            observe(&fixture, &step, &guard),
            GitRootPreservationStepObservation::After
        );
        assert_eq!(
            execute(&fixture, &step, &guard).unwrap(),
            GitCheckedPreservationMutation::AlreadyComplete
        );
        assert_eq!(exact_snapshot(&fixture), before);
    }
    normalize(&fixture, &guard);
    for object in [
        GitRootManagedObject::MarkerWorktree,
        GitRootManagedObject::LockWorktree,
        GitRootManagedObject::Index,
    ] {
        let step = managed_step(
            object,
            GitRootManagedFormName::AttachedClean,
            GitRootManagedFormName::AttachedClean,
        );
        let before = exact_snapshot(&fixture);
        assert_eq!(
            observe(&fixture, &step, &guard),
            GitRootPreservationStepObservation::After
        );
        assert_eq!(
            execute(&fixture, &step, &guard).unwrap(),
            GitCheckedPreservationMutation::AlreadyComplete
        );
        assert_eq!(exact_snapshot(&fixture), before);
    }
}

fn assert_rejected_without_mutation(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
) {
    assert_eq!(
        observe(fixture, step, guard),
        GitRootPreservationStepObservation::Ambiguous
    );
    let before = exact_snapshot(fixture);
    assert_eq!(
        execute(fixture, step, guard).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
    assert_eq!(exact_snapshot(fixture), before);
}

#[test]
fn reordered_skipped_opposite_and_later_advanced_forms_are_ambiguous() {
    for case in 0..5 {
        let fixture = fixture();
        let guard = guard(&prepare(&fixture));
        let steps = normalize_steps();
        let step = match case {
            0 => steps[2].clone(),
            1 => steps[3].clone(),
            2 => {
                for step in &steps[..2] {
                    execute(&fixture, step, &guard).unwrap();
                }
                git(&fixture.root, &["read-tree", &fixture.spec.attached_commit]);
                steps[2].clone()
            }
            3 => {
                for step in &steps[..2] {
                    execute(&fixture, step, &guard).unwrap();
                }
                fs::write(
                    fixture.root.join(crate::artifact::LOCK_PATH),
                    b"attached lock\n",
                )
                .unwrap();
                steps[1].clone()
            }
            4 => {
                for step in &steps[..2] {
                    execute(&fixture, step, &guard).unwrap();
                }
                fs::write(
                    fixture.root.join(crate::artifact::LOCK_PATH),
                    b"restore lock\n",
                )
                .unwrap();
                steps[2].clone()
            }
            _ => unreachable!(),
        };
        assert_rejected_without_mutation(&fixture, &step, &guard);
    }
}

#[test]
fn wrong_or_detached_ref_and_foreign_native_state_are_ambiguous() {
    for case in ["wrong-ref", "detached", "native"] {
        let fixture = fixture();
        let guard = guard(&prepare(&fixture));
        match case {
            "wrong-ref" => git(&fixture.root, &["checkout", "-q", "-b", "other"]),
            "detached" => git(&fixture.root, &["checkout", "-q", "--detach"]),
            "native" => fs::write(
                fixture.root.join(".git/MERGE_HEAD"),
                format!("{}\n", fixture.spec.attached_commit),
            )
            .unwrap(),
            _ => unreachable!(),
        }
        assert_rejected_without_mutation(&fixture, &normalize_steps()[0], &guard);
        if case == "native" {
            assert!(fixture.root.join(".git/MERGE_HEAD").exists());
        }
    }
}

fn command_bytes(fixture: &RootFixture, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(&fixture.root)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed");
    output.stdout
}

#[test]
fn tree_cache_is_invalidated_while_unrelated_index_semantics_survive() {
    let fixture = fixture();
    fs::write(fixture.root.join("unrelated.txt"), b"staged\n").unwrap();
    fixture
        .backend
        .stage_paths(&fixture.root, &["unrelated.txt"])
        .unwrap();
    fs::write(fixture.root.join("unrelated.txt"), b"unstaged\n").unwrap();
    let tuple = command_bytes(&fixture, &["ls-files", "--stage", "--", "unrelated.txt"]);
    let staged = command_bytes(&fixture, &["show", ":unrelated.txt"]);
    let prepared = prepare(&fixture);
    let guard = guard(&prepared);
    for step in &normalize_steps()[..3] {
        execute(&fixture, step, &guard).unwrap();
    }
    git_output(&fixture.root, &["write-tree"]);
    assert!(
        index_bytes(&fixture)
            .windows(4)
            .any(|bytes| bytes == b"TREE")
    );
    execute(&fixture, &normalize_steps()[3], &guard).unwrap();
    assert_eq!(
        command_bytes(&fixture, &["ls-files", "--stage", "--", "unrelated.txt"]),
        tuple
    );
    assert_eq!(command_bytes(&fixture, &["show", ":unrelated.txt"]), staged);
    assert_eq!(
        fs::read(fixture.root.join("unrelated.txt")).unwrap(),
        b"unstaged\n"
    );
    let after = index_bytes(&fixture);
    if let Some(at) = after.windows(4).rposition(|bytes| bytes == b"TREE") {
        assert!(after[at + 8..].starts_with(b"\0-1 "));
    }
}

#[test]
fn reset_partials_reject_without_changing_refs_index_or_worktree() {
    for case in ["checkout-index-c1", "ref-c1", "index-c1", "worktree-c1"] {
        let fixture = fixture();
        for step in normalize_steps() {
            execute(&fixture, &step, &GitRootPreservationGuard::OtherwiseClean).unwrap();
        }
        match case {
            "checkout-index-c1" => git(
                &fixture.root,
                &["read-tree", "--reset", "-u", &fixture.spec.restore_commit],
            ),
            "ref-c1" => git(
                &fixture.root,
                &[
                    "update-ref",
                    "refs/heads/main",
                    &fixture.spec.restore_commit,
                    &fixture.spec.attached_commit,
                ],
            ),
            "index-c1" => git(&fixture.root, &["read-tree", &fixture.spec.restore_commit]),
            "worktree-c1" => fs::write(
                fixture.root.join(crate::artifact::LOCK_PATH),
                b"restore lock\n",
            )
            .unwrap(),
            _ => unreachable!(),
        }
        assert_rejected_without_mutation(
            &fixture,
            &GitRootPreservationPhysicalStep::ResetAttachedRef,
            &GitRootPreservationGuard::OtherwiseClean,
        );
    }
}
