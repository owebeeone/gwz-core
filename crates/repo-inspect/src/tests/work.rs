//! Architecture §4's work-detector test list, against real repositories:
//! "Use known physical file bytes as an oracle instead of asking status to
//! validate itself."

use git2::ObjectFormat;
use gwz_repo_contract::{
    NativeOperation, Observation, PhysicalState, RepoInspector, SuppressionFlag, UnknownKind,
    WorkKind, WorkObservation,
};

use super::admitted;
use crate::LocalRepoInspector;
use crate::fixtures::Fixture;

/// Git's on-disk index flag bits, spelled here independently of the
/// implementation's own constants so the test does not agree with a typo.
const ASSUME_VALID: u16 = 0x8000;
const SKIP_WORKTREE: u16 = 1 << 14;

fn observe(fixture: &Fixture) -> WorkObservation {
    let info = admitted(fixture.root());
    match LocalRepoInspector::new().observe_work(&info) {
        Observation::Known(observation) => observation,
        Observation::Unknown(reasons) => panic!("expected a known observation, got {reasons:?}"),
    }
}

fn kinds_for(observation: &WorkObservation, path: &str) -> Vec<WorkKind> {
    observation
        .entries
        .iter()
        .filter(|entry| entry.path == path.as_bytes())
        .map(|entry| entry.kind)
        .collect()
}

/// What an ordinary `git status` would say about `path`, used only to show
/// that the contract is *not* satisfied by asking status.
fn status_of(fixture: &Fixture, path: &str) -> git2::Status {
    fixture
        .open()
        .status_file(std::path::Path::new(path))
        .expect("status")
}

#[test]
fn staged_and_unstaged_versions_of_the_same_file_are_both_reported() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("a.txt", b"one\n");
    fixture.commit("a");
    fixture.write("a.txt", b"two\n");
    fixture.stage_all();
    fixture.write("a.txt", b"three\n");

    let observation = observe(&fixture);
    let kinds = kinds_for(&observation, "a.txt");
    assert!(kinds.contains(&WorkKind::Staged), "{kinds:?}");
    assert!(kinds.contains(&WorkKind::Unstaged), "{kinds:?}");
}

#[test]
fn a_binary_edit_is_reported_as_binary_and_a_text_edit_is_not() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("image.bin", b"\x89PNG\x00\x01\x02binary");
    fixture.write("notes.txt", b"plain text\n");

    let observation = observe(&fixture);
    let binary_of = |path: &str| {
        observation
            .entries
            .iter()
            .find(|entry| entry.path == path.as_bytes())
            .unwrap_or_else(|| panic!("{path} is reported"))
            .binary
    };
    assert_eq!(binary_of("image.bin"), Some(true));
    assert_eq!(binary_of("notes.txt"), Some(false));
}

#[test]
fn deletions_and_renames_are_distinguished_from_ordinary_edits() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("gone.txt", b"content\n");
    fixture.write(
        "old-name.txt",
        b"a distinctive body that survives the rename\n",
    );
    fixture.commit("setup");

    fixture.remove("gone.txt");
    fixture.remove("old-name.txt");
    fixture.stage_removal("old-name.txt");
    fixture.write(
        "new-name.txt",
        b"a distinctive body that survives the rename\n",
    );
    fixture.stage_all();

    let observation = observe(&fixture);
    assert!(
        kinds_for(&observation, "gone.txt").contains(&WorkKind::Deleted),
        "{:?}",
        observation.entries
    );
    let renamed = observation
        .entries
        .iter()
        .any(|entry| entry.kind == WorkKind::Renamed);
    assert!(renamed, "{:?}", observation.entries);
}

#[cfg(unix)]
#[test]
fn a_mode_change_is_reported_even_though_the_content_is_identical() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("script.sh", b"#!/bin/sh\necho hi\n");
    fixture.commit("script");
    fixture.set_mode("script.sh", 0o755);

    let observation = observe(&fixture);
    let kinds = kinds_for(&observation, "script.sh");
    assert!(kinds.contains(&WorkKind::ModeChange), "{kinds:?}");
}

#[cfg(unix)]
#[test]
fn replacing_a_file_with_a_symlink_is_reported_as_a_link_change() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("target.txt", b"target\n");
    fixture.write("link.txt", b"was a file\n");
    fixture.commit("setup");
    fixture.remove("link.txt");
    crate::fixtures::symlink(
        std::path::Path::new("target.txt"),
        &fixture.real_root().join("link.txt"),
    );

    let observation = observe(&fixture);
    let kinds = kinds_for(&observation, "link.txt");
    assert!(kinds.contains(&WorkKind::LinkChange), "{kinds:?}");
}

