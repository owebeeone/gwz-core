//! Generated marker regression: committed stale digests are not user work.
use super::*;
use crate::artifact::CONF_INTEGRITY_MARKER_PATH;

fn stale_marker_lane(label: &str) -> FamilyFixture {
    let fixture = clean_family_workspace(label);
    let marker = fs::read_to_string(fixture.root.join(CONF_INTEGRITY_MARKER_PATH)).unwrap();
    let stale = marker
        .lines()
        .map(|line| {
            if let Some((prefix, _)) = line.split_once("sha256:") {
                format!("{prefix}sha256:{}", "0".repeat(64))
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_ne!(marker, stale, "fixture must actually change the marker");
    commit_in(
        &fixture.root,
        CONF_INTEGRITY_MARKER_PATH,
        &stale,
        "stale marker",
    );
    clone(&fixture.root, "A");
    fixture
}

#[test]
fn changed_configuration_is_protected_even_with_a_matching_marker() {
    for relative in crate::artifact::GUARDED_CONF_PATHS {
        let fixture = stale_marker_lane("dispose-dirty-guarded-conf");
        let lane = fixture.sibling("A");
        let path = lane.join(relative);
        let contents = format!("{}\n# user work\n", fs::read_to_string(&path).unwrap());
        fs::write(&path, contents).unwrap();
        crate::artifact::refresh_conf_integrity_marker(&lane).unwrap();
        let before = tree_bytes(&lane);
        refuse(&fixture.root, delete_request("A", &[]));
        assert_eq!(before, tree_bytes(&lane));
    }
}

#[test]
fn future_and_malformed_markers_are_protected() {
    for contents in ["schema: gwz.conf-integrity/v99\nfiles: {}\n", "not: [yaml"] {
        let fixture = stale_marker_lane("dispose-unknown-marker");
        let lane = fixture.sibling("A");
        fs::write(lane.join(CONF_INTEGRITY_MARKER_PATH), contents).unwrap();
        let before = tree_bytes(&lane);
        refuse(&fixture.root, delete_request("A", &[]));
        assert_eq!(before, tree_bytes(&lane));
    }
}

#[cfg(unix)]
#[test]
fn marker_symlink_is_never_generated_maintenance() {
    let fixture = stale_marker_lane("dispose-marker-symlink");
    let lane = fixture.sibling("A");
    let marker = lane.join(CONF_INTEGRITY_MARKER_PATH);
    let outside = fixture.root.join("marker-copy");
    fs::write(&outside, fs::read(&marker).unwrap()).unwrap();
    fs::remove_file(&marker).unwrap();
    std::os::unix::fs::symlink(&outside, &marker).unwrap();
    refuse(&fixture.root, delete_request("A", &[]));
    assert!(
        fs::symlink_metadata(&marker)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(outside.exists());
}

#[test]
fn legacy_source_without_a_marker_refuses_before_allocation() {
    let fixture = clean_family_workspace("clone-no-integrity-marker");
    fs::remove_file(fixture.root.join(CONF_INTEGRITY_MARKER_PATH)).unwrap();
    let result = handle_clone_local_workspace(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        clone_request("A"),
        "op-clone",
        &NullSink,
    );
    assert!(result.is_err());
    assert!(!fixture.sibling("A").exists());
}

#[test]
fn marker_admission_is_rechecked_at_the_removal_boundary() {
    let fixture = stale_marker_lane("dispose-marker-drift");
    let lane = fixture.sibling("A");
    let mut ports = CoreDisposalPorts::new(
        fixture.root.clone(),
        family(&fixture.root).unwrap(),
        MemberName::parse("A").unwrap(),
        open_merge_probe,
    );
    ports.observe_target(&lane).unwrap();
    let marker = lane.join(CONF_INTEGRITY_MARKER_PATH);
    let changed = format!(
        "{}\n# late user edit\n",
        fs::read_to_string(&marker).unwrap()
    );
    fs::write(&marker, &changed).unwrap();
    assert!(ports.remove_directory(&lane).is_err());
    assert_eq!(fs::read_to_string(&marker).unwrap(), changed);
}

#[cfg(unix)]
#[test]
fn unreadable_marker_remains_protected() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = stale_marker_lane("dispose-unreadable-marker");
    let lane = fixture.sibling("A");
    let marker = lane.join(CONF_INTEGRITY_MARKER_PATH);
    let original = fs::metadata(&marker).unwrap().permissions();
    fs::set_permissions(&marker, fs::Permissions::from_mode(0o0)).unwrap();
    let result = try_local(&fixture.root, delete_request("A", &[]));
    fs::set_permissions(&marker, original).unwrap();
    assert!(result.is_err());
    assert!(lane.exists());
}

#[test]
fn new_root_history_after_marker_admission_prevents_removal() {
    let fixture = stale_marker_lane("dispose-late-root-history");
    let lane = fixture.sibling("A");
    let mut ports = CoreDisposalPorts::new(
        fixture.root.clone(),
        family(&fixture.root).unwrap(),
        MemberName::parse("A").unwrap(),
        open_merge_probe,
    );
    ports.observe_target(&lane).unwrap();
    commit_in(&lane, "new-root-work", "keep this", "late root commit");
    assert!(ports.remove_directory(&lane).is_err());
    assert_eq!(
        fs::read_to_string(lane.join("new-root-work")).unwrap(),
        "keep this"
    );
}

#[test]
fn clone_keeps_committed_manifest_formatting() {
    let fixture = clean_family_workspace("clone-manifest-formatting");
    let path = crate::workspace::WORKSPACE_MANIFEST;
    let contents = format!(
        "{}\n# committed comment\n",
        fs::read_to_string(fixture.root.join(path)).unwrap()
    );
    commit_in(&fixture.root, path, &contents, "manifest comment");
    clone(&fixture.root, "A");
    assert_eq!(
        fs::read_to_string(fixture.sibling("A").join(path)).unwrap(),
        contents
    );
    local(&fixture.root, delete_request("A", &[]));
}

#[test]
fn clone_does_not_overwrite_status_suppressed_marker_work() {
    let fixture = clean_family_workspace("clone-suppressed-marker");
    let repo = git2::Repository::open(&fixture.root).unwrap();
    let mut index = repo.index().unwrap();
    let mut entry = index
        .get_path(Path::new(CONF_INTEGRITY_MARKER_PATH), 0)
        .unwrap();
    entry.flags |= 0x8000;
    index.add(&entry).unwrap();
    index.write().unwrap();
    let marker = fixture.root.join(CONF_INTEGRITY_MARKER_PATH);
    let bytes = format!(
        "{}\n# suppressed work\n",
        fs::read_to_string(&marker).unwrap()
    );
    fs::write(&marker, &bytes).unwrap();
    let result = handle_clone_local_workspace(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        clone_request("A"),
        "op-clone",
        &NullSink,
    );
    if result.is_ok() {
        assert_eq!(
            fs::read_to_string(fixture.sibling("A").join(CONF_INTEGRITY_MARKER_PATH)).unwrap(),
            bytes
        );
    } else {
        assert!(!fixture.sibling("A").exists(), "{result:?}");
    }
}

#[test]
fn untouched_lane_with_regenerated_marker_disposes() {
    let fixture = stale_marker_lane("dispose-regenerated-marker");
    local(&fixture.root, delete_request("A", &[]));
    assert!(!fixture.sibling("A").exists());
    assert!(!listed_names(&fixture.root).contains(&"A".to_owned()));
}

#[test]
fn staged_regenerated_marker_remains_protected() {
    let fixture = stale_marker_lane("dispose-staged-marker");
    let lane = fixture.sibling("A");
    let repo = git2::Repository::open(&lane).unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_path(Path::new(CONF_INTEGRITY_MARKER_PATH))
        .unwrap();
    index.write().unwrap();
    let before = tree_bytes(&lane);
    refuse(&fixture.root, delete_request("A", &[]));
    assert_eq!(before, tree_bytes(&lane));
}

#[test]
fn noncanonical_marker_remains_protected() {
    let fixture = stale_marker_lane("dispose-edited-marker");
    let lane = fixture.sibling("A");
    let marker = lane.join(CONF_INTEGRITY_MARKER_PATH);
    let mut bytes = fs::read(&marker).unwrap();
    bytes.extend_from_slice(b"# user note\n");
    fs::write(marker, bytes).unwrap();
    let before = tree_bytes(&lane);
    refuse(&fixture.root, delete_request("A", &[]));
    assert_eq!(before, tree_bytes(&lane));
}

#[test]
fn clone_preserves_source_marker_work_and_manifest_bytes() {
    let fixture = clean_family_workspace("clone-source-conf-work");
    let manifest = fixture.root.join(crate::workspace::WORKSPACE_MANIFEST);
    let original = fs::read_to_string(&manifest).unwrap();
    let with_comment = format!("{original}\n# source work to preserve\n");
    fs::write(&manifest, &with_comment).unwrap();
    let marker = fixture.root.join(CONF_INTEGRITY_MARKER_PATH);
    let marker_work = format!(
        "{}\n# marker work to preserve\n",
        fs::read_to_string(&marker).unwrap()
    );
    fs::write(&marker, &marker_work).unwrap();
    // Refusing before allocation is also permitted; silently overwriting is not.
    let result = handle_clone_local_workspace(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        clone_request("A"),
        "op-clone",
        &NullSink,
    );
    assert_eq!(fs::read_to_string(&manifest).unwrap(), with_comment);
    assert_eq!(fs::read_to_string(&marker).unwrap(), marker_work);
    if result.is_ok() {
        assert_eq!(
            fs::read_to_string(
                fixture
                    .sibling("A")
                    .join(crate::workspace::WORKSPACE_MANIFEST)
            )
            .unwrap(),
            with_comment
        );
        assert_eq!(
            fs::read_to_string(fixture.sibling("A").join(CONF_INTEGRITY_MARKER_PATH)).unwrap(),
            marker_work
        );
    } else {
        assert!(
            !fixture.sibling("A").exists(),
            "unexpected partial clone: {result:?}"
        );
    }
}
