use super::support::*;
use super::*;

fn observation(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
) -> GitRootPreservationStepObservation {
    fixture
        .backend
        .observe_root_preservation_step(&fixture.root, &fixture.spec, step, guard)
        .unwrap()
}

fn complete(
    fixture: &RootFixture,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
) {
    let before = observation(fixture, step, guard);
    let mutation = fixture
        .backend
        .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, step, guard)
        .unwrap_or_else(|error| panic!("step {step:?}: {error:?}"));
    assert!(
        matches!(
            (before, mutation),
            (
                GitRootPreservationStepObservation::Before,
                GitCheckedPreservationMutation::Applied
                    | GitCheckedPreservationMutation::RefReset(_)
            ) | (
                GitRootPreservationStepObservation::After
                    | GitRootPreservationStepObservation::AfterNeedsDurability,
                GitCheckedPreservationMutation::AlreadyComplete
            )
        ),
        "step: {step:?}"
    );
    let after = observation(fixture, step, guard);
    assert!(matches!(
        after,
        GitRootPreservationStepObservation::After
            | GitRootPreservationStepObservation::AfterNeedsDurability
    ));
    if after == GitRootPreservationStepObservation::AfterNeedsDurability {
        assert_eq!(
            fixture
                .backend
                .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, step, guard,)
                .unwrap(),
            GitCheckedPreservationMutation::AlreadyComplete
        );
    }
}

fn restore_from(source: GitRootManagedFormName) -> [GitRootPreservationPhysicalStep; 4] {
    [
        GitRootManagedObject::Index,
        GitRootManagedObject::LockWorktree,
        GitRootManagedObject::MarkerParentDirectory,
        GitRootManagedObject::MarkerWorktree,
    ]
    .map(|object| managed_step(object, source, GitRootManagedFormName::Handoff))
}

fn assert_prepare_rejects_unchanged(fixture: &RootFixture) {
    let before = exact_snapshot(fixture);
    assert!(
        fixture
            .backend
            .prepare_root_preservation_stash(&fixture.root, &fixture.spec)
            .is_err()
    );
    assert_eq!(exact_snapshot(fixture), before);
}

/// Regression pin for the runner-template class: every leaf the prepare gates
/// observe must be non-executable however the host's git template tree was
/// copied into the fixture (`write_pinned` in `support.rs`). The checked leaf
/// observer classifies an executable file `Invalid` by design, so a 0755
/// `.git/info/exclude` — what the GitHub runner images' template ships — makes
/// `files::observe_boundary` false and every prepare below refuse.
#[cfg(unix)]
#[test]
fn observed_handoff_leaves_are_never_executable_whatever_the_git_template_copied() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = fixture();
    for path in [".git/info/exclude", MARKER, crate::artifact::LOCK_PATH] {
        let mode = fs::metadata(fixture.root.join(path))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0, "{path} is executable (mode {mode:o})");
    }
    prepare(&fixture);
}

#[test]
fn partial_or_stale_preimage_is_ambiguous() {
    let fixture = fixture();
    let prepared = prepare(&fixture);
    let guard = guard(&prepared);
    let steps = normalize_steps();
    fixture
        .backend
        .execute_root_preservation_step_checked(&fixture.root, &fixture.spec, &steps[0], &guard)
        .unwrap();
    fs::write(fixture.root.join("new.txt"), b"late\n").unwrap();
    assert_eq!(
        observation(&fixture, &steps[1], &guard),
        GitRootPreservationStepObservation::Ambiguous
    );
    assert_eq!(
        fixture
            .backend
            .execute_root_preservation_step_checked(
                &fixture.root,
                &fixture.spec,
                &steps[1],
                &guard,
            )
            .unwrap_err()
            .code,
        ErrorCode::PreservationEvidenceMismatch
    );
    assert_eq!(
        fs::read(fixture.root.join(crate::artifact::LOCK_PATH)).unwrap(),
        b"handoff lock\n"
    );
}

