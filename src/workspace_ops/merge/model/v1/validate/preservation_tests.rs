use super::super::super::{
    MergeTargetKind, OperationState, ParticipantState, PreservationEvidence,
    PublicationCandidateHash, PublicationStep,
};
use super::super::{
    GitObjectAlgorithmV1, GitObjectIdV1, MergeOperationRecordV1, PendingPreservationActionV1,
    PreservationOwnerV1, PreservationPublicationCandidateV1, PreservationPublicationHandoffV1,
    PreservationRefResetPhaseV1, PreservationStashPhaseV1, PublicationIndexFormV1,
    PublicationPrefixV1,
};
use super::tests::{oid, record, sha};
use super::validate_v1_preservation;
use super::{preservation_handoff_is_compatible, validate_v1_publication};
use crate::model::ErrorCode;

#[test]
fn non_root_owner_cannot_carry_a_root_publication_handoff() {
    let mut case = record();
    case.state = OperationState::Preserving;
    case.preservation_publication_handoff = Some(PreservationPublicationHandoffV1::NoCandidate);
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
        phase: PreservationStashPhaseV1::NormalizeMarker,
        stash_id: None,
        stash_object_id: None,
        message: "gwz:stash_merge_1: merge preservation".to_owned(),
        head_commit: oid('c'),
        preimage_sha256: sha('1'),
        root_publication_handoff: Some(PreservationPublicationCandidateV1 {
            prefix: PublicationPrefixV1::Boundary,
            index: PublicationIndexFormV1::Pre,
        }),
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
    case.preservation_publication_handoff = Some(PreservationPublicationHandoffV1::NoCandidate);
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
        root_publication_handoff: None,
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

#[test]
fn durable_handoff_compatibility_matrix_is_closed() {
    let mut case = super::acceptance_tests::selected_acceptance_record_for_tests();
    case.publication = Some(super::acceptance_tests::valid_candidate_publication_for_tests(&case));
    case.publication.as_mut().unwrap().step = PublicationStep::PublishingCandidate;

    for (prefix, index, expected) in [
        (
            PublicationPrefixV1::Baseline,
            PublicationIndexFormV1::Pre,
            true,
        ),
        (
            PublicationPrefixV1::Marker,
            PublicationIndexFormV1::Pre,
            true,
        ),
        (PublicationPrefixV1::Lock, PublicationIndexFormV1::Pre, true),
        (
            PublicationPrefixV1::Boundary,
            PublicationIndexFormV1::Pre,
            true,
        ),
        (
            PublicationPrefixV1::Baseline,
            PublicationIndexFormV1::Staged,
            false,
        ),
        (
            PublicationPrefixV1::Marker,
            PublicationIndexFormV1::Staged,
            false,
        ),
        (
            PublicationPrefixV1::Lock,
            PublicationIndexFormV1::Staged,
            false,
        ),
        (
            PublicationPrefixV1::Boundary,
            PublicationIndexFormV1::Staged,
            true,
        ),
    ] {
        assert_eq!(
            preservation_handoff_is_compatible(&case, candidate(prefix, index)),
            expected,
            "publishing {prefix:?}/{index:?}"
        );
    }

    case.publication.as_mut().unwrap().step = PublicationStep::Complete;
    assert!(preservation_handoff_is_compatible(
        &case,
        candidate(
            PublicationPrefixV1::Boundary,
            PublicationIndexFormV1::Staged,
        ),
    ));
    assert!(!preservation_handoff_is_compatible(
        &case,
        candidate(PublicationPrefixV1::Boundary, PublicationIndexFormV1::Pre,),
    ));

    let candidate_record = case
        .publication
        .as_mut()
        .unwrap()
        .candidate
        .as_mut()
        .unwrap();
    candidate_record.lock_yaml = candidate_record.baseline_lock_yaml.clone();
    candidate_record.boundary_text = candidate_record.baseline_boundary_text.clone();
    assert!(preservation_handoff_is_compatible(
        &case,
        candidate(PublicationPrefixV1::Marker, PublicationIndexFormV1::Staged,),
    ));
}

#[test]
fn noncandidate_and_evidence_pending_handoffs_follow_the_stored_phase() {
    let case = record();
    assert!(preservation_handoff_is_compatible(
        &case,
        PreservationPublicationHandoffV1::NoCandidate,
    ));

    let mut evidence = super::acceptance_tests::selected_acceptance_record_for_tests();
    evidence.publication =
        Some(super::acceptance_tests::valid_candidate_publication_for_tests(&evidence));
    evidence.publication.as_mut().unwrap().step = PublicationStep::CommittingEvidence;
    assert!(preservation_handoff_is_compatible(
        &evidence,
        PreservationPublicationHandoffV1::EvidencePending,
    ));
    assert!(!preservation_handoff_is_compatible(
        &evidence,
        candidate(PublicationPrefixV1::Baseline, PublicationIndexFormV1::Pre,),
    ));

    let publication = evidence.publication.as_mut().unwrap();
    publication.composition_commit = Some(oid('e'));
    publication.composition_tree = Some(oid('f'));
    publication.candidate_hashes = vec![PublicationCandidateHash {
        path: "candidate".into(),
        sha256: sha('1'),
    }];
    assert!(preservation_handoff_is_compatible(
        &evidence,
        candidate(PublicationPrefixV1::Baseline, PublicationIndexFormV1::Pre,),
    ));
}

#[test]
fn durable_handoff_lifetime_and_legacy_prefix_fail_closed() {
    let mut case = record();
    case.preservation_publication_handoff = Some(PreservationPublicationHandoffV1::NoCandidate);
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );

    case.state = OperationState::Preserving;
    validate_v1_preservation(&case).unwrap();
    case.preservation_publication_handoff = None;
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );

    let mut legacy = super::acceptance_tests::selected_acceptance_record_for_tests();
    legacy.publication =
        Some(super::acceptance_tests::valid_candidate_publication_for_tests(&legacy));
    legacy.publication.as_mut().unwrap().preservation_prefix = Some("baseline".into());
    assert_eq!(
        validate_v1_publication(&legacy).unwrap_err().code,
        ErrorCode::UnexpectedPublicationEvidence
    );
}

