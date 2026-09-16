//! Artifact file I/O: workspace paths, supplied worlds and path diagnostics.

use std::fs;

use crate::model::ErrorCode;

use super::*;

#[test]
fn artifact_file_io_uses_workspace_paths() {
    let temp = TempDir::new("artifact-io");
    write_manifest(temp.path(), &sample_manifest()).unwrap();
    write_lock(temp.path(), &sample_lock()).unwrap();
    write_snapshot(temp.path(), &sample_snapshot()).unwrap();
    write_marker(temp.path(), &sample_marker()).unwrap();

    assert_eq!(read_manifest(temp.path()).unwrap(), sample_manifest());
    assert_eq!(read_lock(temp.path()).unwrap(), sample_lock());
    assert_eq!(
        read_snapshot(temp.path(), "snap_demo").unwrap(),
        sample_snapshot()
    );
    assert_eq!(
        read_marker(temp.path(), &sample_marker().gwz_commit_id).unwrap(),
        sample_marker()
    );

    assert!(
        temp.path()
            .join("gwz.conf/snapshots/snap_demo.yaml")
            .is_file()
    );
    assert!(
        temp.path()
            .join("gwz.conf/markers/01987b0c-2f75-7c4a-9a32-8fd22f7d7c91.yaml")
            .is_file()
    );
}

#[test]
fn manifest_and_lock_in_stay_in_the_supplied_memory_world() {
    let world = crate::operation_context::TestWorld::memory();
    let services = world.context();
    let workspace = services.filesystem().test_workspace().unwrap();

    write_manifest_and_lock_in(
        services.filesystem(),
        workspace.path(),
        &sample_manifest(),
        &sample_lock(),
    )
    .unwrap();

    assert_eq!(
        read_manifest_in(services.filesystem(), workspace.path()).unwrap(),
        sample_manifest()
    );
    assert_eq!(
        read_lock_in(services.filesystem(), workspace.path()).unwrap(),
        sample_lock()
    );
    assert_eq!(
        conf_integrity::inspect_conf_integrity_in(services.filesystem(), workspace.path()),
        ConfIntegrityVerdict::Verified
    );
    for relative in [WORKSPACE_MANIFEST, LOCK_PATH, CONF_INTEGRITY_MARKER_PATH] {
        assert!(
            !workspace.path().join(relative).exists(),
            "the selected memory filesystem must not publish {relative} to the host filesystem"
        );
    }
    assert!(
        services
            .filesystem()
            .read(&workspace.path().join(CONF_INTEGRITY_MARKER_PATH))
            .is_ok(),
        "the integrity marker must publish in the supplied memory filesystem"
    );
}

#[test]
fn path_diagnostics_hint_selectors_only_when_literal_workspace_read_fails() {
    let temp = TempDir::new("selector-path-diagnostic");
    for name in ["@root", "@all"] {
        let root = temp.path().join(name);
        let error = read_manifest(&root).unwrap_err();
        assert_eq!(error.code, ErrorCode::ManifestNotFound);
        assert!(
            error.message.contains(&format!("--target {name}")),
            "{error:?}"
        );
        assert!(error.message.contains(&root.display().to_string()));
        write_manifest(&root, &sample_manifest()).unwrap();
        assert_eq!(read_manifest(&root).unwrap(), sample_manifest());
    }
    let error = read_manifest(&temp.path().join("ordinary")).unwrap_err();
    assert!(!error.message.contains("--target"));
}

#[test]
fn missing_manifest_is_typed_without_reclassifying_other_artifact_io() {
    let temp = TempDir::new("missing-manifest-code");

    let manifest = read_manifest(temp.path()).expect_err("manifest is absent");
    assert_eq!(manifest.code, ErrorCode::ManifestNotFound);

    let permission = manifest_io_error(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "injected permission denial",
    ));
    assert_eq!(permission.code, ErrorCode::PermissionDenied);

    for error in [
        io::Error::new(io::ErrorKind::InvalidData, "injected invalid UTF-8"),
        io::Error::other("injected manifest read failure"),
    ] {
        assert_eq!(
            manifest_io_error(error).code,
            ErrorCode::ManifestInvalid,
            "other workspace-manifest read failures are typed workspace rejections"
        );
    }

    let lock = read_lock(temp.path()).expect_err("lock is absent");
    assert_eq!(
        lock.code,
        ErrorCode::IoError,
        "the narrow correction must not reclassify general artifact I/O"
    );
}

#[test]
fn snapshot_reader_rejects_escape_and_absolute_ids_before_access() {
    let root = TempDir::new("snapshot-id-root");
    let outside = TempDir::new("snapshot-id-outside");
    fs::create_dir_all(root.path().join(SNAPSHOT_DIR)).unwrap();

    // Both files are valid snapshot YAML. A reader that joins before
    // validating can escape to either one.
    fs::write(
        root.path().join("gwz.conf/escape.yaml"),
        sample_snapshot().to_yaml().unwrap(),
    )
    .unwrap();
    let absolute_stem = outside.path().join("absolute");
    fs::write(
        absolute_stem.with_extension("yaml"),
        sample_snapshot().to_yaml().unwrap(),
    )
    .unwrap();

    for id in [
        "",
        "../escape",
        absolute_stem.to_str().expect("UTF-8 temp path"),
    ] {
        let error = read_snapshot(root.path(), id).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "id {id:?}");
        assert!(error.message.contains("snapshot_id"), "{}", error.message);
    }
}
