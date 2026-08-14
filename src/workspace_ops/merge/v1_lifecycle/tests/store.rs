use std::fs;
use std::path::Path;

use serde_yaml::Value;
use sha2::{Digest, Sha256};

use super::super::authority::{
    ParticipantActionPayload, PreparedAcceptedWorkspace, VerifiedParticipantOutcome,
    VerifiedParticipantRollback,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::store::{ArchiveOutcome, CheckedV1Store, CommitFault};
use super::super::transition::{
    AcceptanceTransition, ParticipantTransition, RollbackTransition, V1Transition, prepare,
};
use super::fixtures::up_to_date_action;
use crate::artifact::{ArtifactSourceKind, LockArtifact, ResolvedMemberArtifact};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::acceptance::{
    V1AcceptanceMetadata, V1AcceptanceRecord, build_v1_acceptance,
};
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, ParticipantRollbackKindV1, PendingRollbackActionV1,
    test_record as record,
};
use crate::workspace_ops::merge::{
    ConflictFileEvidence, MergeRecordError, OperationState, ParticipantState,
};
use crate::workspace_ops::tests::TempDir;

mod drift;
mod matrix;
mod shapes;

#[test]
fn commit_preserves_survivors_and_retires_only_the_owned_action_container() {
    let root = TempDir::new_git("merge-v1-store-unknowns");
    let mut model = record();
    model.participants.get_mut("mem_a").unwrap().pending_action = Some(up_to_date_action());
    seed_open(&root, &model, |raw| {
        insert(raw, "future_record", "record-value");
        let action = &mut raw["participants"]["mem_a"]["pending_action"];
        insert(action, "future_action", "action-value");
    });
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    assert_eq!(current.unknown_fields().entries().len(), 2);

    let rewrite = prepare_outcome(&lease, &current);
    let next = store.commit(&lease, &current, rewrite).unwrap();

    assert_eq!(next.raw()["future_record"], "record-value");
    assert!(next.record().participants["mem_a"].pending_action.is_none());
    assert_eq!(next.unknown_fields().entries().len(), 1);
    assert!(
        next.unknown_fields()
            .entries()
            .keys()
            .all(|locator| locator.field != "future_action")
    );
}

#[test]
fn rollback_retires_only_the_typed_pending_conflict_and_error_containers() {
    let root = TempDir::new_git("merge-v1-store-rollback-retirement");
    let mut model = record();
    model.state = OperationState::RollingBack;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Conflicted;
    row.expected_merge_head = Some(row.source_commit.clone());
    row.conflict_paths = vec!["conflicted.txt".into()];
    row.conflict_snapshot = vec![ConflictFileEvidence {
        path: "conflicted.txt".into(),
        sha256: "1".repeat(64),
    }];
    row.error = Some(MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: "resolution failed".into(),
        detail: Some("retry or abort".into()),
    });
    model.pending_rollback = Some(PendingRollbackActionV1::Participant {
        member_id: "mem_a".into(),
        action: ParticipantRollbackKindV1::AbortConflict,
        terminal_state: ParticipantState::Aborted,
    });
    seed_open(&root, &model, |raw| {
        insert(raw, "future_record", "survives");
        insert(
            &mut raw["participants"]["mem_a"]["conflict_snapshot"][0],
            "future_conflict",
            "retires",
        );
        insert(
            &mut raw["participants"]["mem_a"]["error"],
            "future_error",
            "retires",
        );
        insert(&mut raw["pending_rollback"], "future_rollback", "retires");
    });
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    assert_eq!(current.unknown_fields().entries().len(), 4);

    let mut row = current.record().participants["mem_a"].clone();
    row.state = ParticipantState::Aborted;
    row.expected_merge_head = None;
    row.conflict_paths.clear();
    row.conflict_snapshot.clear();
    row.error = None;
    let proof = VerifiedParticipantRollback::for_test(
        &current,
        "mem_a",
        "finish_participant_rollback",
        "completed",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row,
        },
    )
    .unwrap();
    let rewrite = prepare(
        &lease,
        &current,
        V1Transition::Rollback(Box::new(RollbackTransition::FinishParticipant(Box::new(
            proof,
        )))),
    )
    .unwrap();
    let next = store.commit(&lease, &current, rewrite).unwrap();

    assert_eq!(next.raw()["future_record"], "survives");
    assert_eq!(next.unknown_fields().entries().len(), 1);
    assert_eq!(
        next.record().participants["mem_a"].state,
        ParticipantState::Aborted
    );
}

