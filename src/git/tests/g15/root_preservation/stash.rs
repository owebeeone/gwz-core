use super::support::*;
use super::*;

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
enum LateImageChange {
    Staged,
    Unstaged,
    Untracked,
    #[cfg(all(unix, not(target_os = "macos")))]
    RawNonUtf8,
}

#[derive(Clone, Copy, Debug)]
enum LateControlChange {
    Marker,
    Lock,
    Index,
    Boundary,
}

fn create_stash_step() -> GitRootPreservationPhysicalStep {
    GitRootPreservationPhysicalStep::CreateStash {
        merge_id: "merge_1".into(),
    }
}

fn path_bytes(root: &Path, paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
    paths
        .iter()
        .map(|path| match fs::read(root.join(path)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("failed to read {path:?}: {error}"),
        })
        .collect()
}

fn assert_stash_rejected_without_mutation(
    fixture: &RootFixture,
    guard: &GitRootPreservationGuard,
    paths: &[PathBuf],
) {
    let step = create_stash_step();
    let exact = exact_snapshot(fixture);
    let worktree = path_bytes(&fixture.root, paths);
    assert_eq!(
        fixture
            .backend
            .observe_root_preservation_step(&fixture.root, &fixture.spec, &step, guard)
            .unwrap(),
        GitRootPreservationStepObservation::Ambiguous
    );
    assert_eq!(
        fixture
            .backend
            .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, &step, guard,)
            .unwrap_err()
            .code,
        ErrorCode::PreservationEvidenceMismatch
    );
    assert_eq!(exact_snapshot(fixture), exact);
    assert_eq!(path_bytes(&fixture.root, paths), worktree);
    assert!(
        fixture
            .backend
            .stash_list(&fixture.root)
            .unwrap()
            .is_empty()
    );
}

fn assert_raw_index_rejected(fixture: &RootFixture, guard: &GitRootPreservationGuard) {
    let before = exact_snapshot(fixture);
    let step = &normalize_steps()[0];
    assert_eq!(
        fixture
            .backend
            .prepare_root_preservation_stash(&fixture.root, &fixture.spec)
            .unwrap_err()
            .code,
        ErrorCode::PreservationEvidenceMismatch
    );
    assert!(match fixture.backend.observe_root_preservation_step(
        &fixture.root,
        &fixture.spec,
        step,
        guard,
    ) {
        Ok(GitRootPreservationStepObservation::Ambiguous) => true,
        Err(error) => error.code == ErrorCode::PreservationEvidenceMismatch,
        _ => false,
    });
    assert_eq!(
        fixture
            .backend
            .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, step, guard)
            .unwrap_err()
            .code,
        ErrorCode::PreservationEvidenceMismatch
    );
    assert_eq!(exact_snapshot(fixture), before);
}

#[test]
fn index_only_marker_and_semantic_flags_reject_before_mutation() {
    for case in [
        "index-only-marker",
        "assume-valid",
        "skip-worktree",
        "intent-to-add",
        "unknown-extended",
    ] {
        let fixture = fixture();
        let guard = guard(&prepare(&fixture));
        let extra = fixture.root.join("index-intent.txt");
        match case {
            "index-only-marker" => {
                let other = fixture.root.join("gwz.conf/markers/other.yaml");
                fs::write(&other, b"other\n").unwrap();
                git(&fixture.root, &["add", "--", "gwz.conf/markers/other.yaml"]);
                fs::remove_file(other).unwrap();
            }
            "assume-valid" | "skip-worktree" => {
                let flag = if case == "assume-valid" {
                    "--assume-unchanged"
                } else {
                    "--skip-worktree"
                };
                git(
                    &fixture.root,
                    &["update-index", flag, crate::artifact::LOCK_PATH],
                );
            }
            "intent-to-add" => {
                fs::write(&extra, b"intent\n").unwrap();
                git(&fixture.root, &["add", "-N", "--", "index-intent.txt"]);
            }
            "unknown-extended" => inject_unknown_extended_flag(&fixture),
            _ => unreachable!(),
        }
        assert_raw_index_rejected(&fixture, &guard);
        if case == "index-only-marker" {
            assert!(!fixture.root.join("gwz.conf/markers/other.yaml").exists());
        }
        if case == "intent-to-add" {
            assert_eq!(fs::read(extra).unwrap(), b"intent\n");
        }
    }
}

