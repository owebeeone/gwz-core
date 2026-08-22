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
use super::forward::{
    self, commit_facts, execute_then_crash, frozen_no_ff, inject_unknown_field, record_text,
    stored_action,
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
/// this list only through deliberate review. Semantic drift inside an
/// already-listed production file is closed by subsumption pre-A1: the v1
/// writer is `cfg(test)` (no production path writes any v1 row), and durable
/// no-ff rows arriving from outside are refused by the F-1 resume gate
/// (`continue_op/execution.rs`) and the validators. The variant is spelled
/// at runtime so this suite is not its own hit.
#[test]
fn no_ff_mode_mentions_stay_inside_the_pinned_surface() {
    let variant = format!("{}Ff", "No");
    assert_eq!(
        files_containing(&variant),
        [
            // Protocol enum declaration (accepted wire vocabulary).
            "protocol/generated.rs",
            // Production refusals: the F-1 v0 resume gate.
            "workspace_ops/merge/continue_op/execution.rs",
            // Model enum declaration.
            "workspace_ops/merge/model/lifecycle.rs",
            // Validator arms refusing durable no-ff rows pre-A1.
            "workspace_ops/merge/model/v1/validate/action.rs",
            "workspace_ops/merge/model/v1/validate/action_tests.rs",
            // v0 archive corpus fixtures.
            "workspace_ops/merge/record_wire/archive/tests/v0.rs",
            // v0 readers recognizing-and-classifying the foreign row.
            "workspace_ops/merge/record_wire/open_v0/adapter.rs",
            "workspace_ops/merge/record_wire/open_v0/structural.rs",
            // Dispatch arm (refusal routing).
            "workspace_ops/merge/runtime/dispatch.rs",
            // The only construction site: the cfg(test) v1 lifecycle.
            "workspace_ops/merge/v1_lifecycle/authority/observe/forward.rs",
            // This package's own suites (cfg(test)).
            "workspace_ops/merge/v1_lifecycle/tests/forward.rs",
            // Production refusal: validate surface.
            "workspace_ops/merge/validate.rs",
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