#[test]
fn first_acceptance_write_preserves_derived_lock_member_extensions() {
    let root = TempDir::new_git("merge-v1-store-acceptance-extensions");
    let mut model = record();
    model.state = OperationState::Finalizing;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::FastForwarded;
    row.resulting_commit = Some("d".repeat(40));
    let mut lock = LockArtifact::from_yaml(model.baseline.lock_yaml.as_ref().unwrap()).unwrap();
    lock.members.insert(
        "mem_a".into(),
        ResolvedMemberArtifact {
            path: "members/a".into(),
            source_id: Some("src_a".into()),
            source_kind: ArtifactSourceKind::Git,
            commit: Some("a".repeat(40)),
            branch: Some("main".into()),
            detached: Some(false),
            upstream: None,
            dirty: Some(false),
            materialized: Some(true),
        },
    );
    let mut lock_raw = serde_yaml::to_value(&lock).unwrap();
    insert(
        &mut lock_raw["members"]["mem_a"],
        "future_lock_member",
        "derived-and-preserved",
    );
    let lock_yaml = serde_yaml::to_string(&lock_raw).unwrap();
    model.baseline.lock_sha256 = digest(&lock_yaml);
    model.baseline.lock_yaml = Some(lock_yaml);
    seed_open(&root, &model, |_| {});

    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    let built = build_v1_acceptance(
        V1AcceptanceRecord::V1(current.record()),
        V1AcceptanceMetadata::OperationBaseline,
    )
    .unwrap();
    let proof = PreparedAcceptedWorkspace::for_test(
        &current,
        "@operation",
        "freeze_acceptance",
        "prepared",
        built.into_accepted_workspace(),
    )
    .unwrap();
    let rewrite = prepare(
        &lease,
        &current,
        V1Transition::Acceptance(Box::new(AcceptanceTransition::Freeze(Box::new(proof)))),
    )
    .unwrap();
    let next = store.commit(&lease, &current, rewrite).unwrap();

    assert_eq!(
        next.raw()["accepted_workspace"]["member_audit"]["mem_a"]["lock_member"]["future_lock_member"],
        "derived-and-preserved"
    );
    assert!(
        next.unknown_fields()
            .entries()
            .keys()
            .any(|locator| locator.field == "future_lock_member")
    );
}

#[test]
fn contention_and_wrong_root_are_rejected_before_mutation() {
    let root = TempDir::new_git("merge-v1-store-contention");
    let other = TempDir::new_git("merge-v1-store-wrong-root");
    let mut model = record();
    model.participants.get_mut("mem_a").unwrap().pending_action = Some(up_to_date_action());
    let original = seed_open(&root, &model, |_| {});
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let wrong_lease = V1MutationLease::acquire_for_test(&other.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();

    let wrong = store
        .commit(&wrong_lease, &current, prepare_outcome(&lease, &current))
        .err()
        .unwrap();
    assert_eq!(wrong.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(fs::read(current.location().path()).unwrap(), original);

    let rewrite = prepare_outcome(&lease, &current);
    let mut contended = original.clone();
    contended.extend_from_slice(b"# concurrent rewrite\n");
    fs::write(current.location().path(), &contended).unwrap();
    let error = store.commit(&lease, &current, rewrite).err().unwrap();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(fs::read(current.location().path()).unwrap(), contended);
}

#[test]
fn fault_before_publish_keeps_source_and_cleans_temporary() {
    let root = TempDir::new_git("merge-v1-store-temp-fault");
    let mut model = record();
    model.participants.get_mut("mem_a").unwrap().pending_action = Some(up_to_date_action());
    let original = seed_open(&root, &model, |_| {});
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let current = CheckedV1Store::default()
        .load_open(&root.path, "merge_1")
        .unwrap();
    let error = CheckedV1Store::failing_after(CommitFault::AfterTemporarySync)
        .commit(&lease, &current, prepare_outcome(&lease, &current))
        .err()
        .unwrap();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(fs::read(current.location().path()).unwrap(), original);
    assert!(temporary_files(current.location().path()).is_empty());
}

#[test]
fn fault_after_publish_is_recoverable_by_reopen_and_stale_retry_is_rejected() {
    let root = TempDir::new_git("merge-v1-store-publish-fault");
    let mut model = record();
    model.participants.get_mut("mem_a").unwrap().pending_action = Some(up_to_date_action());
    seed_open(&root, &model, |_| {});
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    let expected = prepare_outcome(&lease, &current).next().clone();
    let stale_retry = prepare_outcome(&lease, &current);

    let error = CheckedV1Store::failing_after(CommitFault::AfterPublish)
        .commit(&lease, &current, prepare_outcome(&lease, &current))
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    let reopened = store.load_open(&root.path, "merge_1").unwrap();
    assert_eq!(reopened.record(), &expected);
    assert!(store.commit(&lease, &current, stale_retry).is_err());
}

#[test]
fn archive_moves_exact_bytes_and_reconciles_both_crash_shapes() {
    let root = TempDir::new_git("merge-v1-store-archive");
    let mut terminal = record();
    terminal.state = OperationState::Aborted;
    terminal.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
    let original = seed_open(&root, &terminal, |raw| {
        insert(raw, "future_terminal", "must-remain-exact");
    });
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    let source = current.location().path().to_owned();
    let destination = source.parent().unwrap().join("done/merge_1.yaml");

    assert_eq!(
        store.archive(&lease, &current).unwrap(),
        ArchiveOutcome::Published
    );
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), original);
    assert_eq!(
        store.archive(&lease, &current).unwrap(),
        ArchiveOutcome::ReconciledDestination
    );

    fs::write(&source, &original).unwrap();
    assert_eq!(
        store.archive(&lease, &current).unwrap(),
        ArchiveOutcome::ReconciledBothCopies
    );
    assert!(!source.exists());
    assert_eq!(fs::read(destination).unwrap(), original);
}

