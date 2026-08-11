use super::super::authority::{
    ParticipantActionPayload, PublicationHandoffFact, RecordEvidenceOr, V1LifecycleRequest,
    VerifiedParticipantNotStarted, VerifiedParticipantOutcome, VerifiedPreservationEntryPreflight,
    VerifiedPreservationExhausted, VerifiedPublicationHandoff, VerifiedRollbackEntryPreflight,
    observe_reverse_publication_handoff, prepare_direct_rollback_entry,
    prepare_exhausted_rollback_entry, prepare_preservation_entry,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::{
    OperationTransition, ReverseEntryKind, ReverseEntryPredecessor, V1Transition, prepare,
    preview_reverse_entry, reverse_entry_kind,
};
use super::fixtures::up_to_date_action;
use super::predecessor_matrix::record_for_state;
use crate::git::Git2Backend;
use crate::operation::{ActionKind, OperationContext};
use crate::workspace_ops::merge::model::v1::test_record;
use crate::workspace_ops::merge::{OperationState, ParticipantState};
use crate::workspace_ops::tests::TempDir;

fn context() -> OperationContext {
    OperationContext {
        operation_id: "op_1".into(),
        request_id: "req_1".into(),
        schema_version: "gwz.protocol/v0".into(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}

#[test]
fn preview_uses_the_reducer_outcome_shape_and_binds_the_f_handoff() {
    let root = TempDir::new("merge-v1-reverse-entry-outcome-preview");
    let mut model = test_record();
    model.state = OperationState::Halted;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Failed;
    row.error = Some(crate::workspace_ops::merge::MergeRecordError {
        code: crate::model::ErrorCode::GitCommandFailed,
        message: "halted".into(),
        detail: None,
    });
    row.pending_action = Some(up_to_date_action());
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let mut completed = current.record().participants["mem_a"].clone();
    completed.state = ParticipantState::UpToDate;
    completed.resulting_commit = Some(completed.before_commit.clone());
    completed.error = None;
    completed.pending_action = None;
    let proof = VerifiedParticipantOutcome::for_test(
        &current,
        "mem_a",
        "participant_outcome",
        "completed",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row: completed,
        },
    )
    .unwrap();
    let preview = preview_reverse_entry(
        &current,
        V1LifecycleRequest::Preserve,
        ReverseEntryPredecessor::ParticipantOutcome(&proof),
    )
    .unwrap();
    assert_eq!(preview.kind(), ReverseEntryKind::Preservation);

    let handoff =
        observe_reverse_publication_handoff(&Git2Backend::new(), &context(), &current, &preview)
            .unwrap();
    let RecordEvidenceOr::Ready(handoff) = handoff else {
        panic!("pre-finalizing outcome unexpectedly requested evidence recording")
    };
    assert_eq!(handoff.value().kind, ReverseEntryKind::Preservation);
    assert_eq!(handoff.value().request, V1LifecycleRequest::Preserve);
    assert_eq!(
        handoff.value().anticipated_model_sha256,
        preview.anticipated_model_sha256()
    );
}

#[test]
fn preview_rejects_stale_lineage_and_cross_kind_reuse() {
    let root = TempDir::new("merge-v1-reverse-entry-stale-preview");
    let current = StoredV1Record::for_test(&root.path, test_record()).unwrap();
    let preview = preview_reverse_entry(
        &current,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    assert_eq!(preview.kind(), ReverseEntryKind::DirectRollback);

    let mut changed = current.record().clone();
    changed.writer_version.push_str("-changed");
    let changed = StoredV1Record::for_test(&root.path, changed).unwrap();
    let error = match observe_reverse_publication_handoff(
        &Git2Backend::new(),
        &context(),
        &changed,
        &preview,
    ) {
        Ok(_) => panic!("stale preview unexpectedly produced a handoff"),
        Err(error) => error,
    };
    assert_eq!(error.code, crate::model::ErrorCode::MergeRecoveryRequired);

    let preserving =
        StoredV1Record::for_test(&root.path, super::fixtures::preserving_record()).unwrap();
    let exhausted_preview = preview_reverse_entry(
        &preserving,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    let exhausted_handoff = ready_handoff(
        &Git2Backend::new(),
        &context(),
        &preserving,
        &exhausted_preview,
    );
    let exhausted_preflight =
        VerifiedRollbackEntryPreflight::for_entry_test(&preserving, &exhausted_handoff).unwrap();
    assert!(
        prepare_direct_rollback_entry(
            &preserving,
            &exhausted_preview,
            exhausted_handoff,
            exhausted_preflight,
        )
        .is_err()
    );
}

#[test]
fn production_entry_constructors_require_matching_preview_handoff_and_preflight() {
    let root = TempDir::new("merge-v1-reverse-entry-production-constructors");
    let backend = Git2Backend::new();
    let operation_context = context();

    let current = StoredV1Record::for_test(&root.path, test_record()).unwrap();
    let preservation = preview_reverse_entry(
        &current,
        V1LifecycleRequest::Preserve,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    let RecordEvidenceOr::Ready(handoff) =
        observe_reverse_publication_handoff(&backend, &operation_context, &current, &preservation)
            .unwrap()
    else {
        panic!("pre-finalizing preservation unexpectedly requested evidence recording")
    };
    let preflight = VerifiedPreservationEntryPreflight::for_entry_test(&current, &handoff).unwrap();
    let prepared = prepare_preservation_entry(&current, &preservation, handoff, preflight).unwrap();
    assert!(prepared.anticipated_model_matches(current.record()));

    let direct = preview_reverse_entry(
        &current,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    let RecordEvidenceOr::Ready(handoff) =
        observe_reverse_publication_handoff(&backend, &operation_context, &current, &direct)
            .unwrap()
    else {
        panic!("pre-finalizing rollback unexpectedly requested evidence recording")
    };
    let preflight = VerifiedRollbackEntryPreflight::for_entry_test(&current, &handoff).unwrap();
    let prepared = prepare_direct_rollback_entry(&current, &direct, handoff, preflight).unwrap();
    assert!(prepared.anticipated_model_matches(current.record()));

    let preserving =
        StoredV1Record::for_test(&root.path, super::fixtures::preserving_record()).unwrap();
    let exhausted = preview_reverse_entry(
        &preserving,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    let RecordEvidenceOr::Ready(handoff) =
        observe_reverse_publication_handoff(&backend, &operation_context, &preserving, &exhausted)
            .unwrap()
    else {
        panic!("post-preservation rollback unexpectedly requested evidence recording")
    };
    let preflight = VerifiedRollbackEntryPreflight::for_entry_test(&preserving, &handoff).unwrap();
    let exhaustion = VerifiedPreservationExhausted::for_test(
        &preserving,
        "@operation",
        "preservation_exhausted",
        "verified",
        (),
    )
    .unwrap();
    let prepared =
        prepare_exhausted_rollback_entry(&preserving, &exhausted, handoff, preflight, exhaustion)
            .unwrap();
    assert!(prepared.anticipated_model_matches(preserving.record()));
}

#[test]
fn action_free_preview_rejects_a_retained_forward_owner() {
    let root = TempDir::new("merge-v1-reverse-entry-action-free");
    let mut model = test_record();
    model.participants.get_mut("mem_a").unwrap().pending_action = Some(up_to_date_action());
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    assert!(
        preview_reverse_entry(
            &current,
            V1LifecycleRequest::Abort,
            ReverseEntryPredecessor::ActionFree,
        )
        .is_err()
    );
}

#[test]
fn reverse_entry_state_and_request_matrix_is_closed() {
    let requests = [
        V1LifecycleRequest::ResumeStart,
        V1LifecycleRequest::Continue,
        V1LifecycleRequest::Abort,
        V1LifecycleRequest::Preserve,
        V1LifecycleRequest::Status,
        V1LifecycleRequest::Archive,
    ];
    for state in [
        OperationState::Executing,
        OperationState::AwaitingResolution,
        OperationState::Halted,
        OperationState::Finalizing,
        OperationState::Preserving,
        OperationState::RollingBack,
        OperationState::Completed,
        OperationState::Aborted,
        OperationState::RecoveryRequired,
    ] {
        for request in requests {
            let expected =
                if state == OperationState::Preserving && request != V1LifecycleRequest::Status {
                    Some(ReverseEntryKind::ExhaustedRollback)
                } else if matches!(
                    state,
                    OperationState::Executing
                        | OperationState::AwaitingResolution
                        | OperationState::Halted
                        | OperationState::Finalizing
                ) {
                    match request {
                        V1LifecycleRequest::Preserve => Some(ReverseEntryKind::Preservation),
                        V1LifecycleRequest::Abort => Some(ReverseEntryKind::DirectRollback),
                        _ => None,
                    }
                } else {
                    None
                };
            assert_eq!(
                reverse_entry_kind(state, request).ok(),
                expected,
                "{state:?}/{request:?}"
            );
            for predecessor in [
                PredecessorCase::ActionFree,
                PredecessorCase::ParticipantOutcome,
                PredecessorCase::ParticipantNotStarted,
            ] {
                let expected = match predecessor {
                    PredecessorCase::ActionFree => expected.is_some(),
                    PredecessorCase::ParticipantOutcome => {
                        state == OperationState::Halted
                            && matches!(
                                request,
                                V1LifecycleRequest::Abort | V1LifecycleRequest::Preserve
                            )
                    }
                    PredecessorCase::ParticipantNotStarted => {
                        matches!(state, OperationState::Executing | OperationState::Halted)
                            && matches!(
                                request,
                                V1LifecycleRequest::Abort | V1LifecycleRequest::Preserve
                            )
                    }
                };
                assert_eq!(
                    preview_succeeds(state, request, predecessor),
                    expected,
                    "{state:?}/{request:?}/{predecessor:?}"
                );
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum PredecessorCase {
    ActionFree,
    ParticipantOutcome,
    ParticipantNotStarted,
}

fn preview_succeeds(
    state: OperationState,
    request: V1LifecycleRequest,
    predecessor: PredecessorCase,
) -> bool {
    let root = TempDir::new("merge-v1-reverse-entry-predecessor-matrix");
    let mut model = record_for_state(state);
    if !matches!(predecessor, PredecessorCase::ActionFree)
        && matches!(state, OperationState::Executing | OperationState::Halted)
    {
        model.participants.get_mut("mem_a").unwrap().pending_action = Some(up_to_date_action());
    }
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    match predecessor {
        PredecessorCase::ActionFree => {
            preview_reverse_entry(&current, request, ReverseEntryPredecessor::ActionFree).is_ok()
        }
        PredecessorCase::ParticipantOutcome => {
            let mut row = current.record().participants["mem_a"].clone();
            row.state = ParticipantState::UpToDate;
            row.resulting_commit = Some(row.before_commit.clone());
            row.error = None;
            row.pending_action = None;
            let proof = VerifiedParticipantOutcome::for_test(
                &current,
                "mem_a",
                "participant_outcome",
                "completed",
                ParticipantActionPayload {
                    member_id: "mem_a".into(),
                    row,
                },
            )
            .unwrap();
            preview_reverse_entry(
                &current,
                request,
                ReverseEntryPredecessor::ParticipantOutcome(&proof),
            )
            .is_ok()
        }
        PredecessorCase::ParticipantNotStarted => {
            let proof = VerifiedParticipantNotStarted::for_test(
                &current,
                "mem_a",
                "participant_action",
                "not_started",
                "mem_a".into(),
            )
            .unwrap();
            preview_reverse_entry(
                &current,
                request,
                ReverseEntryPredecessor::ParticipantNotStarted(&proof),
            )
            .is_ok()
        }
    }
}

#[test]
fn exhausted_rollback_rejects_cross_request_replay_at_the_production_constructor() {
    let root = TempDir::new("merge-v1-reverse-entry-cross-request");
    let backend = Git2Backend::new();
    let operation_context = context();
    let current =
        StoredV1Record::for_test(&root.path, super::fixtures::preserving_record()).unwrap();
    let continued = preview_reverse_entry(
        &current,
        V1LifecycleRequest::Continue,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    let aborted = preview_reverse_entry(
        &current,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    let RecordEvidenceOr::Ready(abort_handoff) =
        observe_reverse_publication_handoff(&backend, &operation_context, &current, &aborted)
            .unwrap()
    else {
        panic!("pre-finalizing rollback unexpectedly requested evidence recording")
    };
    let abort_preflight =
        VerifiedRollbackEntryPreflight::for_entry_test(&current, &abort_handoff).unwrap();
    let exhausted = preservation_exhausted(&current);
    assert!(
        prepare_exhausted_rollback_entry(
            &current,
            &continued,
            abort_handoff,
            abort_preflight,
            exhausted,
        )
        .is_err()
    );

    let RecordEvidenceOr::Ready(continue_handoff) =
        observe_reverse_publication_handoff(&backend, &operation_context, &current, &continued)
            .unwrap()
    else {
        panic!("pre-finalizing rollback unexpectedly requested evidence recording")
    };
    let continue_preflight =
        VerifiedRollbackEntryPreflight::for_entry_test(&current, &continue_handoff).unwrap();
    assert!(
        prepare_exhausted_rollback_entry(
            &current,
            &continued,
            continue_handoff,
            continue_preflight,
            preservation_exhausted(&current),
        )
        .is_ok()
    );
}

#[test]
fn production_constructor_rejects_wrong_digest_and_reducer_rejects_stale_prepared_entry() {
    let root = TempDir::new("merge-v1-reverse-entry-digest-and-stale");
    let backend = Git2Backend::new();
    let operation_context = context();
    let current = StoredV1Record::for_test(&root.path, test_record()).unwrap();
    let preview = preview_reverse_entry(
        &current,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    let mut wrong = current.record().clone();
    wrong.writer_version.push_str("-wrong");
    let wrong_handoff = VerifiedPublicationHandoff::for_entry_request_test(
        &current,
        V1LifecycleRequest::Abort,
        ReverseEntryKind::DirectRollback,
        &wrong,
        PublicationHandoffFact::NoCandidate,
    )
    .unwrap();
    let wrong_preflight =
        VerifiedRollbackEntryPreflight::for_entry_test(&current, &wrong_handoff).unwrap();
    assert!(
        prepare_direct_rollback_entry(&current, &preview, wrong_handoff, wrong_preflight).is_err()
    );

    let RecordEvidenceOr::Ready(handoff) =
        observe_reverse_publication_handoff(&backend, &operation_context, &current, &preview)
            .unwrap()
    else {
        panic!("pre-finalizing rollback unexpectedly requested evidence recording")
    };
    let preflight = VerifiedRollbackEntryPreflight::for_entry_test(&current, &handoff).unwrap();
    let prepared = prepare_direct_rollback_entry(&current, &preview, handoff, preflight).unwrap();
    let changed = StoredV1Record::for_test(&root.path, wrong).unwrap();
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    assert!(
        prepare(
            &lease,
            &changed,
            V1Transition::Operation(Box::new(OperationTransition::BeginRollback(Box::new(
                prepared,
            )))),
        )
        .is_err()
    );
}

#[test]
fn production_constructor_rejects_stale_handoff_and_preflight_independently() {
    let root = TempDir::new("merge-v1-reverse-entry-stale-authorities");
    let backend = Git2Backend::new();
    let operation_context = context();
    let current = StoredV1Record::for_test(&root.path, test_record()).unwrap();
    let old_preview = preview_reverse_entry(
        &current,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    let old_handoff = ready_handoff(&backend, &operation_context, &current, &old_preview);
    let old_preflight =
        VerifiedRollbackEntryPreflight::for_entry_test(&current, &old_handoff).unwrap();

    let mut changed_model = current.record().clone();
    changed_model.writer_version.push_str("-changed");
    let changed = StoredV1Record::for_test(&root.path, changed_model).unwrap();
    let changed_preview = preview_reverse_entry(
        &changed,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )
    .unwrap();
    let fresh_handoff = ready_handoff(&backend, &operation_context, &changed, &changed_preview);
    let fresh_preflight =
        VerifiedRollbackEntryPreflight::for_entry_test(&changed, &fresh_handoff).unwrap();
    assert!(
        prepare_direct_rollback_entry(&changed, &changed_preview, old_handoff, fresh_preflight,)
            .is_err()
    );

    let fresh_handoff = ready_handoff(&backend, &operation_context, &changed, &changed_preview);
    assert!(
        prepare_direct_rollback_entry(&changed, &changed_preview, fresh_handoff, old_preflight,)
            .is_err()
    );
}

#[test]
fn reverse_entry_visitor_source_gate_allows_only_the_declared_observer_modules() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/workspace_ops/merge/v1_lifecycle");
    let approved = [
        root.join("authority/observe/finalization/handoff.rs"),
        root.join("authority/observe/reverse/preservation/entry.rs"),
        root.join("authority/observe/reverse/rollback.rs"),
    ];
    let test_root = root.join("tests");
    let mut stack = vec![root];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }
            if path.starts_with(&test_root) {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            if source.contains("SealedReverseEntryVisitor for")
                || source.contains("reverse_entry_visitor_seal::Visitor for")
            {
                assert!(approved.contains(&path), "unapproved visitor: {path:?}");
            }
        }
    }
}

fn ready_handoff(
    backend: &Git2Backend,
    operation_context: &OperationContext,
    current: &StoredV1Record,
    preview: &super::super::transition::PreparedReverseEntryView,
) -> VerifiedPublicationHandoff {
    let RecordEvidenceOr::Ready(handoff) =
        observe_reverse_publication_handoff(backend, operation_context, current, preview).unwrap()
    else {
        panic!("pre-finalizing reverse entry unexpectedly requested evidence recording")
    };
    handoff
}

fn preservation_exhausted(current: &StoredV1Record) -> VerifiedPreservationExhausted {
    VerifiedPreservationExhausted::for_test(
        current,
        "@operation",
        "preservation_exhausted",
        "verified",
        (),
    )
    .unwrap()
}
