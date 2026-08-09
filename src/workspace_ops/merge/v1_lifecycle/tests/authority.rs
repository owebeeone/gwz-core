use super::super::authority::*;
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::{DriftTransition, RecoveryTransition, V1Transition, prepare};
use crate::artifact::{LOCK_PATH, ManifestArtifact};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::model::v1::{
    PreservationOwnerV1, PreservationStashPhaseV1, test_record as record,
};
use crate::workspace_ops::merge::{
    MergeTargetKind, OperationDrift, OperationDriftKind, OperationState, ParticipantDrift,
    ParticipantDriftKind, ParticipantState,
};
use crate::workspace_ops::tests::TempDir;
use sha2::{Digest, Sha256};
use std::fs;

fn checked(name: &str) -> (TempDir, StoredV1Record) {
    let root = TempDir::new(name);
    let stored = StoredV1Record::for_test(&root.path, record()).unwrap();
    (root, stored)
}

#[test]
fn bound_payload_rejects_stale_record_and_value_tampering() {
    let (root, stored) = checked("merge-v1-proof-binding");
    let mut decision =
        BoundPublicationDecision::for_test(&stored, "@publication", "classify", "required", true)
            .unwrap();
    assert!(decision.matches(&stored, "@publication", "classify", "required"));
    decision.corrupt_payload_for_test(false);
    assert!(!decision.matches(&stored, "@publication", "classify", "required"));

    let mut changed = record();
    changed.writer_version = "stale".into();
    let stale = StoredV1Record::for_test(&root.path, changed).unwrap();
    assert!(!VerifiedParticipants::for_test(
        &stored,
        "@operation",
        "enter_finalizing",
        "executing",
        (),
    )
    .unwrap()
    .matches(&stale, "@operation", "enter_finalizing", "executing"));
}

#[test]
fn entries_bind_handoff_anticipated_model_and_preservation_exhaustion() {
    let (_root, stored) = checked("merge-v1-entry-binding");
    let handoff = || {
        VerifiedPublicationHandoff::for_test(&stored, "@publication", "handoff", "verified", ())
            .unwrap()
    };
    let preservation =
        PreparedPreservationEntry::for_test(&stored, stored.record(), handoff()).unwrap();
    assert!(preservation.matches(&stored, "@operation", "begin_preservation", "preflight"));
    assert!(preservation.anticipated_model_matches(stored.record()));

    let direct =
        PreparedRollbackEntry::direct_for_test(&stored, stored.record(), handoff()).unwrap();
    assert_eq!(direct.origin(), RollbackEntryOrigin::Direct);
    let exhausted = VerifiedPreservationExhausted::for_test(
        &stored,
        "@operation",
        "preservation_exhausted",
        "verified",
        (),
    )
    .unwrap();
    assert_eq!(exhausted.value(), &());
    let reverse = PreparedRollbackEntry::from_preserving_for_test(
        &stored,
        stored.record(),
        handoff(),
        exhausted,
    )
    .unwrap();
    assert_eq!(reverse.origin(), RollbackEntryOrigin::FromPreserving);
    assert!(reverse.matches(&stored, "@operation", "begin_rollback", "preflight"));
}

#[test]
fn preservation_action_proof_embeds_live_cursor_prefix() {
    let (_root, stored) = checked("merge-v1-preservation-prefix");
    let owner = PreservationOwnerV1::Participant {
        member_id: "mem_a".into(),
    };
    let position = PreservationCursorPosition::Stash(PreservationStashPhaseV1::CreateStash);
    let prefix = VerifiedPreservationCursorPrefix::for_test(
        &stored,
        "mem_a",
        "preservation_cursor",
        "prefix_verified",
        PreservationCursorPrefix {
            owner: owner.clone(),
            position,
        },
    )
    .unwrap();
    let intent = PreparedStashIntent::for_test(
        &stored,
        "mem_a",
        "begin_preservation",
        "cursor_checked",
        PreservationPayload {
            owner: owner.clone(),
            observed_position: position,
            pending: None,
            evidence: None,
            publication_prefix: None,
        },
        prefix,
    )
    .unwrap();
    assert!(intent.matches(&stored, "mem_a", "begin_preservation", "cursor_checked"));

    let wrong_prefix = VerifiedPreservationCursorPrefix::for_test(
        &stored,
        "mem_a",
        "preservation_cursor",
        "prefix_verified",
        PreservationCursorPrefix {
            owner: owner.clone(),
            position: PreservationCursorPosition::BackupRef,
        },
    )
    .unwrap();
    let mismatched = PreparedStashIntent::for_test(
        &stored,
        "mem_a",
        "begin_preservation",
        "cursor_checked",
        PreservationPayload {
            owner,
            observed_position: position,
            pending: None,
            evidence: None,
            publication_prefix: None,
        },
        wrong_prefix,
    )
    .unwrap();
    assert!(!mismatched.matches(&stored, "mem_a", "begin_preservation", "cursor_checked"));
}

