use super::super::authority::PreservationCursorPosition as P;
use super::super::authority::*;
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::{
    DriftTransition, RecoveryTransition, ReverseEntryKind, V1Transition, prepare,
};
use super::fixtures::{oid, up_to_date_action};
use crate::artifact::{LOCK_PATH, ManifestArtifact};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::model::v1::{
    GitObjectAlgorithmV1, GitObjectIdV1, PendingPreservationActionV1, PreservationOwnerV1,
    PreservationPublicationHandoffV1, PreservationStashPhaseV1 as S, PublicationIndexFormV1,
    PublicationPrefixV1, RecoveryOriginStateV1 as Origin, test_record as record,
};
use crate::workspace_ops::merge::{
    MergeTargetKind, OperationDrift, OperationDriftKind, OperationState, ParticipantDrift,
    ParticipantDriftKind, ParticipantState, PendingMergeAction, PreservationEvidence,
};
use crate::workspace_ops::tests::TempDir;
use V1LifecycleRequest::ResumeStart;
use sha2::{Digest, Sha256};
use std::fs;

const PRESERVE: &str = "begin_preservation";
const CURSOR_CHECKED: &str = "cursor_checked";

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
fn every_authority_binding_rejects_an_identical_record_from_another_root() {
    let first_root = TempDir::new("merge-v1-binding-location-first");
    let second_root = TempDir::new("merge-v1-binding-location-second");
    let mut model = record();
    let action = up_to_date_action();
    model.participants.get_mut("mem_a").unwrap().pending_action = Some(action.clone());
    let first = StoredV1Record::for_test(&first_root.path, model.clone()).unwrap();
    let second = StoredV1Record::for_test(&second_root.path, model).unwrap();

    let token =
        VerifiedParticipants::for_test(&first, "@operation", "enter_finalizing", "executing", ())
            .unwrap();
    assert!(token.matches(&first, "@operation", "enter_finalizing", "executing"));
    assert!(!token.matches(&second, "@operation", "enter_finalizing", "executing"));

    let request_first = participant_request(&first);
    let observation_first = not_started_participant_observation(&first, &request_first, &action);
    assert!(
        resolve_participant(&second, request_first, observation_first, None).is_err(),
        "the request binding must reject the second root",
    );

    let request_first = participant_request(&first);
    let observation_first = not_started_participant_observation(&first, &request_first, &action);
    let request_second = participant_request(&second);
    assert!(
        resolve_participant(&second, request_second, observation_first, None).is_err(),
        "the exact-observation binding must reject the second root",
    );

    let physical = prepared_participant_action(&first, &action);
    assert!(physical.authorize(&first).is_ok());
    assert!(physical.authorize(&second).is_err());

    let attempt = prepared_participant_action(&first, &action)
        .record_attempt(&first, ExecutionDiagnostic::Success)
        .unwrap();
    let request_second = participant_request(&second);
    let observation_second = ambiguous_participant_observation(&second, &request_second);
    assert!(
        resolve_participant(&second, request_second, observation_second, Some(attempt)).is_err(),
        "the execution-attempt binding must reject the second root",
    );

    let attempt = prepared_participant_action(&first, &action)
        .record_attempt(&first, ExecutionDiagnostic::Success)
        .unwrap();
    let request_first = participant_request(&first);
    let observation_first = ambiguous_participant_observation(&first, &request_first);
    assert!(resolve_participant(&first, request_first, observation_first, Some(attempt)).is_ok());
}

fn resolve_participant(
    current: &StoredV1Record,
    request: BoundObservationRequest,
    observation: BoundExactObservation,
    attempt: Option<BoundExecutionAttempt>,
) -> crate::model::ModelResult<ResolvedV1Action> {
    resolve_observation(current, ResumeStart, request, observation, attempt)
}

fn participant_request(current: &StoredV1Record) -> BoundObservationRequest {
    BoundObservationRequest::for_test(
        current,
        ResumeStart,
        ObservationKind::ParticipantAction {
            member_id: "mem_a".into(),
        },
    )
    .unwrap()
}

fn not_started_participant_observation(
    current: &StoredV1Record,
    request: &BoundObservationRequest,
    action: &PendingMergeAction,
) -> BoundExactObservation {
    BoundExactObservation::for_test(
        current,
        request,
        ExactObservationFact::NotStarted(NotStartedObservation::Participant {
            member_id: "mem_a".into(),
            action: Box::new(action.clone()),
        }),
    )
    .unwrap()
}

fn prepared_participant_action(
    current: &StoredV1Record,
    action: &PendingMergeAction,
) -> Box<BoundPhysicalAction> {
    let request = participant_request(current);
    let observation = not_started_participant_observation(current, &request, action);
    match resolve_participant(current, request, observation, None).unwrap() {
        ResolvedV1Action::Execute(action) => action,
        _ => panic!("participant observation did not produce a physical action"),
    }
}

fn ambiguous_participant_observation(
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> BoundExactObservation {
    let ambiguity = BoundAmbiguityEvidence::for_test(
        current,
        "@operation",
        "enter_recovery",
        "ambiguous",
        Origin::Executing,
    )
    .unwrap();
    BoundExactObservation::for_test(current, request, ExactObservationFact::Ambiguous(ambiguity))
        .unwrap()
}

#[test]
fn entries_bind_handoff_anticipated_model_and_preservation_exhaustion() {
    let (_root, stored) = checked("merge-v1-entry-binding");
    let handoff =
        |kind| VerifiedPublicationHandoff::for_entry_test(&stored, kind, stored.record()).unwrap();
    let preservation = PreparedPreservationEntry::for_test(
        &stored,
        stored.record(),
        handoff(ReverseEntryKind::Preservation),
    )
    .unwrap();
    assert!(preservation.matches(&stored, "@operation", "begin_preservation", "preflight"));
    assert!(preservation.anticipated_model_matches(stored.record()));

    let direct = PreparedRollbackEntry::direct_for_test(
        &stored,
        stored.record(),
        handoff(ReverseEntryKind::DirectRollback),
    )
    .unwrap();
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
        handoff(ReverseEntryKind::ExhaustedRollback),
        exhausted,
    )
    .unwrap();
    assert_eq!(reverse.origin(), RollbackEntryOrigin::FromPreserving);
    assert!(reverse.matches(&stored, "@operation", "begin_rollback", "preflight"));
}

