//! P-WIRE and the M5b boundary tripwires (design §6, §7).
//!
//! The no-ff wire rows are already decode-legal and validator-clean in this
//! tree; what M5b owes is the *consumption* table — what a decoded two-parent
//! action reconciles to — plus the negative row for a forged non-canonical
//! spec, and the structural tripwires A1's activation checklist will invert.

use std::fs;
use std::path::Path;

use super::super::authority::V1LifecycleRequest;
use super::super::forward::ForwardRuntime;
use super::super::reverse::ReverseRuntime;
use super::forward::{
    self, Fixture, Kind, commit_facts, execute_then_crash, freeze_without_mutation, frozen_no_ff,
    inject_unknown_field, record_text, seed_open, stored_action,
};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::status::{PendingActionReconciliation, reconcile_pending_action};
use crate::workspace_ops::merge::{
    MergeParticipantRecord, OperationState, ParticipantState, PendingMergeActionKind,
};

#[test]
fn forged_non_canonical_signature_spec_executes_then_never_reconciles() {
    let (fixture, frozen) = frozen_no_ff("merge-v1-no-ff-forged-signature");
    let canonical = frozen.commit_spec.clone().unwrap().author.name;

    // Space padding clears the wire bounds (`action.rs:103-109`), which forbid
    // only NUL/CR/LF and out-of-range offsets.
    pad_author_name(&fixture);
    let forged = stored_action(&fixture).unwrap().commit_spec.unwrap();
    assert_eq!(forged.author.name, format!("  {canonical}  "));

    // libgit2 trims the "crud", so the commit executes and is created.
    let created = execute_then_crash(&fixture);
    let facts = commit_facts(&fixture.member, &created);
    assert_eq!(facts.parents.len(), 2);
    assert_eq!(facts.author.0, canonical, "libgit2 trimmed the forged name");

    // Restart-based reconciliation of the still-pending action can never
    // adopt it: permanently Ambiguous, typed stop, no wrong evidence.
    let row = participant(&fixture);
    assert!(matches!(
        reconcile_pending_action(&fixture.backend, &fixture.root.path, "mem_a", &row).unwrap(),
        PendingActionReconciliation::Ambiguous { .. }
    ));
    let context = forward::context();
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
            .is_none(),
        "availability is lost, but no wrong evidence is ever adopted"
    );
    let refused = forward::run_production(&fixture, &mut runtime, V1LifecycleRequest::Continue)
        .err()
        .unwrap();
    assert_eq!(refused.code, ErrorCode::RecoveryEvidenceMismatch);
}

#[test]
fn two_parent_restart_reconciliation_rows() {
    let (fixture, frozen) = frozen_no_ff("merge-v1-no-ff-reconciliation-rows");
    assert_eq!(frozen.kind, PendingMergeActionKind::TrueMerge);
    let row = participant(&fixture);
    let foreign = "b".repeat(40);

    // Exactly at `before_commit` with the frozen spec still valid.
    assert_eq!(
        reconcile_pending_action(&fixture.backend, &fixture.root.path, "mem_a", &row).unwrap(),
        PendingActionReconciliation::NotStarted
    );

    // Intent mismatch: the action no longer equals the frozen participant row
    // (`integration.rs:180-182`).
    let mut drifted = row.clone();
    drifted.pending_action.as_mut().unwrap().before_commit = foreign.clone();
    assert!(matches!(
        reconcile_pending_action(&fixture.backend, &fixture.root.path, "mem_a", &drifted).unwrap(),
        PendingActionReconciliation::Ambiguous { .. }
    ));

    // Merge-head mismatch (`integration.rs:183-189`).
    let mut mismatched = row.clone();
    mismatched.expected_merge_head = Some(foreign);
    assert!(matches!(
        reconcile_pending_action(&fixture.backend, &fixture.root.path, "mem_a", &mismatched)
            .unwrap(),
        PendingActionReconciliation::Ambiguous { .. }
    ));

    // Completed only at the exact frozen two-parent commit, field-wise.
    let created = execute_then_crash(&fixture);
    assert_eq!(
        reconcile_pending_action(
            &fixture.backend,
            &fixture.root.path,
            "mem_a",
            &participant(&fixture)
        )
        .unwrap(),
        PendingActionReconciliation::Completed {
            resulting_commit: created
        }
    );
}