#[test]
fn real_sha256_repository_prepares_exact_handoff() {
    let fixture = fixture_with_format("sha256");
    assert_eq!(fixture.spec.attached_commit.len(), 64);
    assert_eq!(
        prepare(&fixture).normalized_image.dirty,
        GitPreservationDirtySummary::default()
    );
}

#[test]
fn marker_presence_matrix_covers_attached_and_restore_clean_forms() {
    for clean_present in [false, true] {
        for handoff_present in [false, true] {
            for empty_parent in [false, true] {
                let attached = clean_present.then_some(b"attached marker\n".as_slice());
                let restore = clean_present.then_some(b"restore marker\n".as_slice());
                let handoff = handoff_present.then_some(b"handoff marker\n".as_slice());
                let fixture = fixture_with_markers("sha1", attached, restore, handoff);
                if empty_parent && !handoff_present {
                    fs::create_dir_all(fixture.root.join(crate::artifact::MARKER_DIR)).unwrap();
                }
                let guard = guard(&prepare(&fixture));
                for step in normalize_steps() {
                    complete(&fixture, &step, &guard);
                }
                complete(
                    &fixture,
                    &GitRootPreservationPhysicalStep::ResetAttachedRef,
                    &GitRootPreservationGuard::OtherwiseClean,
                );
                for step in restore_from(GitRootManagedFormName::RestoreClean) {
                    complete(&fixture, &step, &GitRootPreservationGuard::OtherwiseClean);
                }
            }
        }
    }
}

#[test]
fn parent_observation_distinguishes_optional_established_and_required_empty_forms() {
    use GitRootPreservationStepObservation as O;
    for (source, handoff, parent_form, expected) in [
        (false, false, "missing", O::After),
        (false, false, "empty", O::After),
        (true, false, "established", O::After),
        (true, true, "established", O::After),
        (false, true, "missing", O::Before),
        (false, true, "empty", O::AfterNeedsDurability),
    ] {
        let clean = source.then_some(b"clean marker\n".as_slice());
        let handoff = handoff.then_some(b"handoff marker\n".as_slice());
        let fixture = fixture_with_markers("sha1", clean, clean, handoff);
        if source && handoff.is_none() {
            fs::create_dir_all(fixture.root.join(crate::artifact::MARKER_DIR)).unwrap();
        }
        let guard = guard(&prepare(&fixture));
        for step in normalize_steps() {
            complete(&fixture, &step, &guard);
        }
        let restores = restore_from(GitRootManagedFormName::AttachedClean);
        for step in &restores[..2] {
            complete(&fixture, step, &GitRootPreservationGuard::OtherwiseClean);
        }
        let parent = fixture.root.join(crate::artifact::MARKER_DIR);
        if parent_form == "missing" && parent.is_dir() {
            fs::remove_dir(&parent).unwrap();
        } else if parent_form == "empty" {
            fs::create_dir_all(&parent).unwrap();
        }
        let before = exact_snapshot(&fixture);
        assert_eq!(
            observation(
                &fixture,
                &restores[2],
                &GitRootPreservationGuard::OtherwiseClean,
            ),
            expected,
            "source={source} handoff={} parent={parent_form}",
            handoff.is_some()
        );
        assert_eq!(exact_snapshot(&fixture), before);
    }
}

#[test]
fn source_equals_goal_is_a_write_free_after_for_every_managed_object() {
    let fixture = fixture();
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
            observation(&fixture, &step, &guard),
            GitRootPreservationStepObservation::After
        );
        assert_eq!(
            fixture
                .backend
                .execute_root_preservation_step_checked(
                    &fixture.root,
                    &fixture.spec,
                    &step,
                    &guard,
                )
                .unwrap(),
            GitCheckedPreservationMutation::AlreadyComplete
        );
        assert_eq!(exact_snapshot(&fixture), before);
    }
}

