use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::Value;
use sha2::{Digest, Sha256};

use crate::git::{GitBackend, GitHeadState, GitStatus};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::{
    AtomicUpgradeFault, AtomicUpgradeOutcome, FileMergeStore, MergeOperationRecord, MergeStore,
    PreparedOpenV0Upgrade,
};

#[derive(Debug, Eq, PartialEq)]
struct RepoObservation {
    path: PathBuf,
    head: GitHeadState,
    status: GitStatus,
}

pub(super) fn assert_upgrade_fixture<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    rule_id: &str,
    case_id: &str,
) {
    let next_action = crate::workspace_ops::merge::finalization_next_action_for_i2(record).unwrap();
    let path = open_path(root, &record.merge_id);
    let source = fs::read(&path).unwrap();
    let observations = observe_repositories(backend, root, record);

    let outcome = upgrade(backend, root, &record.merge_id, AtomicUpgradeFault::None).unwrap();
    assert_eq!(
        outcome,
        AtomicUpgradeOutcome::Upgraded {
            rule_id: rule_id.to_owned(),
            next_action: next_action.to_owned(),
        }
    );
    let published = fs::read(&path).unwrap();
    assert_ne!(published, source);
    assert_v1_restart(&published, &record.merge_id, next_action);
    assert!(temporary_files(&path).is_empty());
    assert_eq!(observe_repositories(backend, root, record), observations);

    fs::write(&path, &source).unwrap();
    if case_id == "changed/finalizing-before-publication-record" {
        assert_fault_matrix(backend, root, record, &source, next_action);
        assert_multiple_open_rejected(backend, root, record, &source);
    }
    assert_unknown_fields_and_verifier(backend, root, record, &source);
    if case_id == "changed/candidate-persisted" {
        assert_accepted_lock_extension(backend, root, record, &source);
    }
    fs::write(path, source).unwrap();
}

pub(super) fn assert_valid_unlisted<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) {
    let path = open_path(root, &record.merge_id);
    let source = fs::read(&path).unwrap();
    assert_eq!(
        upgrade(backend, root, &record.merge_id, AtomicUpgradeFault::None).unwrap(),
        AtomicUpgradeOutcome::ValidUnlisted
    );
    assert_eq!(fs::read(&path).unwrap(), source);
    assert!(temporary_files(&path).is_empty());
}

pub(super) fn assert_rejection_guards<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) {
    let mut no_ff = record.clone();
    no_ff.mode = crate::workspace_ops::merge::MergeExecutionMode::NoFf;
    assert_rejected_without_stage(
        backend,
        root,
        &record.merge_id,
        serde_yaml::to_string(&no_ff).unwrap().as_bytes(),
        ErrorCode::UnsupportedLegacyMode,
    );

    let raw = serde_yaml::to_value(record).unwrap();
    for field in [
        "accepted_workspace",
        "recovery_context",
        "pending_rollback",
        "pending_preservation",
    ] {
        let mut colliding = raw.clone();
        colliding[field] = Value::String("future-v0-value".to_owned());
        assert_rejected_without_stage(
            backend,
            root,
            &record.merge_id,
            serde_yaml::to_string(&colliding).unwrap().as_bytes(),
            ErrorCode::MergeRecordUnreadable,
        );
    }
}

fn assert_rejected_without_stage<B: GitBackend>(
    backend: &B,
    root: &Path,
    merge_id: &str,
    replacement: &[u8],
    expected: ErrorCode,
) {
    let path = open_path(root, merge_id);
    let original = fs::read(&path).unwrap();
    fs::write(&path, replacement).unwrap();
    let error = upgrade(backend, root, merge_id, AtomicUpgradeFault::None).unwrap_err();
    assert_eq!(error.code, expected);
    assert_eq!(fs::read(&path).unwrap(), replacement);
    assert!(temporary_files(&path).is_empty());
    fs::write(path, original).unwrap();
}