#[test]
fn no_ff_record_unknown_fields_survive_rewrite_and_retire_on_reconciliation() {
    let (fixture, _) = frozen_no_ff("merge-v1-no-ff-unknown-fields");
    inject_unknown_field(&fixture, &["pending_action"], "action_probe");
    inject_unknown_field(&fixture, &["pending_action", "commit_spec"], "spec_probe");

    // A record rewrite that keeps the action pending must carry them through.
    fs::write(fixture.member.join("untracked.txt"), "drift\n").unwrap();
    let context = forward::context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);
    let stopped =
        forward::run_production(&fixture, &mut runtime, V1LifecycleRequest::Continue).unwrap();
    assert_eq!(
        stopped.current().record().state,
        OperationState::RecoveryRequired
    );
    let pending = stored_action(&fixture).unwrap();
    assert!(pending.extensions.contains_key("action_probe"));
    assert!(
        pending
            .commit_spec
            .as_ref()
            .unwrap()
            .extensions
            .contains_key("spec_probe")
    );

    // Exact reconciliation retires the container, and its unknown fields.
    fs::remove_file(fixture.member.join("untracked.txt")).unwrap();
    let mut resumed = ForwardRuntime::new(&fixture.backend, &context);
    let response =
        forward::run_production(&fixture, &mut resumed, V1LifecycleRequest::Continue).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::Merged);
    assert!(row.pending_action.is_none());
    let text = record_text(&fixture);
    assert!(!text.contains("action_probe"));
    assert!(!text.contains("spec_probe"));
}

/// T-3 (design §6): no writer output and no positive-path fixture serializes
/// the no-ff wire row; the deliberate negative fixtures are the allowlist.
#[test]
fn no_writer_output_or_positive_fixture_serializes_the_no_ff_wire_row() {
    // Built at runtime so this assertion is not itself a corpus hit.
    let serialized = format!("mode:{}no_ff", ' ');
    assert_eq!(
        files_containing(&serialized),
        [
            // Production: the v0 forged-action resume gate's typed message.
            "workspace_ops/merge/continue_op/execution.rs",
            // Deliberate negative fixture: the T-2 envelope rejection.
            "workspace_ops/merge/store/tests.rs",
            // The gate package's deliberate negative fixtures (T-6).
            "workspace_ops/tests/g23/continue_v0_gate.rs",
        ]
    );
}