#[test]
fn archive_rejects_nonterminal_and_mismatched_destination_without_deleting_source() {
    let root = TempDir::new_git("merge-v1-store-archive-reject");
    let original = seed_open(&root, &record(), |_| {});
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    assert_eq!(
        store.archive(&lease, &current).unwrap_err().code,
        ErrorCode::MergeRecoveryRequired
    );

    let mut terminal = record();
    terminal.state = OperationState::Aborted;
    terminal.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
    let terminal_bytes = seed_open(&root, &terminal, |_| {});
    let current = store.load_open(&root.path, "merge_1").unwrap();
    let done = current.location().path().parent().unwrap().join("done");
    fs::create_dir_all(&done).unwrap();
    fs::write(done.join("merge_1.yaml"), b"different").unwrap();
    assert_eq!(
        store.archive(&lease, &current).unwrap_err().code,
        ErrorCode::MergeRecoveryRequired
    );
    assert_eq!(fs::read(current.location().path()).unwrap(), terminal_bytes);
    assert_ne!(original, terminal_bytes);
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    windows
))]
#[test]
fn archive_rename_primitive_never_clobbers_an_existing_destination() {
    let root = TempDir::new_git("merge-v1-archive-rename-noreplace");
    let source = root.path.join("source");
    let destination = root.path.join("destination");
    fs::write(&source, b"source").unwrap();
    fs::write(&destination, b"destination").unwrap();

    assert_eq!(
        crate::durable_fs::rename_noreplace(&source, &destination)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read(destination).unwrap(), b"destination");
}

#[cfg(unix)]
#[test]
fn archive_rejects_symlinked_done_directory_before_renaming_outside_the_workspace() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new_git("merge-v1-store-archive-done-symlink");
    let outside = TempDir::new_git("merge-v1-store-archive-outside");
    let mut terminal = record();
    terminal.state = OperationState::Aborted;
    terminal.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
    let original = seed_open(&root, &terminal, |_| {});
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    symlink(
        &outside.path,
        current.location().path().parent().unwrap().join("done"),
    )
    .unwrap();

    assert_eq!(
        store.archive(&lease, &current).unwrap_err().code,
        ErrorCode::MergeRecoveryRequired
    );
    assert_eq!(fs::read(current.location().path()).unwrap(), original);
    assert!(!outside.path.join("merge_1.yaml").exists());
}

#[cfg(unix)]
#[test]
fn open_rejects_symlink_and_noncanonical_record_identity() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new_git("merge-v1-store-open-shape");
    let bytes = serde_yaml::to_string(&record()).unwrap().into_bytes();
    let merge_root = root.path.join(".gwz/merge");
    fs::create_dir_all(&merge_root).unwrap();
    fs::write(merge_root.join("target.yaml"), &bytes).unwrap();
    symlink(
        merge_root.join("target.yaml"),
        merge_root.join("merge_1.yaml"),
    )
    .unwrap();
    let store = CheckedV1Store::default();
    assert_eq!(
        store.load_open(&root.path, "merge_1").err().unwrap().code,
        ErrorCode::MergeRecordUnreadable
    );
    assert_eq!(
        store
            .load_open(&root.path, "../merge_1")
            .err()
            .unwrap()
            .code,
        ErrorCode::MergeRecoveryRequired
    );
}

fn prepare_outcome(
    lease: &V1MutationLease,
    current: &StoredV1Record,
) -> super::super::transition::PreparedV1Rewrite {
    let mut row = current.record().participants["mem_a"].clone();
    row.state = ParticipantState::UpToDate;
    row.resulting_commit = Some(row.before_commit.clone());
    row.pending_action = None;
    row.error = None;
    let proof = VerifiedParticipantOutcome::for_test(
        current,
        "mem_a",
        "participant_outcome",
        "completed",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row,
        },
    )
    .unwrap();
    prepare(
        lease,
        current,
        V1Transition::Participant(Box::new(ParticipantTransition::RecordOutcome(Box::new(
            proof,
        )))),
    )
    .unwrap()
}

fn seed_open(
    root: &TempDir,
    model: &MergeOperationRecordV1,
    mutate: impl FnOnce(&mut Value),
) -> Vec<u8> {
    let mut raw = serde_yaml::to_value(model).unwrap();
    mutate(&mut raw);
    let bytes = serde_yaml::to_string(&raw).unwrap().into_bytes();
    let merge_root = root.path.join(".gwz/merge");
    fs::create_dir_all(&merge_root).unwrap();
    fs::write(merge_root.join(format!("{}.yaml", model.merge_id)), &bytes).unwrap();
    bytes
}

fn insert(value: &mut Value, key: &str, content: &str) {
    value
        .as_mapping_mut()
        .unwrap()
        .insert(Value::String(key.into()), Value::String(content.into()));
}

fn temporary_files(path: &Path) -> Vec<String> {
    fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".v1.tmp"))
        .collect()
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
