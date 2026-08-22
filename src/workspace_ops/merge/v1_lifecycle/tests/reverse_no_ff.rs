//! P-REV — reverse-path consumption of a frozen two-parent action
//! (M5b design §5.3, §7).
//!
//! Reverse ownership is mode-blind by construction: abandonment consumes the
//! bound exact `NotStarted` proof, rollback anchors on the *recorded result
//! commit*, and preservation anchors on the participant result. These suites
//! prove those sentences hold when that result is a no-ff two-parent commit.
//!
//! Order-independence (design §7, State P2-2): nothing here asserts the
//! absence of preservation-cursor rows, so the suites pass whether or not the
//! D3 durable-cursor wire lands first.

use std::fs;

use super::super::authority::{
    BoundExactObservation, EntryFact, ExactObservationFact, ResolvedV1Action, V1LifecycleRequest,
    V1NextAction, VerifiedParticipantNotStarted, VerifiedPublicationHandoff, next_action,
    resolve_observation,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::forward::ForwardRuntime;
use super::super::reverse::ReverseRuntime;
use super::super::store::CheckedV1Store;
use super::super::transition::{ReverseEntryKind, prepare};
use super::forward::{
    self, commit_facts, execute_then_crash, frozen_no_ff, head_commit, inject_unknown_field,
    stored_action,
};
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, ParticipantRollbackKindV1, PendingRollbackActionV1, PreservationOwnerV1,
};
use crate::workspace_ops::merge::status::{PendingActionReconciliation, reconcile_pending_action};
use crate::workspace_ops::merge::{OperationState, ParticipantState};

#[test]
fn abort_abandons_a_not_started_no_ff_action_via_bound_exact_observation() {
    assert_abandonment(V1LifecycleRequest::Abort, "merge-v1-no-ff-abort-abandon");
}

#[test]
fn preserve_abort_abandons_a_not_started_no_ff_action() {
    assert_abandonment(
        V1LifecycleRequest::Preserve,
        "merge-v1-no-ff-preserve-abandon",
    );
}

/// A pending two-parent action retires without an integration outcome only
/// through the bound exact `NotStarted` proof consumed at reverse entry
/// (record contract §7, 2026-08-09 amendment; design §5.3).
///
/// The `NotStarted` fact here is earned from the real repository against the
/// real frozen action, not asserted: that reconciliation is the no-ff claim.
/// The transition is then driven through the authority/reducer path that owns
/// abandonment, matching the tree's existing abandonment harness
/// (`dispatcher_attempt_matrix::abort_and_preserve_abandon_only_their_bound_not_started_owner`).
fn assert_abandonment(request: V1LifecycleRequest, name: &str) {
    let (fixture, frozen) = frozen_no_ff(name);
    inject_unknown_field(&fixture, &["pending_action"], "m5b_probe");
    let before_bytes = fs::read(fixture.member.join("README.md")).unwrap();
    let lease = V1MutationLease::acquire_for_test(&fixture.root.path).unwrap();
    let current = load(&fixture);
    let row = &current.record().participants["mem_a"];

    // The frozen two-parent action over an untouched repository is exactly
    // NotStarted — the observation abandonment is allowed to consume.
    assert_eq!(
        reconcile_pending_action(&fixture.backend, &fixture.root.path, "mem_a", row).unwrap(),
        PendingActionReconciliation::NotStarted
    );
    let pending = row.pending_action.clone().unwrap();
    assert_eq!(pending.kind, frozen.kind);
    assert!(
        pending.extensions.contains_key("m5b_probe"),
        "the probe rides inside the durable pending-action container"
    );

    let mut anticipated = current.record().clone();
    anticipated
        .participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action = None;
    let proof = VerifiedParticipantNotStarted::for_test(
        &current,
        "mem_a",
        "participant_action",
        "not_started",
        "mem_a".into(),
    )
    .unwrap();
    let kind = match request {
        V1LifecycleRequest::Abort => ReverseEntryKind::DirectRollback,
        V1LifecycleRequest::Preserve => ReverseEntryKind::Preservation,
        _ => unreachable!(),
    };
    let handoff = VerifiedPublicationHandoff::for_entry_test(&current, kind, &anticipated).unwrap();
    let entry = match request {
        V1LifecycleRequest::Abort => EntryFact::Rollback(Box::new(
            super::super::authority::PreparedRollbackEntry::direct_for_test(
                &current,
                &anticipated,
                handoff,
            )
            .unwrap(),
        )),
        V1LifecycleRequest::Preserve => EntryFact::Preservation(Box::new(
            super::super::authority::PreparedPreservationEntry::for_test(
                &current,
                &anticipated,
                handoff,
            )
            .unwrap(),
        )),
        _ => unreachable!(),
    };
    let V1NextAction::Observe(observation_request) = next_action(&current, request).unwrap() else {
        panic!("the persisted two-parent owner was not reconciled")
    };
    let observation = BoundExactObservation::for_test(
        &current,
        &observation_request,
        ExactObservationFact::Abandon(Box::new(proof), entry),
    )
    .unwrap();
    let ResolvedV1Action::Apply(transition) =
        resolve_observation(&current, request, observation_request, observation, None).unwrap()
    else {
        panic!("request-specific abandonment of the two-parent action was not resolved")
    };

    let rewrite = prepare(&lease, &current, transition).unwrap();

    let next = rewrite.next();
    assert_eq!(
        next.state,
        match request {
            V1LifecycleRequest::Abort => OperationState::RollingBack,
            _ => OperationState::Preserving,
        }
    );
    let row = &next.participants["mem_a"];
    assert!(
        row.pending_action.is_none(),
        "the retired action container takes its unknown fields with it"
    );
    assert!(
        row.resulting_commit.is_none(),
        "abandonment fabricates no integration outcome"
    );
    assert_ne!(row.state, ParticipantState::Merged);
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.before.as_str()),
        "the pre-action state is untouched"
    );
    assert_eq!(
        fs::read(fixture.member.join("README.md")).unwrap(),
        before_bytes
    );
    assert!(!fixture.member.join("source.txt").exists());
}