#[test]
fn checked_stash_rechecks_preimage_and_preserves_ignored_work() {
    for format in ["sha1", "sha256"] {
        checked_stash_round_trip(format);
    }
}

fn checked_stash_round_trip(format: &str) {
    let fixture = fixture_with_format(format);
    fs::create_dir_all(fixture.root.join("ignored")).unwrap();
    fs::write(fixture.root.join("ignored/keep.txt"), b"ignored\n").unwrap();
    fs::write(fixture.root.join("user.txt"), b"user\n").unwrap();
    let prepared = prepare(&fixture);
    let guard = guard(&prepared);
    normalize(&fixture, &guard);
    let step = GitRootPreservationPhysicalStep::CreateStash {
        merge_id: "merge_1".into(),
    };
    assert_eq!(
        fixture
            .backend
            .observe_root_preservation_step(&fixture.root, &fixture.spec, &step, &guard)
            .unwrap(),
        GitRootPreservationStepObservation::Before
    );
    assert!(matches!(
        fixture
            .backend
            .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, &step, &guard,)
            .unwrap(),
        GitCheckedPreservationMutation::StashCreated(_)
    ));
    assert_eq!(
        fixture
            .backend
            .observe_root_preservation_step(&fixture.root, &fixture.spec, &step, &guard)
            .unwrap(),
        GitRootPreservationStepObservation::After
    );
    let completed = exact_snapshot(&fixture);
    assert_eq!(
        fixture
            .backend
            .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, &step, &guard)
            .unwrap(),
        GitCheckedPreservationMutation::AlreadyComplete
    );
    assert_eq!(exact_snapshot(&fixture), completed);
    assert_eq!(
        fs::read(fixture.root.join("ignored/keep.txt")).unwrap(),
        b"ignored\n"
    );
    assert!(!fixture.root.join("user.txt").exists());
    assert_eq!(
        fixture
            .backend
            .preservation_stashes(&fixture.root, "merge_1")
            .unwrap()[0]
            .image,
        prepared.normalized_image
    );
    assert!(
        fixture
            .backend
            .checkout_matches_commit(
                &fixture.root,
                &fixture.spec.attached_branch,
                &fixture.spec.attached_commit,
            )
            .unwrap()
    );
    assert!(
        fixture
            .backend
            .index_matches_candidate_files(
                &fixture.root,
                std::slice::from_ref(&fixture.spec.attached_clean_form.lock),
                &[MARKER.into()],
            )
            .unwrap()
    );
    assert_eq!(
        fixture
            .backend
            .preservation_image(&fixture.root, true)
            .unwrap()
            .dirty,
        GitPreservationDirtySummary::default()
    );
    for restore in restore_steps() {
        let parent = matches!(
            restore,
            GitRootPreservationPhysicalStep::Managed(GitRootManagedTransition {
                object: GitRootManagedObject::MarkerParentDirectory,
                ..
            })
        );
        let expected = GitRootPreservationStepObservation::Before;
        assert_eq!(
            fixture
                .backend
                .observe_root_preservation_step(
                    &fixture.root,
                    &fixture.spec,
                    &restore,
                    &GitRootPreservationGuard::OtherwiseClean,
                )
                .unwrap(),
            expected,
            "restore step: {restore:?}"
        );
        let mutation = fixture
            .backend
            .execute_root_preservation_step_checked(
                &fixture.root,
                &fixture.spec,
                &restore,
                &GitRootPreservationGuard::OtherwiseClean,
            )
            .unwrap();
        assert_eq!(mutation, GitCheckedPreservationMutation::Applied);
        if parent {
            assert_eq!(
                fixture
                    .backend
                    .execute_root_preservation_step_checked(
                        &fixture.root,
                        &fixture.spec,
                        &restore,
                        &GitRootPreservationGuard::OtherwiseClean,
                    )
                    .unwrap(),
                GitCheckedPreservationMutation::AlreadyComplete
            );
        }
    }
    assert_eq!(
        fs::read(fixture.root.join(MARKER)).unwrap(),
        b"handoff marker\n"
    );
}

