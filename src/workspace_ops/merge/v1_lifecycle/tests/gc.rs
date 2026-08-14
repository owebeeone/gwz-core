use std::fs;

use super::*;
use crate::git::{Git2Backend, GitBackend, GitDirectRefObservation};
use crate::workspace_ops::merge::PreservationEvidence;
use crate::workspace_ops::merge::model::v1::RecordVersion;
use crate::workspace_ops::merge::record_wire::archived_fixture_for_test;
use crate::workspace_ops::tests::{TempDir, commit_file};

#[test]
fn gc_deletes_valid_no_ref_v0_and_v1_archives() {
    let backend = Git2Backend::new();
    for version in [RecordVersion::V0, RecordVersion::V1] {
        let root = TempDir::new_git(&format!("merge-gc-empty-{version:?}"));
        let (bytes, merge_id) = archived_fixture_for_test(version);
        write_done(&root, merge_id, &bytes);

        let result = gc_archived(&backend, &root.path, merge_id).unwrap();

        assert_eq!(result.source_version(), version);
        assert!(result.cleanup().backup_refs().is_empty());
        assert!(!done_path(&root, merge_id).exists());
    }
}

#[test]
fn gc_requires_global_open_record_absence_and_a_supported_archive() {
    let backend = Git2Backend::new();
    let blocked = TempDir::new_git("merge-gc-open-blocker");
    let (bytes, merge_id) = archived_fixture_for_test(RecordVersion::V1);
    write_done(&blocked, merge_id, &bytes);
    let open = blocked.path.join(".gwz/merge/merge_other.yaml");
    fs::write(&open, b"an open record is present").unwrap();
    let error = expect_error(gc_archived(&backend, &blocked.path, merge_id));
    assert_eq!(error.code, ErrorCode::OpenOperation);
    assert!(done_path(&blocked, merge_id).is_file());

    for (name, bytes) in [
        ("malformed", b"not: [valid".as_slice()),
        (
            "future",
            b"schema: gwz.merge-operation/v2\nrecord_schema_version: 2\n".as_slice(),
        ),
    ] {
        let root = TempDir::new_git(&format!("merge-gc-{name}"));
        write_done(&root, "merge_archive", bytes);
        assert!(gc_archived(&backend, &root.path, "merge_archive").is_err());
        assert!(done_path(&root, "merge_archive").is_file());
    }
}

#[test]
fn gc_restarts_from_an_absent_prefix_and_keeps_stash_bundles() {
    for version in [RecordVersion::V0, RecordVersion::V1] {
        let fixture = cleanup_fixture(&format!("merge-gc-partial-{version:?}"), version, true);
        fixture
            .backend
            .delete_backup_ref_checked(&fixture.member, &fixture.member_ref, &fixture.member_commit)
            .unwrap();
        let bundle =
            crate::stash::bundle_path(&fixture.root.path, &format!("stash_{}", fixture.merge_id));
        fs::create_dir_all(bundle.parent().unwrap()).unwrap();
        fs::write(&bundle, b"immutable preservation bundle").unwrap();

        let result = gc_archived(&fixture.backend, &fixture.root.path, &fixture.merge_id).unwrap();

        assert_eq!(result.cleanup().backup_refs().len(), 2);
        assert!(result.cleanup().has_stash_evidence());
        assert!(
            fixture
                .backend
                .read_ref(&fixture.member, &fixture.member_ref)
                .unwrap()
                .is_none()
        );
        assert!(
            fixture
                .backend
                .read_ref(&fixture.root.path, &fixture.root_ref)
                .unwrap()
                .is_none()
        );
        assert_eq!(fs::read(bundle).unwrap(), b"immutable preservation bundle");
        assert!(!done_path(&fixture.root, &fixture.merge_id).exists());
    }
}

#[test]
fn gc_full_preflight_retains_every_ref_on_later_mismatch_or_missing_repo() {
    let mismatch = cleanup_fixture("merge-gc-mismatch", RecordVersion::V1, false);
    let wrong = commit_file(
        &mismatch.root.path,
        "wrong.txt",
        "wrong\n",
        "wrong",
        &[git2::Oid::from_str(&mismatch.root_commit).unwrap()],
    )
    .unwrap();
    mismatch
        .backend
        .delete_backup_ref_checked(
            &mismatch.root.path,
            &mismatch.root_ref,
            &mismatch.root_commit,
        )
        .unwrap();
    mismatch
        .backend
        .create_backup_ref(&mismatch.root.path, &mismatch.root_ref, &wrong)
        .unwrap();

    let error = expect_error(gc_archived(
        &mismatch.backend,
        &mismatch.root.path,
        &mismatch.merge_id,
    ));
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(
        mismatch
            .backend
            .read_ref(&mismatch.member, &mismatch.member_ref)
            .unwrap()
            .as_deref(),
        Some(mismatch.member_commit.as_str())
    );
    assert!(done_path(&mismatch.root, &mismatch.merge_id).is_file());

    let missing = cleanup_fixture("merge-gc-missing-repo", RecordVersion::V1, false);
    fs::remove_dir_all(&missing.member).unwrap();
    assert!(gc_archived(&missing.backend, &missing.root.path, &missing.merge_id).is_err());
    assert_eq!(
        missing
            .backend
            .read_ref(&missing.root.path, &missing.root_ref)
            .unwrap()
            .as_deref(),
        Some(missing.root_commit.as_str())
    );
    assert!(done_path(&missing.root, &missing.merge_id).is_file());
}

