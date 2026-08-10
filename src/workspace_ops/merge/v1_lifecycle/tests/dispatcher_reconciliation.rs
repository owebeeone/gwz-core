use sha2::{Digest, Sha256};

use super::super::authority::{
    BoundAmbiguityEvidence, BoundExactObservation, BoundExecutionAttempt, CompletedObservation,
    EntryFact, ExactObservationFact, ExecutionDiagnostic, NotStartedObservation,
    ParticipantActionPayload, ParticipantFailurePayload, ParticipantObservation,
    PhysicalActionKind, PreparedFailureHaltBatch, PreparedRollbackEntry,
    PreservationCursorPosition, PreservationCursorPrefix, PreservationObservation,
    PreservationPayload, PublicationObservation, PublicationPhysicalAction, ResolvedV1Action,
    V1LifecycleRequest, V1NextAction, VerifiedEvidenceResult, VerifiedParticipantOutcome,
    VerifiedPreservationCursorPrefix, VerifiedPreservationExhausted, VerifiedPublicationHandoff,
    VerifiedRefResetPhase, VerifiedStashPhase, next_action, preservation_durability_fact,
    resolve_observation,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::{ReverseEntryKind, prepare};
use super::fixtures::{
    evidence_payload, evidence_rollback_record, preserving_record, up_to_date_action,
};
use crate::artifact::ManifestArtifact;
use crate::git::{GitCheckedPreservationMutation, GitRootPreservationStepObservation};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::{
    GitObjectAlgorithmV1, GitObjectIdV1, MergeOperationRecordV1, PendingPreservationActionV1,
    PreservationOwnerV1, PreservationPublicationHandoffV1, PreservationRefResetPhaseV1 as R,
    PreservationStashPhaseV1 as S, PublicationIndexFormV1, PublicationPrefixV1,
    RecoveryOriginStateV1, test_record as record,
};
use crate::workspace_ops::merge::{
    MergeRecordError, OperationState, ParticipantState, PreservationEvidence,
};
use crate::workspace_ops::tests::TempDir;

#[rustfmt::skip] #[test] fn halted_completion_resumes_only_after_the_final_halt_cause_is_removed() {
    let root = TempDir::new("merge-v1-halted-outcome-aggregate");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let mut model = record();
    add_second_participant(&mut model);
    model.state = OperationState::Halted;
    let first = model.participants.get_mut("mem_a").unwrap();
    first.state = ParticipantState::Failed;
    first.error = Some(git_error("first"));
    first.pending_action = Some(up_to_date_action());
    let second = model.participants.get_mut("mem_b").unwrap();
    second.state = ParticipantState::Failed;
    second.error = Some(git_error("second"));
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let V1NextAction::Observe(request) = next_action(&current, V1LifecycleRequest::Continue).unwrap()
        else { panic!("pending halted owner was not observed") };
    let mut row = current.record().participants["mem_a"].clone();
    row.state = ParticipantState::UpToDate;
    row.resulting_commit = Some(row.before_commit.clone());
    row.error = None;
    row.pending_action = None;
    let proof = VerifiedParticipantOutcome::for_test(&current, "mem_a", "participant_outcome", "completed",
        ParticipantActionPayload { member_id: "mem_a".into(), row }).unwrap();
    let observation = BoundExactObservation::for_test(&current, &request,
        ExactObservationFact::Completed(CompletedObservation::Participant(
            ParticipantObservation::Outcome(Box::new(proof), EntryFact::None)))).unwrap();
    let ResolvedV1Action::Apply(transition) = resolve_observation(
        &current, V1LifecycleRequest::Continue, request, observation, None).unwrap()
        else { panic!("halted aggregate outcome was not resolved") };
    let rewrite = prepare(&lease, &current, transition).unwrap();
    assert_eq!(rewrite.next().state, OperationState::Halted);
    assert_eq!(rewrite.next().participants["mem_a"].state, ParticipantState::UpToDate);
    assert_eq!(rewrite.next().participants["mem_b"].state, ParticipantState::Failed);
}

