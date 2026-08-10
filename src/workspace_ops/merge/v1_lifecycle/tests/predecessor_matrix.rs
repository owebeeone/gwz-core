use std::cell::RefCell;
use std::collections::BTreeSet;

use super::super::authority::*;
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::*;
use super::fixtures::{accepted_workspace, align_baseline_lock, evidence_rollback_record};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, RecoveryContextV1, RecoveryOriginStateV1, test_record as record,
};
use crate::workspace_ops::merge::{
    MergeRecordError, OperationState, ParticipantState, PublicationProgress, PublicationStep,
};
use crate::workspace_ops::tests::TempDir;

const STATES: [OperationState; 9] = [
    OperationState::Executing,
    OperationState::AwaitingResolution,
    OperationState::Halted,
    OperationState::Finalizing,
    OperationState::Preserving,
    OperationState::RollingBack,
    OperationState::Completed,
    OperationState::Aborted,
    OperationState::RecoveryRequired,
];

thread_local! {
    static EFFECT_CAPTURE: RefCell<Option<Vec<StoreEffectCase>>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub(super) struct StoreEffectCase {
    pub(super) kind: EffectKind,
    pub(super) old: MergeOperationRecordV1,
    pub(super) next: MergeOperationRecordV1,
    pub(super) effect: TransitionEffect,
}

pub(crate) fn record_effect(
    kind: EffectKind,
    old: &MergeOperationRecordV1,
    next: &MergeOperationRecordV1,
    effect: &TransitionEffect,
) {
    EFFECT_CAPTURE.with(|capture| {
        if let Some(cases) = capture.borrow_mut().as_mut() {
            cases.push(StoreEffectCase {
                kind,
                old: old.clone(),
                next: next.clone(),
                effect: effect.clone(),
            });
        }
    });
}

#[test]
fn every_transition_variant_executes_its_declared_footprint() {
    let cases = capture_effect_cases();
    let observed = cases
        .into_iter()
        .map(|case| format!("{:?}", case.kind))
        .collect::<BTreeSet<_>>();
    let expected = super::effect::all_effect_kinds()
        .into_iter()
        .map(|kind| format!("{kind:?}"))
        .collect::<BTreeSet<_>>();
    assert_eq!(observed, expected);
    assert_eq!(observed.len(), EFFECT_VARIANT_COUNT);
}

pub(super) fn capture_effect_cases() -> Vec<StoreEffectCase> {
    EFFECT_CAPTURE.with(|capture| *capture.borrow_mut() = Some(Vec::new()));

    super::reducer::operation_reducers_cover_every_direct_state_edge();
    super::reducer::participant_prepare_and_outcome_are_checked_reducers();
    super::reducer::participant_compounds_preserve_write_ahead_ownership();
    super::vocabulary::preparation_failure_and_no_mutation_abort_reduce_exactly_once();
    super::dispatcher::failed_attempt_is_not_outcome_authority_and_halts_through_a_bound_batch();
    super::dispatcher::failed_resolution_retains_the_authoritative_conflict_and_owner();
    super::effect::publication_reducers_follow_every_exact_forward_phase();
    super::effect::migrated_publication_compatibility_has_only_the_two_named_successors();
    super::authority::recovery_and_drift_proofs_drive_only_their_exact_reducers();
    super::journal_vocabulary::preservation_reducers_enforce_the_exact_no_prefix_phase_graph();
    super::journal_vocabulary::rollback_reducers_follow_only_exact_cursor_successors();

    EFFECT_CAPTURE.with(|capture| capture.borrow_mut().take().unwrap())
}

#[derive(Clone, Copy, Debug)]
enum OperationCase {
    BeginExecution,
    AwaitResolution,
    Halt,
    EnterFinalizing,
    BeginPreservation,
    BeginRollback,
    Complete,
    Abort,
}

#[test]
fn every_operation_transition_accepts_only_its_listed_predecessor_states() {
    let root = TempDir::new("merge-v1-operation-predecessor-matrix");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    for case in [
        OperationCase::BeginExecution,
        OperationCase::AwaitResolution,
        OperationCase::Halt,
        OperationCase::EnterFinalizing,
        OperationCase::BeginPreservation,
        OperationCase::BeginRollback,
        OperationCase::Complete,
        OperationCase::Abort,
    ] {
        for state in STATES {
            let current = StoredV1Record::for_test(&root.path, model(case, state)).unwrap();
            let result = prepare(&lease, &current, transition(case, &current));
            assert_eq!(
                result.is_ok(),
                allowed(case, state),
                "{case:?} from {state:?}"
            );
        }
    }
}

#[test]
fn publication_physical_action_phase_matrix_is_closed() {
    let root = TempDir::new("merge-v1-publication-physical-phase-matrix");
    for (step, evidence_absent) in [
        (PublicationStep::CommittingEvidence, true),
        (PublicationStep::PublishingCandidate, false),
    ] {
        let mut model = evidence_rollback_record(&root);
        model.state = OperationState::Finalizing;
        let publication = model.publication.as_mut().unwrap();
        publication.step = step;
        if evidence_absent {
            publication.composition_commit = None;
            publication.composition_tree = None;
            publication.root_merge_commit = None;
            publication.candidate_hashes.clear();
        }
        let current = StoredV1Record::for_test(&root.path, model).unwrap();
        for (action, phase) in [
            (PublicationPhysicalAction::EvidenceCommit, "evidence_commit"),
            (PublicationPhysicalAction::WriteMarker, "write_marker"),
            (PublicationPhysicalAction::WriteLock, "write_lock"),
            (PublicationPhysicalAction::WriteBoundary, "write_boundary"),
            (PublicationPhysicalAction::StageIndex, "stage_index"),
        ] {
            let accepted = (step == PublicationStep::CommittingEvidence)
                == (action == PublicationPhysicalAction::EvidenceCommit);
            assert_eq!(
                authorize_publication(&current, action, phase).is_ok(),
                accepted
            );
            assert!(authorize_publication(&current, action, "wrong_phase").is_err());
        }
    }
}

fn authorize_publication(
    current: &StoredV1Record,
    action: PublicationPhysicalAction,
    phase: &str,
) -> crate::model::ModelResult<()> {
    let V1NextAction::Observe(request) = next_action(current, V1LifecycleRequest::Continue)? else {
        panic!("publication owner was not observed")
    };
    let proof = VerifiedPublicationAction::for_test(
        current,
        "@publication",
        "publication_action",
        phase,
        action,
    )?;
    let observation = BoundExactObservation::for_test(
        current,
        &request,
        ExactObservationFact::NotStarted(NotStartedObservation::Publication(proof)),
    )?;
    match resolve_observation(
        current,
        V1LifecycleRequest::Continue,
        request,
        observation,
        None,
    )? {
        ResolvedV1Action::Execute(_) => Ok(()),
        _ => Err(crate::model::ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "publication action was not executable",
        )),
    }
}