#[test]
fn ignored_user_data_and_an_ignored_nested_git_directory_are_reported() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write(".gitignore", b"build/\nvendor/\n");
    fixture.commit("ignore rules");
    fixture.write("build/artifact.o", b"\x00\x01object");
    // An unmanaged nested repository inside ignored data: "ignored does not
    // mean disposable" (design §5.1).
    crate::fixtures::nested_repository(&fixture.mkdir("vendor/library"));

    let observation = observe(&fixture);
    let ignored: Vec<_> = observation
        .entries
        .iter()
        .filter(|entry| entry.kind == WorkKind::Ignored)
        .map(|entry| String::from_utf8_lossy(&entry.path).into_owned())
        .collect();
    assert!(
        ignored.iter().any(|path| path.starts_with("build")),
        "{ignored:?}"
    );
    assert!(
        ignored.iter().any(|path| path.starts_with("vendor")),
        "{ignored:?}"
    );
}

#[test]
fn an_edited_assume_unchanged_path_is_observed_physically_and_never_as_clean() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("watched.txt", b"committed\n");
    fixture.commit("watched");
    fixture.set_index_flags("watched.txt", ASSUME_VALID, 0);
    fixture.write("watched.txt", b"edited on disk\n");

    // Status is not the oracle: it reports this path as clean.
    assert!(
        status_of(&fixture, "watched.txt").is_empty(),
        "the fixture must be one status would call clean"
    );

    let observation = observe(&fixture);
    let entry = observation
        .suppressed
        .iter()
        .find(|entry| entry.path == b"watched.txt")
        .unwrap_or_else(|| panic!("{:?}", observation.suppressed));
    assert_eq!(entry.flag, SuppressionFlag::AssumeUnchanged);
    assert_eq!(entry.physical, PhysicalState::Differs);
}

#[test]
fn an_unedited_assume_unchanged_path_matches_the_index() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("watched.txt", b"committed\n");
    fixture.commit("watched");
    fixture.set_index_flags("watched.txt", ASSUME_VALID, 0);

    let observation = observe(&fixture);
    let entry = observation
        .suppressed
        .iter()
        .find(|entry| entry.path == b"watched.txt")
        .unwrap_or_else(|| panic!("{:?}", observation.suppressed));
    assert_eq!(entry.physical, PhysicalState::MatchesIndex);
}

#[test]
fn a_present_skip_worktree_path_is_observed_physically() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("sparse.txt", b"committed\n");
    fixture.commit("sparse");
    fixture.set_index_flags("sparse.txt", 0, SKIP_WORKTREE);
    fixture.write("sparse.txt", b"present and edited\n");

    let observation = observe(&fixture);
    let entry = observation
        .suppressed
        .iter()
        .find(|entry| entry.path == b"sparse.txt")
        .unwrap_or_else(|| panic!("{:?}", observation.suppressed));
    assert_eq!(entry.flag, SuppressionFlag::SkipWorktree);
    assert_eq!(entry.physical, PhysicalState::Differs);
    assert!(observation.sparse_absent.is_empty());
}

#[test]
fn a_valid_sparse_absence_is_recorded_separately_from_a_missing_suppressed_path() {
    let sparse = Fixture::checkout(ObjectFormat::Sha1);
    sparse.write("out-of-cone.txt", b"committed\n");
    sparse.commit("cone");
    sparse.set_index_flags("out-of-cone.txt", 0, SKIP_WORKTREE);
    sparse.remove("out-of-cone.txt");
    sparse.append_config("[core]\n\tsparseCheckout = true\n");

    let observation = observe(&sparse);
    assert_eq!(observation.sparse_absent, vec![b"out-of-cone.txt".to_vec()]);
    assert!(
        observation.suppressed.is_empty(),
        "{:?}",
        observation.suppressed
    );

    // The same absence without a sparse checkout is a suppressed path whose
    // bytes are missing, not a valid absence.
    let plain = Fixture::checkout(ObjectFormat::Sha1);
    plain.write("out-of-cone.txt", b"committed\n");
    plain.commit("cone");
    plain.set_index_flags("out-of-cone.txt", 0, SKIP_WORKTREE);
    plain.remove("out-of-cone.txt");

    let observation = observe(&plain);
    assert!(observation.sparse_absent.is_empty());
    assert_eq!(
        observation
            .suppressed
            .iter()
            .map(|entry| entry.physical)
            .collect::<Vec<_>>(),
        vec![PhysicalState::Absent]
    );
}