#[test]
fn rollback_observers_consume_the_shared_exact_cursor() {
    let root = TempDir::new("merge-v1-rollback-cursor-authority");
    let mut current = record();
    current.state = OperationState::RollingBack;
    let mut manifest =
        ManifestArtifact::from_yaml(current.baseline.manifest_yaml.as_deref().unwrap()).unwrap();
    let mut member = manifest.members[0].clone();
    member.id = "mem_b".into();
    member.path = "members/b".into();
    member.source_id = "src_b".into();
    manifest.members.push(member);
    let manifest = manifest.to_yaml().unwrap();
    current.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest.as_bytes()));
    current.baseline.manifest_yaml = Some(manifest);
    let mut second = current.participants["mem_a"].clone();
    second.path = "members/b".into();
    current.participants.insert("mem_b".into(), second);
    current.selected_targets.push("mem_b".into());

    let stored = StoredV1Record::for_test(&root.path, current).unwrap();
    let proof = no_mutation_abort(&stored).unwrap();
    assert_eq!(proof.value(), "mem_b");
    assert!(proof.matches(
        &stored,
        "mem_b",
        "record_no_mutation_abort",
        "cursor_verified"
    ));

    let mut next = stored.record().clone();
    next.participants.get_mut("mem_b").unwrap().state = ParticipantState::Aborted;
    let next = StoredV1Record::for_test(&root.path, next).unwrap();
    assert_eq!(no_mutation_abort(&next).unwrap().value(), "mem_a");
    assert!(rollback_exhausted(&next).is_err());

    let mut complete = next.record().clone();
    complete.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
    let complete = StoredV1Record::for_test(&root.path, complete).unwrap();
    assert!(rollback_exhausted(&complete).is_ok());
}

#[test]
fn selected_root_exhaustion_requires_the_exact_live_baseline() {
    let root = TempDir::new("merge-v1-selected-root-exhaustion");
    let mut current = record();
    current.state = OperationState::RollingBack;
    let mut selected_root = current.participants["mem_a"].clone();
    selected_root.path = ".".into();
    selected_root.target_kind = MergeTargetKind::Root;
    selected_root.before_commit = current.baseline.root_head.clone().unwrap();
    selected_root.state = ParticipantState::Aborted;
    current.selected_targets = vec!["@root".into()];
    current.participants.clear();
    current.participants.insert("@root".into(), selected_root);
    current.baseline.lock_commit_sha256 = Some("4".repeat(64));
    current.baseline.manifest_commit_sha256 = Some("5".repeat(64));

    let manifest = current.baseline.manifest_yaml.clone().unwrap();
    let lock = current.baseline.lock_yaml.clone().unwrap();
    fs::create_dir_all(root.path.join("gwz.conf")).unwrap();
    fs::write(root.path.join(WORKSPACE_MANIFEST), &manifest).unwrap();
    fs::write(root.path.join(LOCK_PATH), &lock).unwrap();
    let stored = StoredV1Record::for_test(&root.path, current).unwrap();
    assert!(rollback_exhausted(&stored).is_ok());

    fs::write(root.path.join(WORKSPACE_MANIFEST), "changed").unwrap();
    assert!(rollback_exhausted(&stored).is_err());
    fs::write(root.path.join(WORKSPACE_MANIFEST), &manifest).unwrap();
    fs::write(root.path.join(LOCK_PATH), "changed").unwrap();
    assert!(rollback_exhausted(&stored).is_err());
}