fn assert_fault_matrix<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    source: &[u8],
    next_action: &str,
) {
    let path = open_path(root, &record.merge_id);
    for fault in [
        AtomicUpgradeFault::BeforeStageWrite,
        AtomicUpgradeFault::AfterStageFsync,
        AtomicUpgradeFault::BeforeAtomicRename,
        AtomicUpgradeFault::AfterRenameBeforeVerification,
    ] {
        fs::write(&path, source).unwrap();
        remove_temporary_files(&path);
        let error = upgrade(backend, root, &record.merge_id, fault).unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired, "{fault:?}");
        let temporary = temporary_files(&path);
        if fault == AtomicUpgradeFault::AfterRenameBeforeVerification {
            let published = fs::read(&path).unwrap();
            assert_v1_restart(&published, &record.merge_id, next_action);
            assert_eq!(
                FileMergeStore
                    .load(root, &record.merge_id)
                    .unwrap_err()
                    .code,
                ErrorCode::UnsupportedRecordVersion
            );
            assert!(temporary.is_empty());
        } else {
            assert_eq!(fs::read(&path).unwrap(), source, "{fault:?}");
            assert_eq!(
                FileMergeStore.load(root, &record.merge_id).unwrap(),
                *record,
                "{fault:?}"
            );
            assert_eq!(
                FileMergeStore.discover_open(root).unwrap().unwrap(),
                *record,
                "{fault:?}"
            );
            assert_eq!(
                temporary.len(),
                usize::from(matches!(
                    fault,
                    AtomicUpgradeFault::AfterStageFsync | AtomicUpgradeFault::BeforeAtomicRename
                )),
                "{fault:?}"
            );
        }
    }
    remove_temporary_files(&path);
}

fn assert_multiple_open_rejected<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    source: &[u8],
) {
    let path = open_path(root, &record.merge_id);
    fs::write(&path, source).unwrap();
    let mut second = record.clone();
    second.merge_id = "merge_atomic_second".to_owned();
    let second_path = open_path(root, &second.merge_id);
    let second_bytes = serde_yaml::to_string(&second).unwrap().into_bytes();
    fs::write(&second_path, &second_bytes).unwrap();

    let error = upgrade(backend, root, &record.merge_id, AtomicUpgradeFault::None).unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::read(&second_path).unwrap(), second_bytes);
    assert!(temporary_files(&path).is_empty());
    fs::remove_file(second_path).unwrap();
}

fn assert_accepted_lock_extension<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    source: &[u8],
) {
    let path = open_path(root, &record.merge_id);
    let mut extended = record.clone();
    let publication = extended.publication.as_mut().unwrap();
    let candidate = publication.candidate.as_mut().unwrap();
    let mut lock: Value = serde_yaml::from_str(&candidate.lock_yaml).unwrap();
    lock["members"][&record.selected_targets[0]]["future_lock_member"] =
        Value::String("retained".to_owned());
    candidate.lock_yaml = serde_yaml::to_string(&lock).unwrap();
    let lock_sha256 = format!("{:x}", Sha256::digest(candidate.lock_yaml.as_bytes()));
    publication.candidate_lock_sha256 = Some(lock_sha256.clone());
    for hash in &mut publication.candidate_hashes {
        if hash.path == crate::artifact::LOCK_PATH {
            hash.sha256.clone_from(&lock_sha256);
        }
    }
    let expected_lock_yaml = candidate.lock_yaml.clone();
    let extended = serde_yaml::to_string(&extended).unwrap().into_bytes();
    fs::write(&path, &extended).unwrap();

    upgrade(backend, root, &record.merge_id, AtomicUpgradeFault::None).unwrap();
    let published: Value = serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        published["accepted_workspace"]["member_audit"][&record.selected_targets[0]]["lock_member"]
            ["future_lock_member"],
        "retained"
    );
    assert_eq!(
        published["accepted_workspace"]["lock"]["exact_yaml"],
        expected_lock_yaml
    );
    fs::write(path, source).unwrap();
}