#[rustfmt::skip] #[test] fn preparation_failure_must_match_the_requested_participant() {
    let root = TempDir::new("merge-v1-preparation-failure-owner");
    let mut model = record();
    add_second_participant(&mut model);
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let V1NextAction::Observe(request) = next_action(&current, V1LifecycleRequest::Continue).unwrap()
        else { panic!("first participant preparation was not requested") };
    let mut row = current.record().participants["mem_b"].clone();
    row.state = ParticipantState::Failed;
    row.error = Some(git_error("wrong owner"));
    let batch = PreparedFailureHaltBatch::for_test(&current, "mem_b", "preparation_failure", "verified",
        ParticipantFailurePayload { member_id: "mem_b".into(), row, later_unattempted: Vec::new() }).unwrap();
    let observation = BoundExactObservation::for_test(&current, &request,
        ExactObservationFact::Completed(CompletedObservation::Participant(
            ParticipantObservation::PreparationFailed(Box::new(batch))))).unwrap();
    assert!(matches!(resolve_observation(&current, V1LifecycleRequest::Continue,
        request, observation, None).unwrap(), ResolvedV1Action::Reject(_)));
}

#[test]
fn abort_and_preserve_record_live_evidence_before_their_entry() {
    let root = TempDir::new("merge-v1-finalizing-handoff-evidence");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    for lifecycle in [V1LifecycleRequest::Abort, V1LifecycleRequest::Preserve] {
        let current = StoredV1Record::for_test(&root.path, publication_record(&root)).unwrap();
        let V1NextAction::Observe(request) = next_action(&current, lifecycle).unwrap() else {
            panic!("finalizing entry did not request a handoff observation")
        };
        let proof = VerifiedEvidenceResult::for_test(
            &current,
            "@publication",
            "record_evidence",
            "completed",
            evidence_payload(&current),
        )
        .unwrap();
        let observation = BoundExactObservation::for_test(
            &current,
            &request,
            ExactObservationFact::Completed(CompletedObservation::Publication(
                PublicationObservation::EvidenceResult(Box::new(proof)),
            )),
        )
        .unwrap();
        let ResolvedV1Action::Apply(transition) =
            resolve_observation(&current, lifecycle, request, observation, None).unwrap()
        else {
            panic!("live evidence was not ordered before the requested entry")
        };
        let rewrite = prepare(&lease, &current, transition).unwrap();
        assert_eq!(rewrite.next().state, OperationState::Finalizing);
        assert!(
            rewrite
                .next()
                .publication
                .as_ref()
                .unwrap()
                .composition_commit
                .is_some()
        );
    }
}

#[test]
fn completed_preservation_enters_rollback_for_every_mutating_resume_request() {
    let root = TempDir::new("merge-v1-preservation-exhausted-request-matrix");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    for lifecycle in [
        V1LifecycleRequest::ResumeStart,
        V1LifecycleRequest::Continue,
        V1LifecycleRequest::Abort,
        V1LifecycleRequest::Preserve,
        V1LifecycleRequest::Archive,
    ] {
        let current = StoredV1Record::for_test(&root.path, preserving_record()).unwrap();
        let V1NextAction::Observe(request) = next_action(&current, lifecycle).unwrap() else {
            panic!("preserving cursor was not resumed")
        };
        let exhausted = VerifiedPreservationExhausted::for_test(
            &current,
            "@operation",
            "preservation_exhausted",
            "verified",
            (),
        )
        .unwrap();
        let handoff = VerifiedPublicationHandoff::for_entry_test(
            &current,
            ReverseEntryKind::ExhaustedRollback,
            current.record(),
        )
        .unwrap();
        let entry = PreparedRollbackEntry::from_preserving_for_test(
            &current,
            current.record(),
            handoff,
            exhausted,
        )
        .unwrap();
        let observation = BoundExactObservation::for_test(
            &current,
            &request,
            ExactObservationFact::Completed(CompletedObservation::Preservation(
                PreservationObservation::Exhausted(Box::new(entry)),
            )),
        )
        .unwrap();
        let ResolvedV1Action::Apply(transition) =
            resolve_observation(&current, lifecycle, request, observation, None).unwrap()
        else {
            panic!("completed preservation did not enter rollback")
        };
        assert_eq!(
            prepare(&lease, &current, transition).unwrap().next().state,
            OperationState::RollingBack
        );
    }
}