/// T-3 second needle (M5b Code review round-1 [P2-1]): the literal scan
/// above cannot trip on a writer that serializes the no-ff mode variant
/// programmatically, so the variant token's whole source surface is pinned
/// here — any new file mentioning it, constructor or pattern alike, joins
/// this list only through deliberate review. The variant is spelled at
/// runtime so this suite is not its own hit.
///
/// RE-PINNED BY A1 ([P3-1]). The pre-A1 annotation closed semantic drift
/// inside an already-listed production file by subsumption: "the v1 writer is
/// `cfg(test)` (no production path writes any v1 row)". That reasoning
/// expired with the activation — the contract-§2 writer floor now writes v1
/// rows for `--no-ff`, and the mode reaches the v1 lifecycle instead of a
/// typed refusal. What the scan still buys is unchanged: no file names this
/// variant without review. Four files joined, each named below.
#[test]
fn no_ff_mode_mentions_stay_inside_the_pinned_surface() {
    let variant = format!("{}Ff", "No");
    assert_eq!(
        files_containing(&variant),
        [
            // Protocol enum declaration (accepted wire vocabulary).
            "protocol/generated.rs",
            // Production refusals: the F-1 v0 resume gate (survives A1; T-6).
            "workspace_ops/merge/continue_op/execution.rs",
            // Model enum declaration.
            "workspace_ops/merge/model/lifecycle.rs",
            // Validator arms refusing durable no-ff rows on the v0 view.
            "workspace_ops/merge/model/v1/validate/action.rs",
            "workspace_ops/merge/model/v1/validate/action_tests.rs",
            // JOINED AT A1: the contract-§2 writer floor's requested-semantic
            // that selects v1 for `--no-ff`.
            "workspace_ops/merge/model/version.rs",
            // v0 archive corpus fixtures.
            "workspace_ops/merge/record_wire/archive/tests/v0.rs",
            // v0 readers recognizing-and-classifying the foreign row; the
            // adapter's UnsupportedLegacyMode refusal survives A1.
            "workspace_ops/merge/record_wire/open_v0/adapter.rs",
            "workspace_ops/merge/record_wire/open_v0/structural.rs",
            // Dispatch: pre-A1 the refusal routing, post-A1 the coupled
            // pair's comment naming the exclusion that fell with it.
            "workspace_ops/merge/runtime/dispatch.rs",
            // The v1 lifecycle's construction site — production-reachable at
            // A1, and still the only one (T-4 pins the forced-commit half).
            "workspace_ops/merge/v1_lifecycle/authority/observe/forward.rs",
            // JOINED AT A1: the v1 lifecycle's production start owner.
            "workspace_ops/merge/v1_lifecycle/start.rs",
            // This package's own suites (cfg(test)).
            "workspace_ops/merge/v1_lifecycle/tests/forward.rs",
            // Production validation: no-ff now validates like every start.
            "workspace_ops/merge/validate.rs",
            // JOINED AT A1: the activation's own suite.
            "workspace_ops/tests/g23/a1_activation.rs",
            // The gate package's v0 suites (T-6) and compatibility corpora.
            "workspace_ops/tests/g23/atomic_upgrade_v0.rs",
            "workspace_ops/tests/g23/compatibility_v0.rs",
            "workspace_ops/tests/g23/compatibility_v0_edges.rs",
            "workspace_ops/tests/g23/continue_v0_gate.rs",
        ]
    );
}

/// T-4 (design §6): every mention of the forced merge-commit preparation mode
/// stays inside the declaration, its default-trait rejection arm, the
/// `cfg(test)` v1 lifecycle, and the backend test — no production caller
/// constructs it before A1. The variant name is spelled at runtime only, so
/// this suite is never itself a corpus hit.
#[test]
fn force_merge_commit_construction_sites_stay_v1_lifecycle_only() {
    let variant = format!("{}MergeCommit", "Force");
    assert_eq!(
        files_containing(&variant),
        [
            // Declaration plus the default-trait rejection arm.
            "git/gitbackend/contract.rs",
            // Backend-level test of the forced arm.
            "git/tests/g12.rs",
            // The only construction site: the cfg(test) v1 lifecycle.
            "workspace_ops/merge/v1_lifecycle/authority/observe/forward.rs",
        ]
    );
}

fn participant(fixture: &forward::Fixture) -> MergeParticipantRecord {
    let mut row = fixture.model.participants["mem_a"].clone();
    row.pending_action = stored_action(fixture);
    row
}

/// Forge a non-canonical (space-padded) author name into the durable spec.
fn pad_author_name(fixture: &forward::Fixture) {
    let path = forward::record_path(fixture);
    let mut document: serde_yaml::Value = serde_yaml::from_str(&record_text(fixture)).unwrap();
    let author =
        &mut document["participants"]["mem_a"]["pending_action"]["commit_spec"]["author"]["name"];
    let padded = format!("  {}  ", author.as_str().unwrap());
    *author = serde_yaml::Value::String(padded);
    fs::write(path, serde_yaml::to_string(&document).unwrap()).unwrap();
}

