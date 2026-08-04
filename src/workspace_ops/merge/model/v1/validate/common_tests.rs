use std::collections::BTreeMap;

use super::super::super::MergeTargetKind;
use super::super::*;
use super::tests::{digest, oid, participant, record, sha};
use super::validate_common_v1_record;
use crate::artifact::ArtifactSourceKind;
use crate::model::ErrorCode;

fn assert_unreadable(record: &MergeOperationRecordV1, contains: &str) {
    let error = validate_common_v1_record(record).unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecordUnreadable);
    assert!(error.message.contains(contains), "{:?}", error.message);
}

#[test]
fn minimal_selected_member_shape_is_valid() {
    validate_common_v1_record(&record()).unwrap();
}

#[test]
fn envelope_and_identity_contradictions_are_rejected() {
    let mut case = record();
    case.schema = "gwz.merge-operation/v0".to_owned();
    assert_unreadable(&case, "envelope");
    let mut case = record();
    case.workspace_id = "not-portable".to_owned();
    assert_unreadable(&case, "workspace_id");
    let mut case = record();
    case.merge_id = "../escape".to_owned();
    assert_unreadable(&case, "merge_id");
    let mut case = record();
    case.selected_targets.push("mem_a".to_owned());
    assert_unreadable(&case, "duplicated");
    let mut case = record();
    case.participants.remove("mem_a");
    assert_unreadable(&case, "has no participant");
    let mut case = record();
    case.selected_targets.clear();
    case.participants.clear();
    assert_unreadable(&case, "target set is empty");
    let mut case = record();
    case.participants.insert(
        "mem_extra".to_owned(),
        participant("extra", MergeTargetKind::Member),
    );
    assert_unreadable(&case, "do not equal selected targets");
}

#[test]
fn target_path_branch_oid_and_message_contradictions_are_rejected() {
    let mut case = record();
    case.participants.get_mut("mem_a").unwrap().path = "../outside".to_owned();
    assert_unreadable(&case, "path is invalid");
    let mut case = record();
    case.participants.get_mut("mem_a").unwrap().target_branch = "refs/heads/main".to_owned();
    assert_unreadable(&case, "short local branch");
    let mut case = record();
    case.participants.get_mut("mem_a").unwrap().before_commit = "ABC".repeat(14);
    assert_unreadable(&case, "object id");
    let mut case = record();
    case.baseline.lock_sha256 = "A".repeat(64);
    assert_unreadable(&case, "lowercase SHA-256");
    let mut case = record();
    case.participants.get_mut("mem_a").unwrap().commit_message = "merge topic".to_owned();
    assert_unreadable(&case, "commit message");
}

#[test]
fn exact_root_identity_and_baseline_authority_are_required() {
    let mut case = record();
    case.selected_targets = vec!["@root".to_owned()];
    case.participants =
        BTreeMap::from([("@root".to_owned(), participant(".", MergeTargetKind::Root))]);
    case.participants.get_mut("@root").unwrap().before_commit = oid('c');
    case.baseline.lock_commit_sha256 = Some(sha('4'));
    case.baseline.manifest_commit_sha256 = Some(sha('5'));
    validate_common_v1_record(&case).unwrap();

    let mut case = record();
    case.baseline.lock_yaml = None;
    assert_unreadable(&case, "baseline lock bytes are missing");
    let mut case = record();
    case.baseline.manifest_yaml = Some("invalid".to_owned());
    case.baseline.manifest_sha256 = digest("invalid");
    assert_unreadable(&case, "baseline manifest bytes are invalid");
    let mut case = record();
    let mut root = case.participants["mem_a"].clone();
    root.path = ".".to_owned();
    root.target_kind = MergeTargetKind::Root;
    case.selected_targets = vec!["@root".to_owned(), "mem_a".to_owned()];
    case.participants.insert("@root".to_owned(), root);
    assert_unreadable(&case, "selected root is not the final target");
    let mut case = record();
    case.participants.get_mut("mem_a").unwrap().path = "members/other".to_owned();
    assert_unreadable(&case, "baseline identity changed");
}

#[test]
fn accepted_member_portable_fields_are_checked() {
    let mut case = record();
    case.accepted_workspace = Some(AcceptedWorkspaceV1 {
        operation_baseline_lock_sha256: sha('1'),
        metadata_base: AcceptedMetadataBaseV1 {
            source: AcceptedMetadataSourceV1::OperationBaseline,
            manifest_exact_yaml: "manifest".to_owned(),
            manifest_sha256: sha('2'),
            lock_exact_yaml: "lock".to_owned(),
            lock_sha256: sha('1'),
        },
        lock: AcceptedLockV1 {
            exact_yaml: "lock".to_owned(),
            sha256: sha('3'),
        },
        member_audit: BTreeMap::from([(
            "mem_a".to_owned(),
            MemberAcceptanceV1::UnselectedPresent {
                lock_member: AcceptedLockMemberV1 {
                    path: "members/a".to_owned(),
                    source_id: "src_a".to_owned(),
                    source_kind: ArtifactSourceKind::Git,
                    commit: Some(oid('a')),
                    branch: Some("main".to_owned()),
                    detached: Some(false),
                    upstream: None,
                    dirty: Some(false),
                    materialized: Some(true),
                    extensions: BTreeMap::new(),
                },
            },
        )]),
        root: RootPublicationInputV1 {
            base: AcceptedRootBaseV1::BornAttached {
                commit: oid('c'),
                symbolic_branch: "main".to_owned(),
            },
            publication_branch: Some("main".to_owned()),
            baseline_artifact_hashes: RootArtifactHashesV1 {
                lock_worktree_sha256: sha('1'),
                manifest_worktree_sha256: sha('2'),
                lock_commit_sha256: None,
                manifest_commit_sha256: None,
            },
        },
    });
    validate_common_v1_record(&case).unwrap();
    let MemberAcceptanceV1::UnselectedPresent { lock_member } = case
        .accepted_workspace
        .as_mut()
        .unwrap()
        .member_audit
        .get_mut("mem_a")
        .unwrap()
    else {
        panic!("accepted member row missing")
    };
    lock_member.source_id = "invalid".to_owned();
    assert_unreadable(&case, "source_id");
}

#[test]
fn journal_object_id_algorithm_length_is_checked() {
    let mut case = record();
    case.pending_preservation = Some(PendingPreservationActionV1::Stash {
        owner: PreservationOwnerV1::Participant {
            member_id: "mem_a".to_owned(),
        },
        phase: PreservationStashPhaseV1::WriteBundle,
        stash_id: Some("stash_merge_1".to_owned()),
        stash_object_id: Some(GitObjectIdV1 {
            algorithm: GitObjectAlgorithmV1::Sha1,
            digest_hex: "a".repeat(40),
        }),
        message: "gwz:stash_merge_1: merge preservation".to_owned(),
        head_commit: oid('a'),
        preimage_sha256: sha('1'),
        root_publication_prefix: None,
    });
    validate_common_v1_record(&case).unwrap();
    let Some(PendingPreservationActionV1::Stash {
        stash_object_id: Some(object_id),
        ..
    }) = case.pending_preservation.as_mut()
    else {
        panic!("test stash journal missing")
    };
    object_id.digest_hex.push('a');
    assert_unreadable(&case, "object id");
}
