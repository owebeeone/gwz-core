//! Atomic and durable publication tests.

use std::fs;

use super::*;

#[test]
fn atomic_write_replaces_existing_file_without_leftover_temp() {
    let temp = TempDir::new("atomic");
    let target = temp.path().join("nested/file.txt");

    write_atomic(&target, "old").unwrap();
    write_atomic(&target, "new").unwrap();

    assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    assert!(!temp.path().join("nested/file.txt.tmp").exists());
}

#[test]
fn write_atomic_publishes_content_and_leaves_no_temp_file() {
    // F12: the durable write lands the exact bytes and cleans up after itself — no
    // fixed-name `.tmp` lingering to race a concurrent writer.
    let temp = TempDir::new("write-atomic");
    let target = temp.path().join("lock.yaml");

    write_atomic(&target, "first\n").unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "first\n");
    write_atomic(&target, "second\n").unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "second\n");

    let leftovers = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect::<Vec<_>>();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}

#[test]
fn write_manifest_and_lock_publishes_both_consistently() {
    // F14: the consistency-critical pair is published together (lock last), durably,
    // leaving no temp litter.
    let temp = TempDir::new("manifest-lock");
    let manifest = sample_manifest();
    let lock = sample_lock();

    write_manifest_and_lock(temp.path(), &manifest, &lock).unwrap();

    assert_eq!(read_manifest(temp.path()).unwrap(), manifest);
    assert_eq!(read_lock(temp.path()).unwrap(), lock);
    let leftovers = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect::<Vec<_>>();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}