#[test]
fn gc_full_preflight_rejects_a_symbolic_ref_before_any_deletion() {
    let fixture = cleanup_fixture("merge-gc-symbolic-ref", RecordVersion::V1, false);
    fixture
        .backend
        .delete_backup_ref_checked(&fixture.member, &fixture.member_ref, &fixture.member_commit)
        .unwrap();
    let repository = git2::Repository::open(&fixture.member).unwrap();
    repository
        .reference(
            "refs/heads/gc-symbolic-target",
            git2::Oid::from_str(&fixture.member_commit).unwrap(),
            true,
            "gc symbolic target",
        )
        .unwrap();
    repository
        .reference_symbolic(
            &fixture.member_ref,
            "refs/heads/gc-symbolic-target",
            true,
            "gc symbolic preservation ref",
        )
        .unwrap();

    let error = expect_error(gc_archived(
        &fixture.backend,
        &fixture.root.path,
        &fixture.merge_id,
    ));

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_a"));
    assert_eq!(
        fixture
            .backend
            .observe_direct_ref(&fixture.member, &fixture.member_ref)
            .unwrap(),
        GitDirectRefObservation::NonDirect
    );
    assert_eq!(
        fixture
            .backend
            .read_ref(&fixture.root.path, &fixture.root_ref)
            .unwrap()
            .as_deref(),
        Some(fixture.root_commit.as_str())
    );
    assert!(done_path(&fixture.root, &fixture.merge_id).is_file());
}

#[test]
fn gc_rereads_archive_identity_and_every_ref_before_unlink() {
    let changed = cleanup_fixture("merge-gc-archive-change", RecordVersion::V1, false);
    let archive = done_path(&changed.root, &changed.merge_id);
    let error = expect_error(gc_archived_with_hook(
        &changed.backend,
        &changed.root.path,
        &changed.merge_id,
        || {
            let mut bytes = fs::read(&archive).unwrap();
            bytes.extend_from_slice(b"# replacement\n");
            fs::write(&archive, bytes).unwrap();
        },
    ));
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(archive.is_file());

    let reappeared = cleanup_fixture("merge-gc-ref-reappears", RecordVersion::V1, false);
    let error = expect_error(gc_archived_with_hook(
        &reappeared.backend,
        &reappeared.root.path,
        &reappeared.merge_id,
        || {
            reappeared
                .backend
                .create_backup_ref(
                    &reappeared.member,
                    &reappeared.member_ref,
                    &reappeared.member_commit,
                )
                .unwrap();
        },
    ));
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert!(done_path(&reappeared.root, &reappeared.merge_id).is_file());
}

struct CleanupFixture {
    root: TempDir,
    backend: Git2Backend,
    merge_id: String,
    member: std::path::PathBuf,
    member_ref: String,
    member_commit: String,
    root_ref: String,
    root_commit: String,
}

fn cleanup_fixture(name: &str, version: RecordVersion, stash: bool) -> CleanupFixture {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
    let root_commit = commit_file(&root.path, "root.txt", "root\n", "root", &[]).unwrap();
    let member = root.path.join("members/a");
    backend.create_repo(&member).unwrap();
    let member_commit = commit_file(&member, "member.txt", "member\n", "member", &[]).unwrap();
    let (bytes, merge_id) = archived_fixture_for_test(version);
    let bytes = with_cleanup_evidence(bytes, merge_id, &member_commit, &root_commit, stash);
    write_done(&root, merge_id, &bytes);
    let member_ref = format!("refs/gwz/merge/{merge_id}/mem_a/head");
    let root_ref = format!("refs/gwz/merge/{merge_id}/root/head");
    backend
        .create_backup_ref(&member, &member_ref, &member_commit)
        .unwrap();
    backend
        .create_backup_ref(&root.path, &root_ref, &root_commit)
        .unwrap();
    CleanupFixture {
        root,
        backend,
        merge_id: merge_id.to_owned(),
        member,
        member_ref,
        member_commit,
        root_ref,
        root_commit,
    }
}

fn with_cleanup_evidence(
    bytes: Vec<u8>,
    merge_id: &str,
    member_commit: &str,
    root_commit: &str,
    stash: bool,
) -> Vec<u8> {
    let mut raw: serde_yaml::Value = serde_yaml::from_slice(&bytes).unwrap();
    let member = PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{merge_id}/mem_a/head")),
        backup_commit: Some(member_commit.to_owned()),
        stash_id: stash.then(|| format!("stash_{merge_id}")),
        stash_object_id: stash.then(|| member_commit.to_owned()),
    };
    let root = PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{merge_id}/root/head")),
        backup_commit: Some(root_commit.to_owned()),
        stash_id: None,
        stash_object_id: None,
    };
    raw["participants"]["mem_a"]["preservation"] = serde_yaml::to_value(vec![member]).unwrap();
    raw["publication"]["root_preservation"] = serde_yaml::to_value(vec![root]).unwrap();
    serde_yaml::to_string(&raw).unwrap().into_bytes()
}

fn done_path(root: &TempDir, merge_id: &str) -> std::path::PathBuf {
    root.path
        .join(".gwz/merge/done")
        .join(format!("{merge_id}.yaml"))
}

fn write_done(root: &TempDir, merge_id: &str, bytes: &[u8]) {
    let path = done_path(root, merge_id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn expect_error<T>(result: ModelResult<T>) -> ModelError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("operation unexpectedly succeeded"),
    }
}
