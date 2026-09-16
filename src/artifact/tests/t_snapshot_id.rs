//! Snapshot-id validation and the snapshot/marker listings.

use std::cell::Cell;
use std::fs;

use crate::model::ErrorCode;

use super::*;

#[test]
fn f2_snapshot_id_validation_precedes_the_first_filesystem_read() {
    let temp = TempDir::new("snapshot-access-order");
    let reads = Cell::new(0);

    let error = read_snapshot_with(temp.path(), "../escape", |_| {
        reads.set(reads.get() + 1);
        Err(io_error(io::Error::other("access-order sentinel fired")))
    })
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(reads.get(), 0, "invalid id reached the filesystem reader");
}

#[test]
fn snapshot_reader_binds_requested_filename_to_embedded_id() {
    let temp = TempDir::new("snapshot-id-binding");
    fs::create_dir_all(temp.path().join(SNAPSHOT_DIR)).unwrap();
    fs::write(
        snapshot_path(temp.path(), "alias").unwrap(),
        sample_snapshot().to_yaml().unwrap(),
    )
    .unwrap();

    let error = read_snapshot(temp.path(), "alias").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("alias"), "{}", error.message);
    assert!(error.message.contains("snap_demo"), "{}", error.message);
}

#[test]
fn l_rng_6_creation_rejects_boundary_and_adjacent_dots_only() {
    let temp = TempDir::new("snapshot-range-id");
    let mut dotted = sample_snapshot();
    dotted.snapshot_id = "release.one".to_owned();
    write_snapshot(temp.path(), &dotted).unwrap();
    assert_eq!(read_snapshot(temp.path(), "release.one").unwrap(), dotted);

    for id in ["release..one", ".release", "release."] {
        let mut invalid = sample_snapshot();
        invalid.snapshot_id = id.to_owned();
        let error = write_snapshot(temp.path(), &invalid).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "id {id:?}");
        assert!(error.message.contains("snapshot_id"), "{}", error.message);
    }
}

#[test]
fn l_rng_6_schema_v0_legacy_dotted_ids_remain_listable_and_readable() {
    let temp = TempDir::new("snapshot-legacy-dotted-ids");
    fs::create_dir_all(temp.path().join(SNAPSHOT_DIR)).unwrap();
    for id in ["release..one", ".release", "release."] {
        fs::write(
            temp.path().join(SNAPSHOT_DIR).join(format!("{id}.yaml")),
            SNAPSHOT_GOLDEN.replace("snap_demo", id),
        )
        .unwrap();
        assert_eq!(
            read_snapshot(temp.path(), id).unwrap().snapshot_id,
            id,
            "id {id:?}"
        );
    }

    let ids = list_snapshots(temp.path())
        .unwrap()
        .into_iter()
        .map(|snapshot| snapshot.snapshot_id)
        .collect::<Vec<_>>();
    assert_eq!(ids, [".release", "release..one", "release."]);
}

#[test]
fn list_snapshots_reads_sorted_entries() {
    let temp = TempDir::new("artifact-list");
    // No dir yet → empty, not an error.
    assert!(list_snapshots(temp.path()).unwrap().is_empty());

    write_snapshot(temp.path(), &sample_snapshot()).unwrap(); // "snap_demo"
    let mut alpha = sample_snapshot();
    alpha.snapshot_id = "snap_alpha".to_owned();
    write_snapshot(temp.path(), &alpha).unwrap();
    let snapshots = list_snapshots(temp.path()).unwrap();
    assert_eq!(
        snapshots
            .iter()
            .map(|snapshot| snapshot.snapshot_id.as_str())
            .collect::<Vec<_>>(),
        vec!["snap_alpha", "snap_demo"]
    );
}

#[test]
fn list_markers_reads_sorted_entries() {
    let temp = TempDir::new("marker-list");
    // No dir yet -> empty, not an error.
    assert!(list_markers(temp.path()).unwrap().is_empty());

    let marker = sample_marker();
    write_marker(temp.path(), &marker).unwrap();
    let mut alpha = marker.clone();
    alpha.gwz_commit_id = "01987b0c-2f75-7c4a-9a32-8fd22f7d7c92".to_owned();
    write_marker(temp.path(), &alpha).unwrap();
    let markers = list_markers(temp.path()).unwrap();
    assert_eq!(
        markers
            .iter()
            .map(|marker| marker.gwz_commit_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91",
            "01987b0c-2f75-7c4a-9a32-8fd22f7d7c92"
        ]
    );
}