#[test]
fn create_stash_remains_exact_over_checked_artifact_private_residue() {
    let fixture = fixture();
    fs::write(fixture.root.join("user.txt"), b"user\n").unwrap();
    let prepared = prepare(&fixture);
    let guard = guard(&prepared);
    normalize(&fixture, &guard);
    // Crash-retained checked-artifact private residue in the exact shape of
    // the Windows durability anchor. The private area is invisible to the
    // preservation-image model everywhere, so its presence must not change
    // the exact stash evidence even though the native stash sweeps it.
    let private = fixture.root.join(".gwz/checked-artifacts");
    fs::create_dir_all(&private).unwrap();
    fs::write(
        private.join(".ca1-durability-anchor-deadbeefdeadbeefdeadbeefdeadbeef"),
        b"GWZ-CHECKED-ARTIFACT-DURABILITY-ANCHOR-V1\n",
    )
    .unwrap();
    // R2-F R1.1, 2026-09-01: the catalog's own directory is blind on its own
    // ground and must be proved alongside the legacy one. This fixture passes
    // `excluded_worktree_paths = Vec::new()` (`support.rs:133`), so the
    // blindness rests SOLELY on the two constants.
    let catalog = fixture.root.join(".gwz/catalog-final");
    fs::create_dir_all(&catalog).unwrap();
    fs::write(catalog.join("catalog-format"), b"GWZ-CATALOG-FORMAT-V1\n").unwrap();
    let step = create_stash_step();
    assert_eq!(
        fixture
            .backend
            .observe_root_preservation_step(&fixture.root, &fixture.spec, &step, &guard)
            .unwrap(),
        GitRootPreservationStepObservation::Before
    );
    assert!(matches!(
        fixture
            .backend
            .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, &step, &guard,)
            .unwrap(),
        GitCheckedPreservationMutation::StashCreated(_)
    ));
    assert_eq!(
        fixture
            .backend
            .observe_root_preservation_step(&fixture.root, &fixture.spec, &step, &guard)
            .unwrap(),
        GitRootPreservationStepObservation::After
    );
    // The decoded stash image is anchor-blind and equals the durable preimage.
    assert_eq!(
        fixture
            .backend
            .preservation_stashes(&fixture.root, "merge_1")
            .unwrap()[0]
            .image,
        prepared.normalized_image
    );
}

