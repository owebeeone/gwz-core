//! P-DET — determinism of the frozen two-parent commit (M5b design §4, §7).
//!
//! Every suite here proves the same sentence from a different side: after the
//! durable action write, the full byte content of the integration commit —
//! and therefore its OID — is a pure function of the persisted
//! `PendingMergeAction` (design §4.1). Identity is pinned by `commit_file`'s
//! fixture config (`g02.rs` precedent), so the function under test is
//! frozen-action → OID, never the runner's environment (§4.3).

use std::path::{Path, PathBuf};

use super::super::authority::V1LifecycleRequest;
use super::super::forward::ForwardRuntime;
use super::super::reverse::ReverseRuntime;
use super::forward::{
    self, CapturingRuntime, CountingRuntime, Crash, commit_facts, frozen_no_ff, head_commit,
    stored_action,
};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::{
    OperationState, ParticipantState, PendingGitSignature, PendingMergeAction,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn no_ff_commit_oid_is_a_pure_function_of_the_frozen_action() {
    let (fixture, frozen) = frozen_no_ff("merge-v1-no-ff-oid-purity");
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.before.as_str()),
        "the action freezes before any Git mutation"
    );

    // An independent clone that has never seen the integration commit
    // recomputes it from the frozen action's inputs alone (design §4.1).
    let scratch = TempDir::new("merge-v1-no-ff-oid-purity-clone");
    let offline = offline_commit_oid(&clone_member(&fixture, &scratch), &frozen);

    let context = forward::context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);
    let response =
        forward::run_production(&fixture, &mut runtime, V1LifecycleRequest::Continue).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::Merged);
    assert_eq!(
        row.resulting_commit.as_deref(),
        Some(offline.as_str()),
        "the executed two-parent commit is the frozen action's pure function"
    );
}

#[test]
fn no_ff_reexecution_after_crash_is_byte_identical() {
    let (fixture, frozen) = frozen_no_ff("merge-v1-no-ff-byte-identical");
    let scratch = TempDir::new("merge-v1-no-ff-byte-identical-clone");
    let offline = offline_commit_oid(&clone_member(&fixture, &scratch), &frozen);

    // Execute, then crash after ref publication and before the outcome write.
    let context = forward::context();
    let mut crashing = CapturingRuntime::new(
        ForwardRuntime::new(&fixture.backend, &context),
        Crash::AfterExecution,
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            forward::run(&fixture, &mut crashing)
        }))
        .is_err()
    );
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(offline.as_str()),
        "the published commit is byte-identical to the offline computation"
    );
    assert_eq!(crashing.frozen.as_ref(), Some(&frozen));

    // Restart adopts the created commit; it is never re-executed.
    let mut resumed = CountingRuntime {
        inner: ForwardRuntime::new(&fixture.backend, &context),
        executions: 0,
    };
    let response = forward::run(&fixture, &mut resumed).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(response.current().record().state, OperationState::Completed);
    assert_eq!(row.state, ParticipantState::Merged);
    assert_eq!(row.resulting_commit.as_deref(), Some(offline.as_str()));
    assert_eq!(resumed.executions, 5, "only finalization actions execute");
}

#[test]
fn no_ff_crash_between_worktree_materialization_and_ref_publication_classifies_ambiguous_and_refuses_abandonment()
 {
    let (fixture, frozen) = frozen_no_ff("merge-v1-no-ff-mid-execution-window");
    let scratch = TempDir::new("merge-v1-no-ff-mid-execution-clone");
    let offline = offline_commit_oid(&clone_member(&fixture, &scratch), &frozen);
    let spec = frozen.commit_spec.clone().unwrap();

    // Reproduce the `merge_prepared.rs:328-333` window: the worktree and
    // index carry the frozen tree, the ref still points at `before_commit`.
    checkout_tree(&fixture.member, &spec.tree_oid, false);
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.before.as_str())
    );

    // Abandonment is refused: the frozen action is not observably NotStarted.
    let context = forward::context();
    let mut reverse = ReverseRuntime::new(&fixture.backend, &context);
    let abandoned = forward::run_production(&fixture, &mut reverse, V1LifecycleRequest::Abort);
    let row_after_abort = stored_action(&fixture);
    assert_eq!(
        row_after_abort.as_ref(),
        Some(&frozen),
        "the frozen action must survive the refused abandonment"
    );
    let refused_abandonment = abandoned
        .err()
        .expect("an unobservable NotStarted must not authorize abandonment");
    assert_eq!(
        refused_abandonment.code,
        ErrorCode::MergeRecoveryRequired,
        "{}",
        refused_abandonment.message
    );
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.before.as_str())
    );

    // Continue classifies Ambiguous and records the typed stop; a second
    // continue over the same ambiguity is refused, never re-derived.
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);
    let stopped =
        forward::run_production(&fixture, &mut runtime, V1LifecycleRequest::Continue).unwrap();
    assert_eq!(
        stopped.current().record().state,
        OperationState::RecoveryRequired
    );
    assert!(
        stopped.current().record().participants["mem_a"]
            .resulting_commit
            .is_none()
    );
    let refused = forward::run_production(&fixture, &mut runtime, V1LifecycleRequest::Continue)
        .err()
        .unwrap();
    assert_eq!(refused.code, ErrorCode::RecoveryEvidenceMismatch);

    // The operator restores the before-state worktree; reconciliation
    // re-yields NotStarted and re-execution produces the identical OID.
    checkout_tree(
        &fixture.member,
        &commit_facts(&fixture.member, &fixture.before).tree,
        true,
    );
    let mut resumed = ForwardRuntime::new(&fixture.backend, &context);
    let response =
        forward::run_production(&fixture, &mut resumed, V1LifecycleRequest::Continue).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::Merged);
    assert_eq!(
        row.resulting_commit.as_deref(),
        Some(offline.as_str()),
        "re-execution from the restored before-state is byte-identical"
    );
}

