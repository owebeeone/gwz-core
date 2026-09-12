use super::*;
use crate::artifact::{CONF_INTEGRITY_MARKER_PATH, LOCK_PATH, refresh_conf_integrity_marker};
use crate::git::GitBackend;

#[test]
fn clone_preserves_a_valid_marker_repair_and_its_index_state() {
    for staged in [false, true] {
        let fixture = family_workspace("valid-marker-repair");
        let backend = Git2Backend::without_credential_helpers();
        // The production backend commits the changed lock below. Do not borrow
        // an author identity from the developer's global Git config: hosted
        // runners have none, and Git then refuses the commit.
        let repo = git2::Repository::open(&fixture.root).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "GWZ Fixture").unwrap();
        config
            .set_str("user.email", "fixture@example.invalid")
            .unwrap();
        // Reproduce a composition that committed a changed lock with a stale marker.
        let lock = fs::read_to_string(fixture.root.join(LOCK_PATH)).unwrap();
        fs::write(fixture.root.join(LOCK_PATH), format!("# merged\n{lock}")).unwrap();
        backend.stage_paths(&fixture.root, &[LOCK_PATH]).unwrap();
        backend
            .commit(&fixture.root, "changed lock", false)
            .unwrap();
        refresh_conf_integrity_marker(&fixture.root).unwrap();
        if staged {
            backend
                .stage_paths(&fixture.root, &[CONF_INTEGRITY_MARKER_PATH])
                .unwrap();
        }
        // A verifying worktree variant must survive independently of the index.
        let repaired = fs::read_to_string(fixture.root.join(CONF_INTEGRITY_MARKER_PATH)).unwrap();
        fs::write(
            fixture.root.join(CONF_INTEGRITY_MARKER_PATH),
            format!("# keep this comment\n{repaired}"),
        )
        .unwrap();
        let before = backend.status(&fixture.root).unwrap();
        let index_before = read(&fixture.root.join(".git/index"));
        let marker_before = read(&fixture.root.join(CONF_INTEGRITY_MARKER_PATH));
        handle_clone_local_workspace(
            &backend,
            &fixture.root,
            clone_request("A"),
            "clone-repair",
            &NullSink,
        )
        .unwrap();
        let dest = fixture.sibling("A");
        assert_eq!(
            read(&fixture.root.join(CONF_INTEGRITY_MARKER_PATH)),
            marker_before
        );
        assert_eq!(read(&fixture.root.join(".git/index")), index_before);
        assert_eq!(read(&dest.join(CONF_INTEGRITY_MARKER_PATH)), marker_before);
        let after = backend.status(&dest).unwrap();
        let marker_status = |status: crate::git::GitStatus| {
            status
                .files
                .into_iter()
                .find(|entry| entry.path == CONF_INTEGRITY_MARKER_PATH)
                .unwrap()
        };
        assert_eq!(marker_status(before), marker_status(after));
        assert_eq!(
            inspect_conf_integrity(&dest),
            ConfIntegrityVerdict::Verified
        );
    }
}

#[test]
fn clone_refuses_invalid_marker_work_before_allocation() {
    let fixture = family_workspace("invalid-marker-work");
    fs::write(fixture.root.join(CONF_INTEGRITY_MARKER_PATH), "not: [yaml").unwrap();
    let error = handle_clone_local_workspace(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        clone_request("A"),
        "invalid-marker",
        &NullSink,
    )
    .unwrap_err();
    assert!(error.message.contains("integrity marker"), "{error}");
    assert!(family_files_absent(&fixture.root));
    assert!(!fixture.sibling("A").exists());
}