/// Crate-relative paths of every `.rs` source file containing `needle`.
fn files_containing(needle: &str) -> Vec<String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    let mut stack = vec![source.clone()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }
            if fs::read_to_string(&path).unwrap().contains(needle) {
                hits.push(
                    path.strip_prefix(&source)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    hits.sort();
    hits
}

/// L5 / M5b-IMPL [P3-3] — the abandonment witness, committed.
///
/// M5b's claim that service-level abandonment is mode-blind rested on an
/// un-committed experiment ("Normal-mode control probe fails identically").
/// That review corroborated it structurally only — `abandon()`
/// (`transition/reduce/participant.rs:220-241`) and the NotStarted proof
/// (`:105-118`) read neither mode nor action kind — and handed A1 the duty of
/// an executable witness. This is it.
///
/// The probe freezes a durable participant action WITHOUT executing it (the
/// `VerifiedParticipantNotStarted` shape), then drives the production
/// `service::run` seam with `Abort` once under `Normal` and once under
/// no-ff, and requires the two service-level outcomes to be the same shape.
/// It asserts EQUALITY, not a particular verdict: if the modes ever diverge
/// here the witness fails and says so, whichever way abandonment resolves.
#[test]
fn service_level_abandonment_of_a_not_started_action_is_mode_blind() {
    // Both arms are built by helpers that own the mode, so this file still
    // never spells the mode variant and stays out of its own T-3 corpus.
    let normal = abandonment_outcome(&frozen_normal("merge-v1-abandon-normal"));
    let no_ff = abandonment_outcome(&frozen_no_ff("merge-v1-abandon-no-ff").0);
    assert_eq!(
        normal, no_ff,
        "service-level abandonment of a NotStarted action must not read the mode"
    );

    // Pin the shape too, so the equality above cannot pass vacuously (two
    // identical `Applied` outcomes would also compare equal). What both modes
    // produce today is M5b's "service-level abandonment unreachability",
    // executable for the first time here: the reverse entry cannot bind an
    // abandonment transition against a NotStarted action, so the service
    // refuses before any mutation rather than retiring the action.
    assert_eq!(
        normal,
        AbandonmentOutcome::Refused {
            code: ErrorCode::MergeRecoveryRequired,
            message: "v1 transition predecessor or authority mismatch".to_owned(),
        }
    );
}

/// The mode-independent shape of one service-level abandonment outcome.
///
/// Fixture-specific data (temporary paths, fixture names, commit ids) is
/// deliberately excluded: only what the abandonment decision itself produces
/// is compared.
#[derive(Debug, Eq, PartialEq)]
enum AbandonmentOutcome {
    Refused {
        code: ErrorCode,
        message: String,
    },
    Applied {
        operation_state: OperationState,
        participant_state: ParticipantState,
        pending_action_retired: bool,
    },
}

/// The ordinary-mode twin of `frozen_no_ff`: a fast-forwardable fixture whose
/// action is frozen and durable, with the member repository untouched. The
/// fixture model's default mode is the ordinary one, so nothing is assigned.
fn frozen_normal(name: &str) -> Fixture {
    let fixture = forward::fixture(name, Kind::FastForward);
    seed_open(&fixture);
    freeze_without_mutation(&fixture);
    fixture
}

fn abandonment_outcome(fixture: &Fixture) -> AbandonmentOutcome {
    assert!(
        stored_action(fixture).is_some(),
        "the NotStarted precondition requires a durable pending action"
    );

    let context = forward::context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);
    match forward::run_production(fixture, &mut runtime, V1LifecycleRequest::Abort) {
        Err(error) => AbandonmentOutcome::Refused {
            code: error.code,
            message: error.message,
        },
        Ok(response) => {
            let record = response.current().record();
            AbandonmentOutcome::Applied {
                operation_state: record.state,
                participant_state: record.participants["mem_a"].state,
                pending_action_retired: record.participants["mem_a"].pending_action.is_none(),
            }
        }
    }
}