#[rustfmt::skip] #[test] fn preservation_durability_pending_is_causal_and_fail_closed() {
    assert_eq!(super::super::reverse::preservation_durability_diagnostic(
        Ok(GitCheckedPreservationMutation::AlreadyComplete)), ExecutionDiagnostic::Success);
    let root = TempDir::new("merge-v1-preservation-durability-pending");
    for action in [root_stash(S::NormalizeParent), root_reset(R::PrepareParent), root_stash(S::RestoreParent)] {
        assert_causal_parent_case(&root, action);
    }
}
#[rustfmt::skip] fn assert_causal_parent_case(root: &TempDir, action: PendingPreservationActionV1) {
    use super::dispatcher_attempt_matrix::failed_diagnostic;
    let current = durability_current(root, action);
    let position = action_position(current.record().pending_preservation.as_ref().unwrap());
    let wrong = wrong_position(current.record().pending_preservation.as_ref().unwrap());
    for fact in [durability_fact(&current, position), before_fact(&current, position)] {
        assert!(matches!(resolve_fact(&current, fact, None).unwrap(), ResolvedV1Action::Execute(_)));
    }
    let success = barrier_attempt(&current, ExecutionDiagnostic::Success);
    let second = barrier_attempt(&current, ExecutionDiagnostic::Success);
    let ResolvedV1Action::Apply(transition) = resolve_fact(&current, durability_fact(&current, position), Some(success)).unwrap()
        else { panic!("matching success did not advance {position:?}") };
    assert!(matches!(resolve_fact(&current, durability_fact(&current, position), None).unwrap(), ResolvedV1Action::Execute(_)),
        "crash-before-rewrite did not retry {position:?}");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let next = StoredV1Record::for_test(&root.path, prepare(&lease, &current, transition).unwrap().next().clone()).unwrap();
    let successor = successor_action(current.record().pending_preservation.as_ref().unwrap());
    assert_eq!(next.record().pending_preservation.as_ref(), Some(&successor));
    assert!(resolve_fact(&next, before_fact(&next, action_position(&successor)), Some(second)).is_err());
    for (name, fact) in [("pending", durability_fact(&current, position)),
        ("before", before_fact(&current, position)), ("ambiguous", ambiguous_fact(&current, position))] {
        let ResolvedV1Action::Reject(error) = resolve_fact(&current, fact, Some(barrier_attempt(&current, failed_diagnostic()))).unwrap()
            else { panic!("{name} failure caused a transition or retry for {position:?}") };
        assert_eq!((error.code, error.message.as_str()), (ErrorCode::GitCommandFailed, "late executor diagnostic"), "{name} {position:?}");
    }
    for (name, fact) in [("pending-prefix", durability_fact(&current, wrong)),
        ("before-prefix", before_fact(&current, wrong)), ("ambiguous-prefix", ambiguous_fact(&current, wrong)),
        ("generic-ambiguous", generic_ambiguous_fact(&current))] {
        assert_rejected_without_diagnostic(&current, name, fact);
    }
    assert!(resolve_fact(&current, before_action_fact(&current, successor),
        Some(barrier_attempt(&current, failed_diagnostic()))).is_err());
    let mut stale = current.record().clone();
    stale.extensions.insert("stale_probe".into(), serde_yaml::Value::Bool(true));
    let stale = StoredV1Record::for_test(&root.path, stale).unwrap();
    assert!(resolve_fact(&stale, durability_fact(&stale, position), Some(barrier_attempt(&current, failed_diagnostic()))).is_err());
    for fact in [before_fact(&current, position), ambiguous_fact(&current, position), generic_ambiguous_fact(&current)] {
        assert_rejected(&current, fact, barrier_attempt(&current, ExecutionDiagnostic::Success));
    }
}
#[rustfmt::skip] fn assert_rejected_without_diagnostic(current: &StoredV1Record, name: &str, fact: ExactObservationFact) {
    match resolve_fact(current, fact, Some(barrier_attempt(current, super::dispatcher_attempt_matrix::failed_diagnostic()))) {
        Err(error) | Ok(ResolvedV1Action::Reject(error)) => assert_ne!((error.code, error.message.as_str()),
            (ErrorCode::GitCommandFailed, "late executor diagnostic"), "{name}"),
        Ok(_) => panic!("{name} accepted unauthorized failure authority"),
    }
}
#[rustfmt::skip] fn assert_rejected(current: &StoredV1Record, fact: ExactObservationFact, attempt: BoundExecutionAttempt) {
    assert!(matches!(resolve_fact(current, fact, Some(attempt)), Err(_) | Ok(ResolvedV1Action::Reject(_))));
}
#[rustfmt::skip] fn before_fact(current: &StoredV1Record, position: PreservationCursorPosition) -> ExactObservationFact {
    ExactObservationFact::NotStarted(NotStartedObservation::Preservation {
        action: current.record().pending_preservation.as_ref().unwrap().clone(), prefix: root_prefix(current, position) })
}
#[rustfmt::skip] fn before_action_fact(current: &StoredV1Record, action: PendingPreservationActionV1) -> ExactObservationFact {
    let position = action_position(&action);
    ExactObservationFact::NotStarted(NotStartedObservation::Preservation {
        action, prefix: root_prefix(current, position) })
}
#[rustfmt::skip] fn ambiguous_fact(current: &StoredV1Record, position: PreservationCursorPosition) -> ExactObservationFact {
    ExactObservationFact::PreservationAmbiguous(ambiguity_proof(current), root_prefix(current, position))
}
#[rustfmt::skip] fn generic_ambiguous_fact(current: &StoredV1Record) -> ExactObservationFact { ExactObservationFact::Ambiguous(ambiguity_proof(current)) }
#[rustfmt::skip] fn ambiguity_proof(current: &StoredV1Record) -> BoundAmbiguityEvidence {
    BoundAmbiguityEvidence::for_test(current, "@operation", "enter_recovery", "ambiguous",
        RecoveryOriginStateV1::Preserving).unwrap()
}
#[rustfmt::skip] fn barrier_attempt(current: &StoredV1Record, diagnostic: ExecutionDiagnostic) -> BoundExecutionAttempt {
    let position = action_position(current.record().pending_preservation.as_ref().unwrap());
    let ResolvedV1Action::Execute(action) = resolve_fact(current, durability_fact(current, position), None).unwrap()
        else { panic!("durability barrier was not authorized") };
    action.record_attempt(current, diagnostic).unwrap()
}
#[rustfmt::skip] fn durability_current(root: &TempDir, action: PendingPreservationActionV1) -> StoredV1Record {
    let mut model = evidence_rollback_record(root);
    model.state = OperationState::Preserving;
    model.preservation_publication_handoff = Some(root_handoff());
    if matches!(action, PendingPreservationActionV1::ResetAttachedRef { .. }) {
        model.publication.as_mut().unwrap().root_preservation.push(PreservationEvidence {
            backup_ref: Some("refs/gwz/merge/merge_1/root/head".into()), backup_commit: Some("d".repeat(40)),
            stash_id: None, stash_object_id: None });
    } else if matches!(action, PendingPreservationActionV1::Stash { phase: S::RestoreParent, .. }) {
        model.publication.as_mut().unwrap().root_preservation.push(PreservationEvidence {
            backup_ref: None, backup_commit: None, stash_id: Some("stash_merge_1".into()), stash_object_id: Some("b".repeat(40)) });
    }
    model.pending_preservation = Some(action);
    StoredV1Record::for_test(&root.path, model).unwrap()
}
#[rustfmt::skip] fn durability_fact(current: &StoredV1Record, outer_position: PreservationCursorPosition) -> ExactObservationFact {
    let action = current.record().pending_preservation.as_ref().unwrap();
    let position = action_position(action);
    let payload = PreservationPayload { owner: PreservationOwnerV1::PublicationRoot, observed_position: position,
        pending: Some(successor_action(action)), evidence: None, publication_prefix: Some("baseline".into()) };
    let completion = match action {
        PendingPreservationActionV1::Stash { .. } => PreservationObservation::StashPhase(Box::new(
            VerifiedStashPhase::for_test(current, "@publication-root", "advance_stash", "completed", payload,
                root_prefix(current, position)).unwrap())),
        PendingPreservationActionV1::ResetAttachedRef { .. } => PreservationObservation::ResetPhase(Box::new(
            VerifiedRefResetPhase::for_test(current, "@publication-root", "advance_reset_attached_ref", "completed", payload,
                root_prefix(current, position)).unwrap())),
        PendingPreservationActionV1::BackupRef { .. } => unreachable!(),
    };
    preservation_durability_fact(GitRootPreservationStepObservation::AfterNeedsDurability, completion,
        root_prefix(current, outer_position), action.clone()).unwrap()
}
#[rustfmt::skip] fn action_position(action: &PendingPreservationActionV1) -> PreservationCursorPosition { match action {
    PendingPreservationActionV1::Stash { phase, .. } => PreservationCursorPosition::Stash(*phase),
    PendingPreservationActionV1::ResetAttachedRef { phase, .. } => PreservationCursorPosition::ResetAttachedRef(*phase),
    PendingPreservationActionV1::BackupRef { .. } => unreachable!(), } }
