use std::fs;

use crate::model::ErrorCode;

use super::*;

// GwzStashBranchPlan S2: GitBackend native stash primitives only.

#[test]
fn stash_push_tracked_only_changes_and_leaves_worktree_clean() {
    let temp = TempDir::new("stash-tracked");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "tracked.txt", "base\n", "base", &[]).unwrap();

    fs::write(repo.join("tracked.txt"), "changed\n").unwrap();
    let result = backend
        .stash_push(
            &repo,
            "gwz:stash_tracked: tracked",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();

    assert!(!result.object_id.is_empty());
    assert_text_eq(repo.join("tracked.txt"), "base\n");
    assert_eq!(backend.status(&repo).unwrap(), GitStatus::clean());
}

/// The SECOND tripwired exposure, closed for gwz-born repositories.
///
/// libgit2's `git_stash_save` resets the swept tracked files through its own
/// internal checkout **with filters active**, and exposes no checkout options
/// to turn them off (`GwzM5-8ExactEvidencePlatformAmendment.md`, native-stash
/// residual; `GwzWindowsMatrix-Classification.md` standing residual tripwire
/// exposure (1)). On a filter-active worktree that reset re-materializes CRLF,
/// so every preservation-side raw-byte preimage equality afterwards is
/// unsatisfiable — the class run 8 measured in the un-pinned member fixtures.
///
/// Decision 1 Option B closes it without touching the stash code at all: the
/// birth-time pins leave `stash_save`'s internal reset with no filter to run,
/// so the sweep is a byte no-op. This test proves that, and proves it
/// non-vacuously — the CONTROL arm is an adopted-style repo where the same
/// sweep DOES smudge, so the subject assertion cannot pass merely because the
/// environment has no live smudge source.
#[test]
fn stash_round_trip_on_a_born_repo_is_a_filter_no_op() {
    let temp = TempDir::new("stash-filter-no-op");
    let backend = Git2Backend::new();
    let blob = b"line1\nline2\n";
    let dirty = "line1\nline2\nline3\n";

    // CONTROL: adopted-style repo (repo-local `core.autocrlf=true`). The
    // stash sweep resets `swept.txt` to HEAD through the active smudge
    // filter, so it lands CRLF — the exposure, live.
    let control = temp.path().join("control");
    init_adopted_autocrlf_repo(&control);
    commit_file(&control, "swept.txt", "line1\nline2\n", "seed", &[]).unwrap();
    fs::write(control.join("swept.txt"), dirty).unwrap();
    backend
        .stash_push(
            &control,
            "gwz:stash_crlf: control",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();
    assert_eq!(
        fs::read(control.join("swept.txt")).unwrap(),
        b"line1\r\nline2\r\n",
        "control: libgit2 stash_save's internal reset must re-materialize the \
         swept file through the smudge filter — without this the subject \
         assertion below is vacuous"
    );

    // SUBJECT: a gwz-born repository. Same sweep, same hostile filter intent,
    // but the birth pins make the internal reset a filter no-op.
    let subject = temp.path().join("subject");
    backend.create_repo(&subject).unwrap();
    commit_file(&subject, "swept.txt", "line1\nline2\n", "seed", &[]).unwrap();
    fs::write(subject.join("swept.txt"), dirty).unwrap();
    let pushed = backend
        .stash_push(
            &subject,
            "gwz:stash_crlf: subject",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();
    assert_eq!(
        fs::read(subject.join("swept.txt")).unwrap(),
        blob,
        "gwz-born: the stash sweep's internal reset must be a filter no-op — \
         the swept file stays blob-exact, so preservation-side raw-byte \
         preimage equalities remain satisfiable"
    );

    // ...and the return leg too: popping restores the stashed bytes verbatim,
    // so the whole round trip is byte-identity, not just the sweep.
    backend
        .stash_pop(
            &subject,
            &GitStashTarget::object_id(pushed.object_id.clone()),
            GitStashRestoreOptions::default(),
        )
        .unwrap();
    assert_eq!(
        fs::read(subject.join("swept.txt")).unwrap(),
        dirty.as_bytes(),
        "gwz-born: the stash pop must restore the stashed worktree bytes verbatim"
    );
    assert!(
        !backend
            .stash_list(&subject)
            .unwrap()
            .iter()
            .any(|entry| entry.object_id == pushed.object_id)
    );
}

#[test]
fn stash_push_include_untracked_and_include_ignored() {
    let temp = TempDir::new("stash-untracked-ignored");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    let base = commit_file(&repo, "tracked.txt", "base\n", "base", &[]).unwrap();
    let base_oid = git2::Oid::from_str(&base).unwrap();
    commit_file(&repo, ".gitignore", "ignored.txt\n", "ignore", &[base_oid]).unwrap();

    fs::write(repo.join("untracked.txt"), "new\n").unwrap();
    backend
        .stash_push(
            &repo,
            "gwz:stash_untracked: untracked",
            GitStashPushOptions::include_untracked(),
        )
        .unwrap();
    assert!(!repo.join("untracked.txt").exists());

    fs::write(repo.join("ignored.txt"), "ignored\n").unwrap();
    backend
        .stash_push(
            &repo,
            "gwz:stash_ignored: ignored",
            GitStashPushOptions::include_ignored(),
        )
        .unwrap();
    assert!(!repo.join("ignored.txt").exists());
}

#[test]
fn stash_list_finds_gwz_prefixed_entries_and_object_ids() {
    let temp = TempDir::new("stash-list");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "tracked.txt", "base\n", "base", &[]).unwrap();

    fs::write(repo.join("tracked.txt"), "one\n").unwrap();
    let pushed = backend
        .stash_push(
            &repo,
            "gwz:stash_list: one",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();

    let entries = backend.stash_list(&repo).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].index, 0);
    assert_eq!(entries[0].object_id, pushed.object_id);
    assert!(stash_message_matches_gwz_prefix(
        &entries[0].message,
        "gwz:stash_list:"
    ));
}

