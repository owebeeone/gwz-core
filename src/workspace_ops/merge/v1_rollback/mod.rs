//! The v1 reverse path's observation and execution surface.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** Until this milestone these three
//! modules were `mod v1_rollback` blocks embedded in the v0 abort engine's
//! own files (`merge/abort/{evidence,preflight,participants}.rs`). The engine
//! is deleted; the v1 halves live here, under a name that says what they are.
//! The root-metadata half is `merge/root/v1_rollback.rs`, beside the rest of
//! the root participant's observation surface.

pub(in crate::workspace_ops::merge) mod evidence;
pub(in crate::workspace_ops::merge) mod participants;
mod preflight;

pub(in crate::workspace_ops::merge) use evidence::{
    V1EvidenceRollbackObservation, execute_v1_evidence_rollback, observe_v1_evidence_rollback,
    preflight_v1_evidence, v1_evidence_residue_after_selected_root_is_exact,
};
pub(in crate::workspace_ops::merge) use participants::{
    V1ParticipantRollbackObservation, execute_v1_participant_rollback,
    observe_v1_participant_rollback, terminal_v1_participant_is_exact,
    verify_v1_no_mutation_participant,
};
pub(in crate::workspace_ops::merge) use preflight::preflight_v1_rollback;

#[cfg(test)]
#[path = "tests/evidence_shape.rs"]
mod evidence_shape_tests;