#[rustfmt::skip] fn successor_action(action: &PendingPreservationActionV1) -> PendingPreservationActionV1 { match action {
    PendingPreservationActionV1::Stash { phase: S::NormalizeParent, .. } => root_stash(S::NormalizeMarker),
    PendingPreservationActionV1::Stash { phase: S::RestoreParent, .. } => root_stash(S::RestoreMarker),
    PendingPreservationActionV1::ResetAttachedRef { phase: R::PrepareParent, .. } => root_reset(R::PrepareMarker),
    _ => unreachable!(), } }
#[rustfmt::skip] fn wrong_position(action: &PendingPreservationActionV1) -> PreservationCursorPosition { match action {
    PendingPreservationActionV1::Stash { .. } => PreservationCursorPosition::Stash(S::RestoreLock),
    PendingPreservationActionV1::ResetAttachedRef { .. } => PreservationCursorPosition::ResetAttachedRef(R::PrepareLock),
    PendingPreservationActionV1::BackupRef { .. } => unreachable!(), } }
#[rustfmt::skip] fn root_prefix(current: &StoredV1Record, position: PreservationCursorPosition) -> VerifiedPreservationCursorPrefix {
    VerifiedPreservationCursorPrefix::for_test(current, "@publication-root", "preservation_cursor", "prefix_verified",
        PreservationCursorPrefix { owner: PreservationOwnerV1::PublicationRoot, position }).unwrap()
}
#[rustfmt::skip] fn root_handoff() -> PreservationPublicationHandoffV1 {
    PreservationPublicationHandoffV1::Candidate { prefix: PublicationPrefixV1::Baseline, index: PublicationIndexFormV1::Pre }
}
#[rustfmt::skip] fn root_stash(phase: S) -> PendingPreservationActionV1 {
    let ids = !matches!(phase, S::NormalizeParent | S::NormalizeMarker | S::NormalizeLock | S::NormalizeIndex | S::CreateStash);
    PendingPreservationActionV1::Stash {
        owner: PreservationOwnerV1::PublicationRoot, phase, stash_id: ids.then(|| "stash_merge_1".into()),
        stash_object_id: ids.then(|| GitObjectIdV1 { algorithm: GitObjectAlgorithmV1::Sha1, digest_hex: "b".repeat(40) }),
        message: "gwz:stash_merge_1: merge preservation".into(), head_commit: "e".repeat(40),
        preimage_sha256: "1".repeat(64), root_publication_handoff: root_handoff().candidate(),
    }
}
#[rustfmt::skip] fn root_reset(phase: R) -> PendingPreservationActionV1 {
    PendingPreservationActionV1::ResetAttachedRef { owner: PreservationOwnerV1::PublicationRoot, branch: "main".into(),
        expected_commit: "d".repeat(40), restore_commit: "e".repeat(40), phase,
        root_publication_handoff: root_handoff().candidate() }
}
#[rustfmt::skip] fn resolve_fact(current: &StoredV1Record, fact: ExactObservationFact,
    attempt: Option<BoundExecutionAttempt>) -> crate::model::ModelResult<ResolvedV1Action> {
    let V1NextAction::Observe(request) = next_action(current, V1LifecycleRequest::Continue).unwrap()
        else { panic!("preservation cursor was not observed") };
    let observation = BoundExactObservation::for_test(current, &request, fact).unwrap();
    resolve_observation(current, V1LifecycleRequest::Continue, request, observation, attempt)
}
#[test]
fn ambiguous_observations_override_both_executor_diagnostics_for_every_action_owner() {
    use super::dispatcher_attempt_matrix as attempts;
    let root = TempDir::new("merge-v1-ambiguous-after-diagnostic-matrix");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let mut resolution = attempts::participant_record();
    let row = resolution.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Conflicted;
    row.expected_merge_head = Some(row.source_commit.clone());
    row.pending_action = Some(super::dispatcher::resolve_action());
    let cases = [
        (
            "participant",
            attempts::participant_record(),
            PhysicalActionKind::Participant {
                member_id: "mem_a".into(),
                action: Box::new(up_to_date_action()),
            },
            RecoveryOriginStateV1::Executing,
        ),
        (
            "resolution",
            resolution,
            PhysicalActionKind::Participant {
                member_id: "mem_a".into(),
                action: Box::new(super::dispatcher::resolve_action()),
            },
            RecoveryOriginStateV1::Executing,
        ),
        (
            "publication",
            attempts::publication_record(&root),
            PhysicalActionKind::Publication(PublicationPhysicalAction::EvidenceCommit),
            RecoveryOriginStateV1::Finalizing,
        ),
        (
            "preservation",
            attempts::preservation_record(),
            PhysicalActionKind::Preservation(super::fixtures::backup_action()),
            RecoveryOriginStateV1::Preserving,
        ),
        (
            "rollback",
            attempts::rollback_record(),
            PhysicalActionKind::Rollback(attempts::rollback_action()),
            RecoveryOriginStateV1::RollingBack,
        ),
    ];

    for (name, model, action, origin) in cases {
        for failed in [false, true] {
            let current = StoredV1Record::for_test(&root.path, model.clone()).unwrap();
            let diagnostic = if failed {
                attempts::failed_diagnostic()
            } else {
                ExecutionDiagnostic::Success
            };
            let attempt = attempts::execution_attempt(&current, action.clone(), diagnostic);
            let V1NextAction::Observe(request) =
                next_action(&current, V1LifecycleRequest::Continue).unwrap()
            else {
                panic!("{name} owner was not reobserved")
            };
            let ambiguity = BoundAmbiguityEvidence::for_test(
                &current,
                "@operation",
                "enter_recovery",
                "ambiguous",
                origin,
            )
            .unwrap();
            let fact = if origin == RecoveryOriginStateV1::Preserving {
                ExactObservationFact::PreservationAmbiguous(
                    ambiguity,
                    super::fixtures::preservation_prefix(
                        &current,
                        PreservationCursorPosition::BackupRef,
                    ),
                )
            } else {
                ExactObservationFact::Ambiguous(ambiguity)
            };
            let observation = BoundExactObservation::for_test(&current, &request, fact).unwrap();
            let resolved = resolve_observation(
                &current,
                V1LifecycleRequest::Continue,
                request,
                observation,
                Some(attempt),
            )
            .unwrap();
            if origin == RecoveryOriginStateV1::Preserving {
                assert!(matches!(resolved, ResolvedV1Action::Reject(_)));
                continue;
            }
            let ResolvedV1Action::Apply(transition) = resolved else {
                panic!("{name} ambiguity did not take authority")
            };
            assert_eq!(
                prepare(&lease, &current, transition).unwrap().next().state,
                OperationState::RecoveryRequired,
                "{name} failed={failed}"
            );
        }
    }
}

fn publication_record(root: &TempDir) -> MergeOperationRecordV1 {
    let mut model = evidence_rollback_record(root);
    model.state = OperationState::Finalizing;
    let progress = model.publication.as_mut().unwrap();
    progress.composition_commit = None;
    progress.composition_tree = None;
    progress.root_merge_commit = None;
    progress.candidate_hashes.clear();
    model
}

fn add_second_participant(model: &mut MergeOperationRecordV1) {
    let mut manifest =
        ManifestArtifact::from_yaml(model.baseline.manifest_yaml.as_deref().unwrap()).unwrap();
    let mut member = manifest.members[0].clone();
    member.id = "mem_b".into();
    member.path = "members/b".into();
    member.source_id = "src_b".into();
    manifest.members.push(member);
    let manifest = manifest.to_yaml().unwrap();
    model.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest.as_bytes()));
    model.baseline.manifest_yaml = Some(manifest);
    let mut second = model.participants["mem_a"].clone();
    second.path = "members/b".into();
    model.participants.insert("mem_b".into(), second);
    model.selected_targets.push("mem_b".into());
}

fn git_error(message: &str) -> MergeRecordError {
    MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: message.into(),
        detail: None,
    }
}