#[test]
fn pre_journal_preparation_rejects_every_non_handoff_managed_mix() {
    for handoff_mask in 0_u8..8 {
        let fixture = fixture();
        if handoff_mask & 4 == 0 {
            git(&fixture.root, &["reset", "-q", "HEAD", "--", "gwz.conf"]);
        }
        if handoff_mask & 1 == 0 {
            fs::remove_file(fixture.root.join(MARKER)).unwrap();
        }
        if handoff_mask & 2 == 0 {
            fs::write(
                fixture.root.join(crate::artifact::LOCK_PATH),
                b"attached lock\n",
            )
            .unwrap();
        }
        if handoff_mask == 7 {
            prepare(&fixture);
        } else {
            assert_prepare_rejects_unchanged(&fixture);
        }
    }
}

#[test]
fn fabricated_stale_missing_and_non_commit_provenance_rejects_without_mutation() {
    for case in ["fabricated-c0", "stale-c1", "missing", "non-commit"] {
        let mut fixture = fixture();
        match case {
            "fabricated-c0" => fixture.spec.attached_clean_form.lock.bytes = b"fake\n".to_vec(),
            "stale-c1" => {
                fixture.spec.restore_clean_form = fixture.spec.attached_clean_form.clone()
            }
            "missing" => fixture.spec.attached_commit = "f".repeat(40),
            "non-commit" => {
                fixture.spec.attached_commit = git_output(
                    &fixture.root,
                    &["hash-object", "-w", crate::artifact::LOCK_PATH],
                )
            }
            _ => unreachable!(),
        }
        assert_prepare_rejects_unchanged(&fixture);
    }
}

#[test]
fn index_versions_two_through_four_are_parsed() {
    for version in [2, 3, 4] {
        let fixture = fixture();
        let value = version.to_string();
        git(&fixture.root, &["update-index", "--index-version", &value]);
        if version == 3 {
            force_v3_header(&fixture);
        }
        let bytes = index_bytes(&fixture);
        assert_eq!(u32::from_be_bytes(bytes[4..8].try_into().unwrap()), version);
        prepare(&fixture);
    }
}

#[test]
fn conflicted_raw_index_rejects_before_any_mutation() {
    use std::io::Write;

    let fixture = fixture();
    let GitRootManagedIndexFact::Present(lock) = &fixture.spec.handoff_form.index.lock else {
        panic!("fixture lock must be present");
    };
    git(
        &fixture.root,
        &["update-index", "--force-remove", crate::artifact::LOCK_PATH],
    );
    let mut child = std::process::Command::new("git")
        .args(["update-index", "--index-info"])
        .current_dir(&fixture.root)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "100644 {} 1\t{}",
        lock.object_id,
        crate::artifact::LOCK_PATH
    )
    .unwrap();
    assert!(child.wait().unwrap().success());
    assert_prepare_rejects_unchanged(&fixture);
}

#[test]
fn unsupported_index_extensions_reject_before_any_mutation() {
    for signature in [
        *b"REUC", *b"NAME", *b"UNTR", *b"FSMN", *b"link", *b"sdir", *b"EOIE", *b"IEOT", *b"abcd",
        *b"ZZZZ",
    ] {
        let fixture = fixture();
        inject_index_extension(&fixture, &signature);
        assert_prepare_rejects_unchanged(&fixture);
    }
}

#[test]
fn checksum_mismatch_and_cross_format_ids_reject() {
    let first_fixture = fixture();
    let path = first_fixture.root.join(".git/index");
    let mut bytes = index_bytes(&first_fixture);
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&path, &bytes).unwrap();
    assert_prepare_rejects_unchanged(&first_fixture);

    let mut fixture = fixture();
    let GitRootManagedIndexFact::Present(entry) = &mut fixture.spec.handoff_form.index.marker
    else {
        panic!("fixture marker must be present");
    };
    entry.object_id = "0".repeat(64);
    assert_prepare_rejects_unchanged(&fixture);
}

#[test]
fn restore_parent_never_adopts_a_different_handoff_stage() {
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
    let foreign = fixture.root.join("gwz.conf/.gwz-markers-different.stage");
    fs::create_dir(&foreign).unwrap();
    assert_eq!(
        observation(
            &fixture,
            &restores[2],
            &GitRootPreservationGuard::OtherwiseClean,
        ),
        GitRootPreservationStepObservation::Ambiguous
    );
    assert_eq!(
        fixture
            .backend
            .execute_root_preservation_step_checked(
                &fixture.root,
                &fixture.spec,
                &restores[2],
                &GitRootPreservationGuard::OtherwiseClean,
            )
            .unwrap_err()
            .code,
        ErrorCode::PreservationEvidenceMismatch
    );
    assert!(foreign.is_dir());
}

