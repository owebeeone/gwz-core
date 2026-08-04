use super::super::decode_archived_for_r3_tests;
use super::fixtures::{MERGE_ID, Shape, bytes, oid, v0_record};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::PreservationEvidence;

#[test]
fn cleanup_worklist_is_archive_derived_sorted_and_non_authorizing() {
    let mut record = v0_record(Shape::CompletedCandidate);
    record.participants.get_mut("mem_a").unwrap().preservation = vec![PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/mem_a/head")),
        backup_commit: Some(oid('d')),
        stash_id: Some(format!("stash_{MERGE_ID}")),
        stash_object_id: Some(oid('e')),
    }];
    let decoded = decode_archived_for_r3_tests(&bytes(&record), MERGE_ID).unwrap();
    assert_eq!(decoded.cleanup.backup_refs.len(), 1);
    assert_eq!(decoded.cleanup.backup_refs[0].target_id, "mem_a");
    assert_eq!(decoded.cleanup.backup_refs[0].path, "members/a");
    assert!(decoded.cleanup.has_stash_evidence);
}

#[test]
fn cleanup_rejects_noncanonical_incomplete_and_duplicate_owners() {
    let mut wrong = v0_record(Shape::CompletedCandidate);
    wrong.participants.get_mut("mem_a").unwrap().preservation = vec![PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/other/mem_a/head".to_owned()),
        backup_commit: Some(oid('d')),
        stash_id: None,
        stash_object_id: None,
    }];
    let error = decode_archived_for_r3_tests(&bytes(&wrong), MERGE_ID).unwrap_err();
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert!(
        error
            .message
            .ends_with("archive preservation ref is outside the canonical merge-owned namespace")
    );

    let mut incomplete = v0_record(Shape::CompletedCandidate);
    incomplete
        .participants
        .get_mut("mem_a")
        .unwrap()
        .preservation = vec![PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/mem_a/head")),
        backup_commit: None,
        stash_id: None,
        stash_object_id: None,
    }];
    assert!(decode_archived_for_r3_tests(&bytes(&incomplete), MERGE_ID).is_err());

    let mut duplicate = v0_record(Shape::CompletedCandidate);
    let row = PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/mem_a/head")),
        backup_commit: Some(oid('d')),
        stash_id: None,
        stash_object_id: None,
    };
    duplicate
        .participants
        .get_mut("mem_a")
        .unwrap()
        .preservation = vec![row.clone(), row];
    let error = decode_archived_for_r3_tests(&bytes(&duplicate), MERGE_ID).unwrap_err();
    assert!(
        error
            .message
            .ends_with("archive contains duplicate or colliding preservation owners")
    );

    let mut collision = v0_record(Shape::CompletedCandidate);
    let root_row = PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/root/head")),
        backup_commit: Some(oid('d')),
        stash_id: None,
        stash_object_id: None,
    };
    let mut root = collision.participants["mem_a"].clone();
    root.path = ".".to_owned();
    root.preservation = vec![root_row.clone()];
    collision.participants.insert("@root".to_owned(), root);
    collision.publication.as_mut().unwrap().root_preservation = vec![root_row];
    assert!(super::super::cleanup::from_v0(&collision).is_err());
}

#[test]
fn cleanup_rejects_empty_or_noncanonical_stash_evidence() {
    for evidence in [
        PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: None,
            stash_object_id: None,
        },
        PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: Some("stash_wrong".to_owned()),
            stash_object_id: Some(oid('e')),
        },
        PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: Some(format!("stash_{MERGE_ID}")),
            stash_object_id: Some("not-an-oid".to_owned()),
        },
    ] {
        let mut record = v0_record(Shape::CompletedCandidate);
        record.participants.get_mut("mem_a").unwrap().preservation = vec![evidence];
        let error = decode_archived_for_r3_tests(&bytes(&record), MERGE_ID).unwrap_err();
        assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
        assert!(
            error
                .message
                .ends_with("archive envelope or terminal state is contradictory")
        );
    }
}
