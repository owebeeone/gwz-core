use super::super::super::{OperationState, ParticipantState, PreservationEvidence};
use super::super::{
    GitObjectAlgorithmV1, GitObjectIdV1, PendingPreservationActionV1, PreservationOwnerV1,
    PreservationStashPhaseV1,
};
use super::tests::{oid, record, sha};
use super::validate_v1_preservation;
use crate::model::ErrorCode;

#[test]
fn non_root_owner_cannot_carry_a_root_publication_prefix() {
    let mut case = record();
    case.state = OperationState::Preserving;
    let participant = case.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::Merged;
    participant.resulting_commit = Some(oid('c'));
    participant.preservation.push(PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/merge_1/mem_a/head".to_owned()),
        backup_commit: Some(oid('c')),
        stash_id: None,
        stash_object_id: None,
    });
    case.pending_preservation = Some(PendingPreservationActionV1::Stash {
        owner: PreservationOwnerV1::Participant {
            member_id: "mem_a".to_owned(),
        },
        phase: PreservationStashPhaseV1::NormalizeRoot,
        stash_id: None,
        stash_object_id: None,
        message: "gwz:stash_merge_1: merge preservation".to_owned(),
        head_commit: oid('c'),
        preimage_sha256: sha('1'),
        root_publication_prefix: Some(super::super::PublicationPrefixV1::Boundary),
    });
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn preservation_rows_require_one_canonical_complete_owner_row() {
    let mut case = record();
    let participant = case.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::Merged;
    participant.resulting_commit = Some(oid('c'));
    participant.preservation.push(PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/other/mem_a/head".to_owned()),
        backup_commit: Some(oid('c')),
        stash_id: None,
        stash_object_id: None,
    });
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn selected_root_and_publication_root_preservation_owners_cannot_collide() {
    let mut case = super::acceptance_tests::selected_acceptance_record_for_tests();
    case.publication = Some(super::acceptance_tests::valid_candidate_publication_for_tests(&case));
    case.publication.as_mut().unwrap().composition_commit = Some(oid('e'));

    let mut root = case.participants["mem_a"].clone();
    root.path = ".".to_owned();
    root.target_kind = super::super::super::MergeTargetKind::Root;
    root.resulting_commit = Some(oid('e'));
    root.preservation.push(PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/merge_1/root/head".to_owned()),
        backup_commit: Some(oid('e')),
        stash_id: None,
        stash_object_id: None,
    });
    case.selected_targets.push("@root".to_owned());
    case.participants.insert("@root".to_owned(), root);
    case.publication
        .as_mut()
        .unwrap()
        .root_preservation
        .push(PreservationEvidence {
            backup_ref: Some("refs/gwz/merge/merge_1/root/head".to_owned()),
            backup_commit: Some(oid('e')),
            stash_id: None,
            stash_object_id: None,
        });

    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn pending_stash_result_must_equal_the_stable_owner_evidence_row() {
    let mut case = record();
    case.state = OperationState::Preserving;
    let participant = case.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::Merged;
    participant.resulting_commit = Some(oid('c'));
    participant.preservation.push(PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/merge_1/mem_a/head".to_owned()),
        backup_commit: Some(oid('c')),
        stash_id: Some("stash_merge_1".to_owned()),
        stash_object_id: Some(oid('d')),
    });
    case.pending_preservation = Some(PendingPreservationActionV1::Stash {
        owner: PreservationOwnerV1::Participant {
            member_id: "mem_a".to_owned(),
        },
        phase: PreservationStashPhaseV1::WriteBundle,
        stash_id: Some("stash_merge_1".to_owned()),
        stash_object_id: Some(GitObjectIdV1 {
            algorithm: GitObjectAlgorithmV1::Sha1,
            digest_hex: oid('d'),
        }),
        message: "gwz:stash_merge_1: merge preservation".to_owned(),
        head_commit: oid('c'),
        preimage_sha256: sha('1'),
        root_publication_prefix: None,
    });
    validate_v1_preservation(&case).unwrap();

    let Some(PendingPreservationActionV1::Stash {
        stash_object_id: Some(object_id),
        ..
    }) = case.pending_preservation.as_mut()
    else {
        panic!("pending stash result missing")
    };
    object_id.digest_hex = oid('e');
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );

    let Some(PendingPreservationActionV1::Stash {
        phase,
        stash_id,
        stash_object_id,
        ..
    }) = case.pending_preservation.as_mut()
    else {
        panic!("pending stash result missing")
    };
    *phase = PreservationStashPhaseV1::CreateStash;
    *stash_id = None;
    *stash_object_id = None;
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn stash_only_owner_does_not_require_a_backup_ref() {
    let mut case = record();
    let participant = case.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::Merged;
    participant.resulting_commit = Some(oid('c'));
    participant.preservation.push(PreservationEvidence {
        backup_ref: None,
        backup_commit: None,
        stash_id: Some("stash_merge_1".to_owned()),
        stash_object_id: Some(oid('d')),
    });
    validate_v1_preservation(&case).unwrap();

    case.participants.get_mut("mem_a").unwrap().resulting_commit = None;
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}