#[test]
fn frozen_signature_timestamps_survive_restart() {
    let (fixture, frozen) = frozen_no_ff("merge-v1-no-ff-frozen-timestamps");
    let spec = frozen.commit_spec.clone().unwrap();

    // persist → decode: the durable spec on disk carries the frozen instants.
    let decoded = stored_action(&fixture).unwrap().commit_spec.unwrap();
    assert_eq!(decoded, spec);

    // execute: the created commit reuses them without re-stamping.
    let context = forward::context();
    let mut crashing = CapturingRuntime::new(
        ForwardRuntime::new(&fixture.backend, &context),
        Crash::AfterExecution,
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            forward::run(&fixture, &mut crashing)
        }))
        .is_err()
    );
    let published = head_commit(&fixture).unwrap();
    let facts = commit_facts(&fixture.member, &published);
    assert_eq!(facts.author, identity(&spec.author));
    assert_eq!(facts.committer, identity(&spec.committer));

    // adopt: restart matches those same fields and retires the action.
    let mut resumed = ForwardRuntime::new(&fixture.backend, &context);
    let response =
        forward::run_production(&fixture, &mut resumed, V1LifecycleRequest::Continue).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(row.resulting_commit.as_deref(), Some(published.as_str()));
    assert!(row.pending_action.is_none());
    let adopted = commit_facts(&fixture.member, &published);
    assert_eq!(adopted.author, identity(&spec.author));
    assert_eq!(adopted.committer, identity(&spec.committer));
}

/// Clone the member repository into a scratch root that has never observed
/// the integration commit.
fn clone_member(fixture: &forward::Fixture, scratch: &TempDir) -> PathBuf {
    let destination = scratch.path.join("clone");
    git2::Repository::clone(fixture.member.to_str().unwrap(), &destination).unwrap();
    destination
}

/// Recompute the integration commit from the seven frozen inputs enumerated
/// by design §4.1, using nothing but the persisted action.
fn offline_commit_oid(clone: &Path, action: &PendingMergeAction) -> String {
    let spec = action
        .commit_spec
        .as_ref()
        .expect("a two-parent action freezes a commit spec");
    let repository = git2::Repository::open(clone).unwrap();
    let object = |oid: &str| git2::Oid::from_str(oid).unwrap();
    let tree = repository.find_tree(object(&spec.tree_oid)).unwrap();
    let first = repository
        .find_commit(object(&action.before_commit))
        .unwrap();
    let second = repository
        .find_commit(object(&action.source_commit))
        .unwrap();
    repository
        .commit(
            None,
            &signature(&spec.author),
            &signature(&spec.committer),
            &action.commit_message,
            &tree,
            &[&first, &second],
        )
        .unwrap()
        .to_string()
}

fn signature(frozen: &PendingGitSignature) -> git2::Signature<'static> {
    git2::Signature::new(
        &frozen.name,
        &frozen.email,
        &git2::Time::new(frozen.time_seconds, frozen.timezone_offset_minutes),
    )
    .unwrap()
}

fn identity(frozen: &PendingGitSignature) -> (String, String, i64, i32) {
    (
        frozen.name.clone(),
        frozen.email.clone(),
        frozen.time_seconds,
        frozen.timezone_offset_minutes,
    )
}

/// Materialize `tree` into the member worktree and index without moving any
/// ref — the shape `merge_prepared.rs:328-329` leaves mid-execution.
fn checkout_tree(member: &Path, tree: &str, force: bool) {
    let repository = git2::Repository::open(member).unwrap();
    let object = repository
        .find_tree(git2::Oid::from_str(tree).unwrap())
        .unwrap();
    let mut checkout = git2::build::CheckoutBuilder::new();
    if force {
        checkout.force();
    }
    repository
        .checkout_tree(object.as_object(), Some(&mut checkout))
        .unwrap();
}