#[test]
fn both_root_owner_variants_accept_every_exact_flattened_phase() {
    for handoff in [
        candidate(PublicationPrefixV1::Baseline, PublicationIndexFormV1::Pre),
        candidate(
            PublicationPrefixV1::Boundary,
            PublicationIndexFormV1::Staged,
        ),
    ] {
        for selected_root in [false, true] {
            let mut case = super::acceptance_tests::selected_acceptance_record_for_tests();
            case.state = OperationState::Preserving;
            case.publication =
                Some(super::acceptance_tests::valid_candidate_publication_for_tests(&case));
            case.publication.as_mut().unwrap().step = match handoff {
                PreservationPublicationHandoffV1::Candidate {
                    index: PublicationIndexFormV1::Pre,
                    ..
                } => PublicationStep::CommittingEvidence,
                PreservationPublicationHandoffV1::Candidate {
                    index: PublicationIndexFormV1::Staged,
                    ..
                } => PublicationStep::PublishingCandidate,
                _ => unreachable!(),
            };
            case.preservation_publication_handoff = Some(handoff);
            let owner = if selected_root {
                let mut root = case.participants["mem_a"].clone();
                root.path = ".".into();
                root.target_kind = MergeTargetKind::Root;
                root.resulting_commit = Some(oid('e'));
                case.selected_targets.push("@root".into());
                case.participants.insert("@root".into(), root);
                PreservationOwnerV1::Participant {
                    member_id: "@root".into(),
                }
            } else {
                PreservationOwnerV1::PublicationRoot
            };
            let publication = case.publication.as_mut().unwrap();
            publication.composition_commit = Some(oid('e'));
            publication.composition_tree = Some(oid('f'));
            publication.candidate_hashes = vec![PublicationCandidateHash {
                path: "candidate".into(),
                sha256: sha('1'),
            }];
            for phase in [
                PreservationStashPhaseV1::NormalizeParent,
                PreservationStashPhaseV1::NormalizeMarker,
                PreservationStashPhaseV1::NormalizeLock,
                PreservationStashPhaseV1::NormalizeIndex,
                PreservationStashPhaseV1::CreateStash,
                PreservationStashPhaseV1::RestoreIndex,
                PreservationStashPhaseV1::RestoreLock,
                PreservationStashPhaseV1::RestoreParent,
                PreservationStashPhaseV1::RestoreMarker,
                PreservationStashPhaseV1::WriteBundle,
                PreservationStashPhaseV1::Complete,
            ] {
                let mut phase_case = case.clone();
                let ids_present = matches!(
                    phase,
                    PreservationStashPhaseV1::RestoreIndex
                        | PreservationStashPhaseV1::RestoreLock
                        | PreservationStashPhaseV1::RestoreParent
                        | PreservationStashPhaseV1::RestoreMarker
                        | PreservationStashPhaseV1::WriteBundle
                        | PreservationStashPhaseV1::Complete
                );
                add_root_evidence(&mut phase_case, &owner, ids_present);
                phase_case.pending_preservation = Some(PendingPreservationActionV1::Stash {
                    owner: owner.clone(),
                    phase,
                    stash_id: ids_present.then(|| "stash_merge_1".into()),
                    stash_object_id: ids_present.then(|| GitObjectIdV1 {
                        algorithm: GitObjectAlgorithmV1::Sha1,
                        digest_hex: oid('d'),
                    }),
                    message: "gwz:stash_merge_1: merge preservation".into(),
                    head_commit: oid('e'),
                    preimage_sha256: sha('1'),
                    root_publication_handoff: handoff.candidate(),
                });
                validate_v1_preservation(&phase_case).unwrap();
            }

            for phase in [
                PreservationRefResetPhaseV1::PrepareParent,
                PreservationRefResetPhaseV1::PrepareMarker,
                PreservationRefResetPhaseV1::PrepareLock,
                PreservationRefResetPhaseV1::PrepareIndex,
                PreservationRefResetPhaseV1::ResetRef,
                PreservationRefResetPhaseV1::RestoreIndex,
                PreservationRefResetPhaseV1::RestoreLock,
                PreservationRefResetPhaseV1::RestoreParent,
                PreservationRefResetPhaseV1::RestoreMarker,
                PreservationRefResetPhaseV1::Complete,
            ] {
                let mut phase_case = case.clone();
                add_root_evidence(&mut phase_case, &owner, false);
                phase_case.pending_preservation =
                    Some(PendingPreservationActionV1::ResetAttachedRef {
                        owner: owner.clone(),
                        branch: "main".into(),
                        expected_commit: oid('e'),
                        restore_commit: oid('e'),
                        phase,
                        root_publication_handoff: handoff.candidate(),
                    });
                validate_v1_preservation(&phase_case).unwrap();
            }

            let mut wrong_handoff = case;
            add_root_evidence(&mut wrong_handoff, &owner, false);
            wrong_handoff.pending_preservation = Some(PendingPreservationActionV1::Stash {
                owner,
                phase: PreservationStashPhaseV1::NormalizeMarker,
                stash_id: None,
                stash_object_id: None,
                message: "gwz:stash_merge_1: merge preservation".into(),
                head_commit: oid('e'),
                preimage_sha256: sha('1'),
                root_publication_handoff: handoff.candidate(),
            });
            let Some(PendingPreservationActionV1::Stash {
                root_publication_handoff: Some(action_handoff),
                ..
            }) = wrong_handoff.pending_preservation.as_mut()
            else {
                panic!("root action handoff missing")
            };
            action_handoff.index = match action_handoff.index {
                PublicationIndexFormV1::Pre => PublicationIndexFormV1::Staged,
                PublicationIndexFormV1::Staged => PublicationIndexFormV1::Pre,
            };
            assert_eq!(
                validate_v1_preservation(&wrong_handoff).unwrap_err().code,
                ErrorCode::PreservationEvidenceMismatch,
                "selected_root={selected_root}, handoff={handoff:?}"
            );
        }
    }
}

fn add_root_evidence(
    record: &mut MergeOperationRecordV1,
    owner: &PreservationOwnerV1,
    with_stash: bool,
) {
    let rows = match owner {
        PreservationOwnerV1::Participant { member_id } => {
            &mut record.participants.get_mut(member_id).unwrap().preservation
        }
        PreservationOwnerV1::PublicationRoot => {
            &mut record.publication.as_mut().unwrap().root_preservation
        }
    };
    rows.push(PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/merge_1/root/head".into()),
        backup_commit: Some(oid('e')),
        stash_id: with_stash.then(|| "stash_merge_1".into()),
        stash_object_id: with_stash.then(|| oid('d')),
    });
}

fn candidate(
    prefix: PublicationPrefixV1,
    index: PublicationIndexFormV1,
) -> PreservationPublicationHandoffV1 {
    PreservationPublicationHandoffV1::Candidate { prefix, index }
}