#[test]
#[rustfmt::skip]
fn root_preservation_owner_binding_matrix_is_closed() {
    let (_root, stored) = checked("merge-v1-preservation-binding");
    for (owner, owner_id) in [
        (participant_owner("mem_a"), "mem_a"),
        (participant_owner("@root"), "@root"),
        (PreservationOwnerV1::PublicationRoot, "@publication-root"),
    ] {
        let exact = payload(owner.clone(), S::RestoreParent, true);
        let intent = stash_intent(&stored, owner_id, exact.clone(), exact.observed_position);
        assert!(intent.matches(&stored, owner_id, PRESERVE, CURSOR_CHECKED));
        let wrong_prefix = stash_intent(&stored, owner_id, exact.clone(), P::BackupRef);
        assert!(!wrong_prefix.matches(&stored, owner_id, PRESERVE, CURSOR_CHECKED));
        assert!(!intent.matches(&stored, "wrong-owner", PRESERVE, CURSOR_CHECKED));
        assert!(!intent.matches(&stored, owner_id, "wrong-action", CURSOR_CHECKED));
        assert!(!intent.matches(&stored, owner_id, PRESERVE, "wrong-phase"));
        assert_ne!(hash(&exact), hash(&payload(owner.clone(), S::RestoreMarker, true)));
        assert_ne!(hash(&exact), hash(&payload(owner, S::RestoreParent, false)));
    }
}

#[rustfmt::skip]
fn participant_owner(member_id: &str) -> PreservationOwnerV1 { PreservationOwnerV1::Participant { member_id: member_id.into() } }

#[rustfmt::skip]
fn hash(payload: &PreservationPayload) -> [u8; 32] { payload_hash(payload).unwrap() }

#[rustfmt::skip]
fn stash_intent(current: &StoredV1Record, owner: &str, payload: PreservationPayload,
    prefix_position: P) -> PreparedStashIntent {
    let prefix = VerifiedPreservationCursorPrefix::for_test(
        current, owner, "preservation_cursor", "prefix_verified",
        PreservationCursorPrefix { owner: payload.owner.clone(), position: prefix_position },
    ).unwrap();
    PreparedStashIntent::for_test(current, owner, PRESERVE, CURSOR_CHECKED, payload, prefix).unwrap()
}

#[rustfmt::skip]
fn payload(owner: PreservationOwnerV1, phase: S, exact_goal: bool) -> PreservationPayload {
    let position = P::Stash(phase);
    let (prefix, index) = if exact_goal {
        (PublicationPrefixV1::Boundary, PublicationIndexFormV1::Staged)
    } else {
        (PublicationPrefixV1::Baseline, PublicationIndexFormV1::Pre)
    };
    PreservationPayload {
        owner: owner.clone(), observed_position: position,
        pending: Some(PendingPreservationActionV1::Stash {
            owner, phase, stash_id: Some("stash_merge_1".into()),
            stash_object_id: Some(GitObjectIdV1 {
                algorithm: GitObjectAlgorithmV1::Sha1, digest_hex: oid('b'),
            }),
            message: "gwz:stash_merge_1: merge preservation".into(), head_commit: oid('e'),
            preimage_sha256: "1".repeat(64), root_publication_handoff:
                PreservationPublicationHandoffV1::Candidate { prefix, index }.candidate(),
        }),
        evidence: Some(PreservationEvidence {
            backup_ref: Some("refs/gwz/merge/merge_1/root/head".into()), backup_commit: Some(oid('e')),
            stash_id: Some("stash_merge_1".into()), stash_object_id: Some(oid('b')),
        }),
        publication_prefix: Some("boundary".into()),
    }
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
    assert!(rollback_exhausted_for_test(&next).is_err());

    let mut complete = next.record().clone();
    complete.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
    let complete = StoredV1Record::for_test(&root.path, complete).unwrap();
    assert!(rollback_exhausted_for_test(&complete).is_ok());
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
    assert!(rollback_exhausted_for_test(&stored).is_ok());

    fs::write(root.path.join(WORKSPACE_MANIFEST), "changed").unwrap();
    assert!(rollback_exhausted_for_test(&stored).is_err());
    fs::write(root.path.join(WORKSPACE_MANIFEST), &manifest).unwrap();
    fs::write(root.path.join(LOCK_PATH), "changed").unwrap();
    assert!(rollback_exhausted_for_test(&stored).is_err());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        fs::write(root.path.join(LOCK_PATH), &lock).unwrap();
        fs::remove_file(root.path.join(WORKSPACE_MANIFEST)).unwrap();
        let target = root.path.join("same-manifest-bytes");
        fs::write(&target, &manifest).unwrap();
        symlink(&target, root.path.join(WORKSPACE_MANIFEST)).unwrap();
        assert!(rollback_exhausted_for_test(&stored).is_err());
        assert_eq!(fs::read(target).unwrap(), manifest.as_bytes());
    }
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
        Origin::Executing,
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
        Origin::Executing,
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