fn transition(case: OperationCase, current: &StoredV1Record) -> V1Transition {
    let value = match case {
        OperationCase::BeginExecution => OperationTransition::BeginExecution,
        OperationCase::AwaitResolution => OperationTransition::AwaitResolution,
        OperationCase::Halt => OperationTransition::Halt,
        OperationCase::EnterFinalizing => OperationTransition::EnterFinalizing(
            VerifiedParticipants::for_test(
                current,
                "@operation",
                "enter_finalizing",
                "executing",
                (),
            )
            .unwrap(),
        ),
        OperationCase::BeginPreservation => OperationTransition::BeginPreservation(Box::new(
            PreparedPreservationEntry::for_test(
                current,
                current.record(),
                handoff(current, ReverseEntryKind::Preservation, current.record()),
            )
            .unwrap(),
        )),
        OperationCase::BeginRollback if current.record().state == OperationState::Preserving => {
            let exhausted = VerifiedPreservationExhausted::for_test(
                current,
                "@operation",
                "preservation_exhausted",
                "verified",
                (),
            )
            .unwrap();
            OperationTransition::BeginRollback(Box::new(
                PreparedRollbackEntry::from_preserving_for_test(
                    current,
                    current.record(),
                    handoff(
                        current,
                        ReverseEntryKind::ExhaustedRollback,
                        current.record(),
                    ),
                    exhausted,
                )
                .unwrap(),
            ))
        }
        OperationCase::BeginRollback => OperationTransition::BeginRollback(Box::new(
            PreparedRollbackEntry::direct_for_test(
                current,
                current.record(),
                handoff(current, ReverseEntryKind::DirectRollback, current.record()),
            )
            .unwrap(),
        )),
        OperationCase::Complete => OperationTransition::CompleteOperation(
            VerifiedPublicationCompletion::for_test(
                current,
                "@operation",
                "publication_complete",
                "verified",
                (),
            )
            .unwrap(),
        ),
        OperationCase::Abort => OperationTransition::AbortOperation(
            VerifiedRollbackExhausted::for_test(
                current,
                "@operation",
                "rollback_exhausted",
                "cursor_verified",
                RollbackExhaustedPayload::empty_for_test(),
            )
            .unwrap(),
        ),
    };
    V1Transition::Operation(Box::new(value))
}