#[test]
fn abort_refuses_to_abandon_a_created_two_parent_commit() {
    let (fixture, frozen) = frozen_no_ff("merge-v1-no-ff-abort-refusal");
    let created = execute_then_crash(&fixture);
    assert_eq!(commit_facts(&fixture.member, &created).parents.len(), 2);
    assert_eq!(
        stored_action(&fixture).as_ref(),
        Some(&frozen),
        "the outcome write never happened; the action is still pending"
    );

    let context = forward::context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);
    let response =
        forward::run_production(&fixture, &mut runtime, V1LifecycleRequest::Abort).unwrap();

    // The created commit reconciles Completed first; it is recorded as the
    // participant result and then rolled back from that anchor. It is never
    // abandoned — abandonment would have retired the action with no outcome.
    let row = &response.current().record().participants["mem_a"];
    assert_eq!(
        row.resulting_commit.as_deref(),
        Some(created.as_str()),
        "the two-parent result is adopted, never abandoned"
    );
    assert_eq!(row.state, ParticipantState::RolledBack);
    assert!(row.pending_action.is_none());
    assert_eq!(response.current().record().state, OperationState::Aborted);
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.before.as_str()),
        "rollback anchors on the recorded two-parent result"
    );
}

#[test]
fn rollback_resets_a_no_ff_merged_participant_to_before_commit() {
    let (mut fixture, _) = frozen_no_ff("merge-v1-no-ff-rollback");
    let context = forward::context();
    let mut forward_runtime = ForwardRuntime::new(&fixture.backend, &context);
    let completed =
        forward::run_production(&fixture, &mut forward_runtime, V1LifecycleRequest::Continue)
            .unwrap();
    let result = completed.current().record().participants["mem_a"]
        .resulting_commit
        .clone()
        .unwrap();
    assert_eq!(commit_facts(&fixture.member, &result).parents.len(), 2);
    let before_tree = commit_facts(&fixture.member, &fixture.before).tree;

    // The recorded two-parent result is the rollback anchor; mode never
    // enters the classifier (journal contract §3).
    integrated_model(&mut fixture.model, &result);
    fixture.model.pending_rollback = Some(PendingRollbackActionV1::Participant {
        member_id: "mem_a".into(),
        action: ParticipantRollbackKindV1::ResetIntegrated,
        terminal_state: ParticipantState::RolledBack,
    });
    fixture.model.state = OperationState::RollingBack;
    forward::seed_open(&fixture);

    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);
    let response =
        forward::run_production(&fixture, &mut runtime, V1LifecycleRequest::Abort).unwrap();

    let record = response.current().record();
    assert_eq!(
        record.participants["mem_a"].state,
        ParticipantState::RolledBack
    );
    assert!(record.pending_rollback.is_none());
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.before.as_str()),
        "ResetIntegrated restores the exact before_commit"
    );
    // Clause A: the restored before-state is verified raw-byte.
    assert_eq!(
        commit_facts(&fixture.member, &fixture.before).tree,
        before_tree
    );
    assert_eq!(
        fs::read(fixture.member.join("README.md")).unwrap(),
        b"base\n"
    );
    assert!(
        !fixture.member.join("source.txt").exists(),
        "the two-parent result's content is gone from the worktree"
    );
}

#[test]
fn preservation_backup_ref_anchors_on_the_two_parent_result() {
    let (mut fixture, _) = frozen_no_ff("merge-v1-no-ff-preservation-anchor");
    let context = forward::context();
    let mut forward_runtime = ForwardRuntime::new(&fixture.backend, &context);
    let completed =
        forward::run_production(&fixture, &mut forward_runtime, V1LifecycleRequest::Continue)
            .unwrap();
    let result = completed.current().record().participants["mem_a"]
        .resulting_commit
        .clone()
        .unwrap();

    // Local work lands on top of the integration result, so the live tip is
    // not the anchor: the owner anchor is the immutable participant result.
    let live = crate::workspace_ops::tests::commit_file(
        &fixture.member,
        "later.txt",
        "later\n",
        "later",
        &[git2::Oid::from_str(&result).unwrap()],
    )
    .unwrap();
    integrated_model(&mut fixture.model, &result);
    fixture.model.state = OperationState::Preserving;

    let owners = crate::workspace_ops::merge::preserve::v1_preservation_owners(
        &fixture.backend,
        &fixture.root.path,
        &fixture.model,
    )
    .unwrap();

    assert_eq!(owners.len(), 1);
    let owner = &owners[0];
    assert_eq!(
        owner.owner,
        PreservationOwnerV1::Participant {
            member_id: "mem_a".into()
        }
    );
    assert_eq!(
        owner.anchor, result,
        "the preservation owner anchor is the two-parent participant result"
    );
    assert_eq!(owner.live_commit, live);
    assert_eq!(
        owner.backup_ref,
        format!("refs/gwz/merge/{}/mem_a/head", fixture.model.merge_id)
    );
}

/// Present the participant as a completed no-ff integration.
fn integrated_model(model: &mut MergeOperationRecordV1, result: &str) {
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Merged;
    row.resulting_commit = Some(result.to_owned());
    row.pending_action = None;
}

fn load(fixture: &forward::Fixture) -> StoredV1Record {
    CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap()
}