#[cfg(unix)]
#[test]
fn a_suppressed_path_whose_bytes_cannot_be_read_is_unknown_never_clean() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("secret/keep.txt", b"committed\n");
    fixture.commit("secret");
    fixture.set_index_flags("secret/keep.txt", ASSUME_VALID, 0);
    fixture.set_mode("secret", 0o000);

    let unreadable = std::fs::read(fixture.real_root().join("secret/keep.txt")).is_err();
    // Restore the mode before any assertion so the temporary directory can
    // still be removed, whatever the outcome.
    let observation = observe(&fixture);
    fixture.set_mode("secret", 0o755);
    if !unreadable {
        // The effective user bypasses the mode (a root test runner); there is
        // nothing unreadable to observe.
        return;
    }

    let entry = observation
        .suppressed
        .iter()
        .find(|entry| entry.path == b"secret/keep.txt")
        .unwrap_or_else(|| panic!("{:?}", observation.suppressed));
    assert_eq!(entry.physical, PhysicalState::Unobservable);
    assert!(
        observation
            .unknown
            .iter()
            .any(|reason| reason.kind == UnknownKind::Unreadable
                && reason.path.as_deref() == Some(b"secret/keep.txt".as_slice())),
        "{:?}",
        observation.unknown
    );
}

#[test]
fn an_unfinished_native_operation_is_reported() {
    for (file, expected) in [
        ("MERGE_HEAD", NativeOperation::Merge),
        ("CHERRY_PICK_HEAD", NativeOperation::CherryPick),
        ("REVERT_HEAD", NativeOperation::Revert),
    ] {
        let fixture = Fixture::checkout(ObjectFormat::Sha1);
        fixture.begin_native_operation(file);
        assert_eq!(observe(&fixture).native_operation, Some(expected), "{file}");
    }
    let clean = Fixture::checkout(ObjectFormat::Sha1);
    assert_eq!(observe(&clean).native_operation, None);
}

#[test]
fn a_conflicted_merge_reports_conflict_stages_and_the_open_operation() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.conflicting_merge("shared.txt");

    let observation = observe(&fixture);
    assert_eq!(observation.native_operation, Some(NativeOperation::Merge));
    assert!(
        kinds_for(&observation, "shared.txt").contains(&WorkKind::Conflict),
        "{:?}",
        observation.entries
    );
}

#[test]
fn native_stash_entries_are_counted_including_older_ones() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    assert_eq!(observe(&fixture).stash_entries, 0);

    fixture.write("tracked.txt", b"first change\n");
    fixture.stash("first");
    fixture.write("tracked.txt", b"second change\n");
    fixture.stash("second");

    assert_eq!(observe(&fixture).stash_entries, 2);
}

#[test]
fn a_bare_repository_observes_operation_state_and_no_worktree_dirt() {
    let fixture = Fixture::bare(ObjectFormat::Sha1);
    let observation = observe(&fixture);
    assert!(observation.entries.is_empty());
    assert!(observation.suppressed.is_empty());
    assert!(observation.sparse_absent.is_empty());
    assert!(observation.unknown.is_empty());
    assert_eq!(observation.native_operation, None);
}

#[test]
fn a_sha256_repository_is_observed_physically_with_its_own_digest() {
    let fixture = Fixture::checkout(ObjectFormat::Sha256);
    fixture.write("watched.txt", b"committed\n");
    fixture.commit("watched");
    fixture.set_index_flags("watched.txt", ASSUME_VALID, 0);

    let matching = observe(&fixture);
    assert_eq!(
        matching.suppressed.first().map(|entry| entry.physical),
        Some(PhysicalState::MatchesIndex),
        "{:?}",
        matching.suppressed
    );

    fixture.write("watched.txt", b"edited on disk\n");
    let differing = observe(&fixture);
    assert_eq!(
        differing.suppressed.first().map(|entry| entry.physical),
        Some(PhysicalState::Differs)
    );
}

#[test]
fn a_clean_repository_reports_nothing_and_repeats_itself() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let first = observe(&fixture);
    assert_eq!(first, WorkObservation::default());
    assert_eq!(observe(&fixture), first);
}