fn assert_unknown_fields_and_verifier<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    source: &[u8],
) {
    let path = open_path(root, &record.merge_id);
    let mut raw: Value = serde_yaml::from_slice(source).unwrap();
    raw["future_atomic_top"] = Value::String("top".to_owned());
    raw["baseline"]["future_atomic_baseline"] = Value::String("baseline".to_owned());
    raw["participants"][&record.selected_targets[0]]["future_atomic_participant"] =
        Value::String("participant".to_owned());
    let publication = raw
        .as_mapping_mut()
        .unwrap()
        .get_mut(Value::String("publication".to_owned()))
        .and_then(Value::as_mapping_mut);
    let has_publication = publication.is_some();
    let mut has_candidate = false;
    let mut has_hash = false;
    if let Some(publication) = publication {
        publication.insert(
            Value::String("future_atomic_publication".to_owned()),
            Value::String("publication".to_owned()),
        );
        if let Some(candidate) = publication
            .get_mut(Value::String("candidate".to_owned()))
            .and_then(Value::as_mapping_mut)
        {
            has_candidate = true;
            candidate.insert(
                Value::String("future_atomic_candidate".to_owned()),
                Value::String("candidate".to_owned()),
            );
        }
        if let Some(hash) = publication
            .get_mut(Value::String("candidate_hashes".to_owned()))
            .and_then(Value::as_sequence_mut)
            .and_then(|hashes| hashes.first_mut())
        {
            has_hash = true;
            hash["future_atomic_hash"] = Value::String("hash".to_owned());
        }
    }
    let extended = serde_yaml::to_string(&raw).unwrap().into_bytes();
    fs::write(&path, &extended).unwrap();

    let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(&extended).unwrap();
    let PreparedOpenV0Upgrade::Eligible(prepared) =
        crate::workspace_ops::merge::prepare_upgrade(backend, root, &decoded, "r3-test-writer")
            .unwrap()
    else {
        panic!("unknown-bearing eligible record became unlisted");
    };
    prepared.verify_bytes(b"not: [valid").unwrap_err();

    let mut canonical_drift: Value = serde_yaml::from_slice(prepared.bytes()).unwrap();
    canonical_drift["source_ref"] = Value::String("feature/changed".to_owned());
    prepared
        .verify_bytes(serde_yaml::to_string(&canonical_drift).unwrap().as_bytes())
        .unwrap_err();

    let mut unknown_loss: Value = serde_yaml::from_slice(prepared.bytes()).unwrap();
    unknown_loss
        .as_mapping_mut()
        .unwrap()
        .remove(Value::String("future_atomic_top".to_owned()));
    prepared
        .verify_bytes(serde_yaml::to_string(&unknown_loss).unwrap().as_bytes())
        .unwrap_err();

    upgrade(backend, root, &record.merge_id, AtomicUpgradeFault::None).unwrap();
    let published: Value = serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(published["future_atomic_top"], "top");
    assert_eq!(published["baseline"]["future_atomic_baseline"], "baseline");
    assert_eq!(
        published["participants"][&record.selected_targets[0]]["future_atomic_participant"],
        "participant"
    );
    if has_publication {
        assert_eq!(
            published["publication"]["future_atomic_publication"],
            "publication"
        );
        if has_candidate {
            assert_eq!(
                published["publication"]["candidate"]["future_atomic_candidate"],
                "candidate"
            );
        }
        if has_hash {
            assert_eq!(
                published["publication"]["candidate_hashes"][0]["future_atomic_hash"],
                "hash"
            );
        }
    }
    fs::write(path, source).unwrap();
}

fn assert_v1_restart(bytes: &[u8], merge_id: &str, expected_next_action: &str) {
    let decoded = crate::workspace_ops::merge::decode_v1_for_r3_tests(bytes).unwrap();
    assert_eq!(decoded.record.merge_id, merge_id);
    assert_eq!(
        crate::workspace_ops::merge::finalization_next_action_for_v1(&decoded.record).unwrap(),
        expected_next_action
    );
}

fn observe_repositories<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> Vec<RepoObservation> {
    std::iter::once(root.to_owned())
        .chain(
            record
                .participants
                .values()
                .map(|participant| root.join(&participant.path)),
        )
        .map(|path| RepoObservation {
            head: backend.head(&path).unwrap(),
            status: backend.status(&path).unwrap(),
            path,
        })
        .collect()
}

fn upgrade<B: GitBackend>(
    backend: &B,
    root: &Path,
    merge_id: &str,
    fault: AtomicUpgradeFault,
) -> crate::model::ModelResult<AtomicUpgradeOutcome> {
    crate::workspace_ops::merge::upgrade_open_v0_for_r3_tests(
        backend,
        root,
        merge_id,
        "r3-test-writer",
        fault,
    )
}

fn open_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(format!(".gwz/merge/{merge_id}.yaml"))
}

fn temporary_files(path: &Path) -> Vec<PathBuf> {
    let prefix = format!("{}.", path.file_name().unwrap().to_string_lossy());
    let mut files = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            let name = candidate.file_name().unwrap().to_string_lossy();
            name.starts_with(&prefix) && name.ends_with(".upgrade.tmp")
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn remove_temporary_files(path: &Path) {
    for temporary in temporary_files(path) {
        fs::remove_file(temporary).unwrap();
    }
}