#[test]
fn late_included_image_changes_reject_without_mutation() {
    let cases = vec![
        LateImageChange::Staged,
        LateImageChange::Unstaged,
        LateImageChange::Untracked,
    ];
    #[cfg(all(unix, not(target_os = "macos")))]
    let cases = {
        let mut cases = cases;
        cases.push(LateImageChange::RawNonUtf8);
        cases
    };

    for case in cases {
        let fixture = fixture();
        fs::write(fixture.root.join("kept.txt"), b"kept\n").unwrap();
        if matches!(case, LateImageChange::Unstaged) {
            fs::write(fixture.root.join("late.txt"), b"index\n").unwrap();
            git(&fixture.root, &["add", "--", "late.txt"]);
        }
        let prepared = prepare(&fixture);
        let guard = guard(&prepared);
        normalize(&fixture, &guard);
        let changed = match case {
            LateImageChange::Staged => {
                fs::write(fixture.root.join("late.txt"), b"late staged\n").unwrap();
                git(&fixture.root, &["add", "--", "late.txt"]);
                PathBuf::from("late.txt")
            }
            LateImageChange::Unstaged => {
                fs::write(fixture.root.join("late.txt"), b"late unstaged\n").unwrap();
                PathBuf::from("late.txt")
            }
            LateImageChange::Untracked => {
                fs::write(fixture.root.join("late.txt"), b"late untracked\n").unwrap();
                PathBuf::from("late.txt")
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            LateImageChange::RawNonUtf8 => {
                use std::ffi::OsString;
                use std::os::unix::ffi::OsStringExt;

                let path = PathBuf::from(OsString::from_vec(b"late-\x80".to_vec()));
                fs::write(fixture.root.join(&path), b"late raw\n").unwrap();
                path
            }
        };
        assert_stash_rejected_without_mutation(
            &fixture,
            &guard,
            &[PathBuf::from("kept.txt"), changed],
        );
    }
}

#[test]
fn late_managed_index_and_boundary_changes_reject_without_mutation() {
    for case in [
        LateControlChange::Marker,
        LateControlChange::Lock,
        LateControlChange::Index,
        LateControlChange::Boundary,
    ] {
        let fixture = fixture();
        fs::write(fixture.root.join("kept.txt"), b"kept\n").unwrap();
        let prepared = prepare(&fixture);
        let guard = guard(&prepared);
        normalize(&fixture, &guard);
        let changed = match case {
            LateControlChange::Marker => {
                fs::write(fixture.root.join(MARKER), b"late marker\n").unwrap();
                PathBuf::from(MARKER)
            }
            LateControlChange::Lock => {
                fs::write(
                    fixture.root.join(crate::artifact::LOCK_PATH),
                    b"late lock\n",
                )
                .unwrap();
                PathBuf::from(crate::artifact::LOCK_PATH)
            }
            LateControlChange::Index => {
                fs::write(fixture.root.join(MARKER), b"handoff marker\n").unwrap();
                git(&fixture.root, &["add", "--", MARKER]);
                fs::remove_file(fixture.root.join(MARKER)).unwrap();
                PathBuf::from(".git/index")
            }
            LateControlChange::Boundary => {
                fs::write(fixture.root.join(".git/info/exclude"), b"other/\n").unwrap();
                PathBuf::from(".git/info/exclude")
            }
        };
        assert_stash_rejected_without_mutation(
            &fixture,
            &guard,
            &[PathBuf::from("kept.txt"), changed],
        );
    }
}

#[test]
fn checked_stash_preserves_all_eligible_work_and_excludes_late_ignored_work() {
    let fixture = fixture();
    fs::write(fixture.root.join("staged.txt"), b"index version\n").unwrap();
    git(&fixture.root, &["add", "--", "staged.txt"]);
    fs::write(fixture.root.join("staged.txt"), b"worktree version\n").unwrap();
    fs::write(fixture.root.join("untracked.txt"), b"untracked\n").unwrap();
    fs::create_dir_all(fixture.root.join("ignored")).unwrap();
    fs::write(fixture.root.join("ignored/keep.txt"), b"ignored before\n").unwrap();
    let prepared = prepare(&fixture);
    let guard = guard(&prepared);
    normalize(&fixture, &guard);

    fs::write(fixture.root.join("ignored/keep.txt"), b"ignored after\n").unwrap();
    assert_eq!(
        fixture
            .backend
            .preservation_image(&fixture.root, true)
            .unwrap(),
        prepared.normalized_image
    );
    assert!(matches!(
        fixture
            .backend
            .execute_root_preservation_step_checked(
                &fixture.root,
                &fixture.spec,
                &create_stash_step(),
                &guard,
            )
            .unwrap(),
        GitCheckedPreservationMutation::StashCreated(_)
    ));
    assert!(!fixture.root.join("staged.txt").exists());
    assert!(!fixture.root.join("untracked.txt").exists());
    assert_eq!(
        fs::read(fixture.root.join("ignored/keep.txt")).unwrap(),
        b"ignored after\n"
    );
    let stashes = fixture
        .backend
        .preservation_stashes(&fixture.root, "merge_1")
        .unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].image, prepared.normalized_image);
    assert_eq!(
        stashes[0].image.dirty,
        GitPreservationDirtySummary {
            staged: true,
            unstaged: true,
            untracked: true,
        }
    );
}

// Darwin filesystems reject malformed UTF-8 path bytes with EILSEQ; Unix
// targets that admit such names exercise the real raw-path boundary.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn exact_handoff_boundary_controls_raw_ignored_or_untracked_membership() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let fixture = fixture();
    let raw = PathBuf::from("ignored").join(OsString::from_vec(b"raw-\x80".to_vec()));
    fs::create_dir_all(fixture.root.join("ignored")).unwrap();
    fs::write(fixture.root.join(&raw), b"raw\n").unwrap();
    let prepared = prepare(&fixture);
    assert_eq!(
        prepared.normalized_image.dirty,
        GitPreservationDirtySummary::default()
    );
    let guard = guard(&prepared);
    normalize(&fixture, &guard);
    assert_eq!(
        fixture
            .backend
            .preservation_image(&fixture.root, true)
            .unwrap()
            .dirty,
        GitPreservationDirtySummary::default()
    );

    fs::write(fixture.root.join(".git/info/exclude"), b"elsewhere/\n").unwrap();
    assert!(
        fixture
            .backend
            .preservation_image(&fixture.root, true)
            .unwrap()
            .dirty
            .untracked
    );
    assert_stash_rejected_without_mutation(
        &fixture,
        &guard,
        &[raw, PathBuf::from(".git/info/exclude")],
    );
}