fn allowed(case: OperationCase, state: OperationState) -> bool {
    use OperationCase as C;
    use OperationState as S;
    match case {
        C::BeginExecution => matches!(state, S::AwaitingResolution | S::Halted),
        C::AwaitResolution | C::Halt | C::EnterFinalizing => state == S::Executing,
        C::BeginPreservation => matches!(
            state,
            S::Executing | S::AwaitingResolution | S::Halted | S::Finalizing
        ),
        C::BeginRollback => matches!(
            state,
            S::Executing | S::AwaitingResolution | S::Halted | S::Finalizing | S::Preserving
        ),
        C::Complete => state == S::Finalizing,
        C::Abort => state == S::RollingBack,
    }
}

fn model(case: OperationCase, state: OperationState) -> MergeOperationRecordV1 {
    let mut model = record_for_state(state);
    match (case, state) {
        (OperationCase::AwaitResolution, OperationState::Executing) => conflict(&mut model),
        (OperationCase::Halt, OperationState::Executing) => fail(&mut model),
        (OperationCase::EnterFinalizing, OperationState::Executing) => succeed(&mut model),
        (OperationCase::Complete, OperationState::Finalizing) => complete_publication(&mut model),
        (OperationCase::Abort, OperationState::RollingBack) => {
            model.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
        }
        _ => {}
    }
    model
}

pub(super) fn record_for_state(state: OperationState) -> MergeOperationRecordV1 {
    let mut model = record();
    model.state = state;
    match state {
        OperationState::AwaitingResolution => conflict(&mut model),
        OperationState::Halted => fail(&mut model),
        OperationState::Finalizing | OperationState::Completed => {
            succeed(&mut model);
            align_baseline_lock(&mut model);
            if state == OperationState::Completed {
                complete_publication(&mut model);
                model.state = OperationState::Completed;
            }
        }
        OperationState::Preserving => {
            model = super::fixtures::preserving_record();
        }
        OperationState::Aborted => {
            model.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
        }
        OperationState::RecoveryRequired => {
            model.recovery_context = Some(RecoveryContextV1 {
                origin_state: RecoveryOriginStateV1::Executing,
            });
        }
        _ => {}
    }
    model
}

fn complete_publication(model: &mut MergeOperationRecordV1) {
    model.state = OperationState::Finalizing;
    succeed(model);
    align_baseline_lock(model);
    let seed = StoredV1Record::for_test(std::path::Path::new("."), model.clone()).unwrap();
    model.accepted_workspace = Some(accepted_workspace(&seed));
    model.publication = Some(PublicationProgress {
        step: PublicationStep::Complete,
        candidate_lock_sha256: None,
        candidate_marker_path: None,
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: None,
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    });
}

fn succeed(model: &mut MergeOperationRecordV1) {
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::UpToDate;
    row.resulting_commit = Some(row.before_commit.clone());
}

fn conflict(model: &mut MergeOperationRecordV1) {
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Conflicted;
    row.expected_merge_head = Some(row.source_commit.clone());
}

fn fail(model: &mut MergeOperationRecordV1) {
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Failed;
    row.error = Some(MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: "halted".into(),
        detail: None,
    });
}

fn handoff(
    current: &StoredV1Record,
    kind: ReverseEntryKind,
    anticipated: &MergeOperationRecordV1,
) -> VerifiedPublicationHandoff {
    VerifiedPublicationHandoff::for_entry_test(current, kind, anticipated).unwrap()
}