#[test]
fn attached_and_restore_provenance_reject_unowned_marker_tree_entries() {
    for commit_kind in ["attached", "restore"] {
        for entry_kind in [
            "second-marker",
            "nested-tree",
            "executable-marker",
            "symlink-lock",
            "tree-lock",
            "marker-not-tree",
        ] {
            let mut fixture = fixture();
            let base = if commit_kind == "attached" {
                fixture.spec.attached_commit.clone()
            } else {
                fixture.spec.restore_commit.clone()
            };
            git(&fixture.root, &["reset", "--hard", &base]);
            let marker_dir = fixture.root.join(crate::artifact::MARKER_DIR);
            fs::create_dir_all(&marker_dir).unwrap();
            match entry_kind {
                "second-marker" => {
                    fs::write(marker_dir.join("other.yaml"), b"other\n").unwrap();
                    git(&fixture.root, &["add", crate::artifact::MARKER_DIR]);
                }
                "nested-tree" => {
                    fs::create_dir(marker_dir.join("nested")).unwrap();
                    fs::write(marker_dir.join("nested/other.yaml"), b"other\n").unwrap();
                    git(&fixture.root, &["add", crate::artifact::MARKER_DIR]);
                }
                "executable-marker" | "symlink-lock" => {
                    let input = fixture.root.join(".git/gwz-mode-blob");
                    fs::write(&input, b"mode\n").unwrap();
                    let object =
                        git_output(&fixture.root, &["hash-object", "-w", ".git/gwz-mode-blob"]);
                    let (mode, path) = if entry_kind == "executable-marker" {
                        ("100755", MARKER)
                    } else {
                        ("120000", crate::artifact::LOCK_PATH)
                    };
                    let cache_info = format!("{mode},{object},{path}");
                    git(
                        &fixture.root,
                        &["update-index", "--add", "--cacheinfo", &cache_info],
                    );
                }
                "tree-lock" => {
                    fs::remove_file(fixture.root.join(crate::artifact::LOCK_PATH)).unwrap();
                    fs::create_dir(fixture.root.join(crate::artifact::LOCK_PATH)).unwrap();
                    fs::write(
                        fixture.root.join(crate::artifact::LOCK_PATH).join("child"),
                        b"tree\n",
                    )
                    .unwrap();
                    git(&fixture.root, &["add", "-A", crate::artifact::LOCK_PATH]);
                }
                "marker-not-tree" => {
                    fs::remove_dir(&marker_dir).unwrap();
                    fs::write(&marker_dir, b"not a tree\n").unwrap();
                    git(&fixture.root, &["add", "-A", crate::artifact::MARKER_DIR]);
                }
                _ => unreachable!(),
            }
            git(
                &fixture.root,
                &[
                    "-c",
                    "user.name=GWZ Test",
                    "-c",
                    "user.email=gwz@example.invalid",
                    "commit",
                    "-q",
                    "-m",
                    "invalid marker provenance",
                ],
            );
            let commit = git_output(&fixture.root, &["rev-parse", "HEAD"]);
            if commit_kind == "attached" {
                fixture.spec.attached_commit = commit;
            } else {
                fixture.spec.restore_commit = commit;
            }
            let error = fixture
                .backend
                .prepare_root_preservation_stash(&fixture.root, &fixture.spec)
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::PreservationEvidenceMismatch);
            let expected = match entry_kind {
                "second-marker" | "nested-tree" => "marker directory contains an unexpected entry",
                "marker-not-tree" => "marker directory is not a tree",
                _ => "regular non-executable file",
            };
            assert!(
                error.message.contains(expected),
                "{commit_kind}/{entry_kind}: {}",
                error.message
            );
        }
    }
}