#[test]
fn stash_apply_restores_and_keeps_entry() {
    let temp = TempDir::new("stash-apply");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "tracked.txt", "base\n", "base", &[]).unwrap();
    fs::write(repo.join("tracked.txt"), "changed\n").unwrap();
    let pushed = backend
        .stash_push(
            &repo,
            "gwz:stash_apply: apply",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();

    backend
        .stash_apply(
            &repo,
            &GitStashTarget::object_id(pushed.object_id.clone()),
            GitStashRestoreOptions::default(),
        )
        .unwrap();

    assert_text_eq(repo.join("tracked.txt"), "changed\n");
    assert!(
        backend
            .stash_list(&repo)
            .unwrap()
            .iter()
            .any(|entry| entry.object_id == pushed.object_id)
    );
}

#[test]
fn stash_pop_restores_and_removes_only_matching_gwz_stash_after_indices_move() {
    let temp = TempDir::new("stash-pop");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "tracked.txt", "base\n", "base", &[]).unwrap();

    fs::write(repo.join("tracked.txt"), "old\n").unwrap();
    let old = backend
        .stash_push(
            &repo,
            "gwz:stash_old: old",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();
    fs::write(repo.join("tracked.txt"), "new\n").unwrap();
    let new = backend
        .stash_push(
            &repo,
            "gwz:stash_new: new",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();

    backend
        .stash_pop(
            &repo,
            &GitStashTarget {
                object_id: Some(old.object_id.clone()),
                gwz_message_prefix: Some("gwz:stash_old:".to_owned()),
            },
            GitStashRestoreOptions::default(),
        )
        .unwrap();

    assert_text_eq(repo.join("tracked.txt"), "old\n");
    let entries = backend.stash_list(&repo).unwrap();
    assert!(!entries.iter().any(|entry| entry.object_id == old.object_id));
    assert!(entries.iter().any(|entry| entry.object_id == new.object_id));
}

#[test]
fn stash_drop_removes_only_matching_gwz_stash() {
    let temp = TempDir::new("stash-drop");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "tracked.txt", "base\n", "base", &[]).unwrap();

    fs::write(repo.join("tracked.txt"), "one\n").unwrap();
    let one = backend
        .stash_push(
            &repo,
            "gwz:stash_drop_one: one",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();
    fs::write(repo.join("tracked.txt"), "two\n").unwrap();
    let two = backend
        .stash_push(
            &repo,
            "gwz:stash_drop_two: two",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();

    backend
        .stash_drop(
            &repo,
            &GitStashTarget::gwz_message_prefix("gwz:stash_drop_one:"),
        )
        .unwrap();

    let entries = backend.stash_list(&repo).unwrap();
    assert!(!entries.iter().any(|entry| entry.object_id == one.object_id));
    assert!(entries.iter().any(|entry| entry.object_id == two.object_id));
}

#[test]
fn stash_prefix_fallback_restores_older_bundle_after_newer_indices_change() {
    let temp = TempDir::new("stash-index-shift");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "tracked.txt", "base\n", "base", &[]).unwrap();

    fs::write(repo.join("tracked.txt"), "older\n").unwrap();
    backend
        .stash_push(
            &repo,
            "gwz:stash_restore_older: older",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();
    fs::write(repo.join("tracked.txt"), "newer\n").unwrap();
    let newer = backend
        .stash_push(
            &repo,
            "gwz:stash_restore_newer: newer",
            GitStashPushOptions::tracked_only(),
        )
        .unwrap();

    backend
        .stash_drop(&repo, &GitStashTarget::object_id(newer.object_id))
        .unwrap();
    backend
        .stash_apply(
            &repo,
            &GitStashTarget::gwz_message_prefix("gwz:stash_restore_older:"),
            GitStashRestoreOptions::default(),
        )
        .unwrap();

    assert_text_eq(repo.join("tracked.txt"), "older\n");
}

#[test]
fn non_gwz_stashes_are_not_touched_by_prefix_helpers() {
    let temp = TempDir::new("stash-non-gwz");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "tracked.txt", "base\n", "base", &[]).unwrap();

    fs::write(repo.join("tracked.txt"), "manual\n").unwrap();
    run_git(&repo, &["stash", "push", "-m", "manual"]);
    let non_gwz = backend.stash_list(&repo).unwrap()[0].object_id.clone();

    let err = backend
        .stash_drop(&repo, &GitStashTarget::gwz_message_prefix("gwz:missing:"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::GitCommandFailed);
    assert!(
        backend
            .stash_list(&repo)
            .unwrap()
            .iter()
            .any(|entry| entry.object_id == non_gwz)
    );

    backend
        .stash_drop(&repo, &GitStashTarget::object_id(non_gwz.clone()))
        .unwrap();
    assert!(
        !backend
            .stash_list(&repo)
            .unwrap()
            .iter()
            .any(|entry| entry.object_id == non_gwz)
    );
}