#[test]
pub(super) fn recovery_and_drift_proofs_drive_only_their_exact_reducers() {
    let root = TempDir::new("merge-v1-recovery-drift-authority");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let current = StoredV1Record::for_test(&root.path, record()).unwrap();
    let ambiguity = BoundAmbiguityEvidence::for_test(
        &current,
        "@operation",
        "enter_recovery",
        "ambiguous",
        crate::workspace_ops::merge::model::v1::RecoveryOriginStateV1::Executing,
    )
    .unwrap();
    let recovery = prepare(
        &lease,
        &current,
        V1Transition::Recovery(Box::new(RecoveryTransition::Enter(ambiguity))),
    )
    .unwrap();
    assert_eq!(recovery.next().state, OperationState::RecoveryRequired);
    let recovery = StoredV1Record::for_test(&root.path, recovery.next().clone()).unwrap();
    let resume = VerifiedRecoveryOrigin::for_test(
        &recovery,
        "@operation",
        "resume_recovery",
        "verified",
        crate::workspace_ops::merge::model::v1::RecoveryOriginStateV1::Executing,
    )
    .unwrap();
    assert_eq!(
        prepare(
            &lease,
            &recovery,
            V1Transition::Recovery(Box::new(RecoveryTransition::Resume(resume))),
        )
        .unwrap()
        .next()
        .state,
        OperationState::Executing
    );

    let participant_drift = ParticipantDrift {
        kind: ParticipantDriftKind::HeadAdvanced,
        message: "advanced".into(),
        expected_branch: None,
        live_branch: None,
        expected_head: None,
        live_head: None,
        expected_merge_head: None,
        live_merge_head: None,
    };
    let participant = ParticipantDriftPayload {
        member_id: "mem_a".into(),
        identity: ParticipantDriftIdentity::new(&participant_drift, 0),
        drift: participant_drift,
    };
    let fact = BoundParticipantDrift::for_test(
        &current,
        "mem_a",
        "record_drift",
        "observed",
        participant.clone(),
    )
    .unwrap();
    let drifted = prepare(
        &lease,
        &current,
        V1Transition::Drift(Box::new(DriftTransition::RecordParticipant(Box::new(fact)))),
    )
    .unwrap();
    assert_eq!(drifted.next().participants["mem_a"].drift.len(), 1);
    let drifted = StoredV1Record::for_test(&root.path, drifted.next().clone()).unwrap();
    let clear = VerifiedParticipantDriftClear::for_test(
        &drifted,
        "mem_a",
        "clear_drift",
        "verified",
        participant,
    )
    .unwrap();
    assert!(
        prepare(
            &lease,
            &drifted,
            V1Transition::Drift(Box::new(DriftTransition::ClearParticipant(Box::new(clear)))),
        )
        .unwrap()
        .next()
        .participants["mem_a"]
            .drift
            .is_empty()
    );

    let operation = OperationDrift {
        kind: OperationDriftKind::BaselineLockChanged,
        message: "changed".into(),
    };
    let fact = BoundOperationDrift::for_test(
        &current,
        "@operation",
        "record_drift",
        "observed",
        operation.clone(),
    )
    .unwrap();
    let drifted = prepare(
        &lease,
        &current,
        V1Transition::Drift(Box::new(DriftTransition::RecordOperation(fact))),
    )
    .unwrap();
    assert_eq!(drifted.next().operation_drift.len(), 1);
    let drifted = StoredV1Record::for_test(&root.path, drifted.next().clone()).unwrap();
    let clear = VerifiedOperationDriftClear::for_test(
        &drifted,
        "@operation",
        "clear_drift",
        "verified",
        operation,
    )
    .unwrap();
    assert!(
        prepare(
            &lease,
            &drifted,
            V1Transition::Drift(Box::new(DriftTransition::ClearOperation(clear))),
        )
        .unwrap()
        .next()
        .operation_drift
        .is_empty()
    );
}
