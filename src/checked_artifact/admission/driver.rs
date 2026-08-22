//! The nine-step durable admission sequence.
//!
//! Controlling text: `GwzM5-8R4bR2ConsumerCheckpoint.md` §7 (:203-224) for the
//! sequence and the capacity rule, §6 (:199-201) for the bounded global row
//! classification, and `GwzM5-8R2DInterfaceFreeze.md` §3.1 for the seam this
//! body sits behind.
//!
//! The driver decides; it never mutates. Each iteration takes one read-only
//! observation through the opaque retained catalog, resolves it to exactly one
//! bounded durable edge, and asks the catalog to execute that edge. Every edge
//! is idempotent under restart because every name it uses is derived — the
//! three frozen `ActionAdmission*` slots and the derived
//! `RootEntryNameV1::ActiveAction` name — so a retry reuses names and capacity
//! and never chooses a nonce (`GwzM5-8R4bP1P2-RemPlan-4.md` §4 R2 stop clause
//! :1089-1092).

use crate::checked_artifact::capability::CheckedFsError;
use crate::checked_artifact::catalog::OpaqueRetainedCatalogV1;
use crate::checked_artifact::protocol::{
    ActionAdmissionEdgeV1, ActionAdmissionObservationV1, ActionCapacityReservationV1,
    ActionDirectoryAdmissionV1, AdmissionHandoffDecisionV1, AdmittedActionV1,
    CatalogAdmissionOwnerV1, ObservedActionDirectoryV1, RecordObservationV1,
};

const ADMISSION_FACT: &str = "action admission";

/// The virgin sequence settles in eight durable edges plus one terminating
/// observation, and every iteration either issues the handoff, stops, or
/// executes an edge that strictly advances the durable state. The bound is a
/// liveness assertion, not a policy: reaching it means the sequence failed to
/// converge and admission stops rather than looping.
const MAX_ADMISSION_STEPS: usize = 24;

pub(super) fn resume_or_admit(
    catalog: &OpaqueRetainedCatalogV1<'_>,
    expected: &ActionCapacityReservationV1,
) -> Result<AdmittedActionV1, CheckedFsError> {
    let owner = CatalogAdmissionOwnerV1::new();
    let idle = ActionDirectoryAdmissionV1::idle();
    let preparing = ActionDirectoryAdmissionV1::preparing(expected);
    for _ in 0..MAX_ADMISSION_STEPS {
        // Steps 1-2 and step 8: the plan and schedule are derived from
        // read-only observations, so this path mutates nothing.
        let observed = catalog.observe_admission(expected)?;
        if observed.census.has_unowned_row() {
            return Err(stop(
                "bounded global classification found a malformed or foreign catalog row",
            ));
        }
        let edge = match resolve(&observed, expected)? {
            // The install half of a §7 step 3 / step 7 transition is in
            // flight: the active name is free and the scratch already carries
            // the next durable state.
            AdmissionDriveV1::InstallScratch => ActionAdmissionEdgeV1::PublishAdmissionRecord,
            // The write-ahead half is durable; the superseded record has to
            // leave the active name before the no-replace publication.
            AdmissionDriveV1::RetireActive => ActionAdmissionEdgeV1::RetireAdmissionRecord,
            AdmissionDriveV1::Idle => {
                // Step 9: the handoff is issued only from idle + missing
                // staging + an exact final reservation with no extra children.
                if let Some(admitted) = owner.admit(
                    &idle,
                    expected,
                    &observed.staging,
                    &observed.final_directory,
                ) {
                    return Ok(admitted);
                }
                // §7 (:205-207): the bounded global lookup found the derived
                // final action name occupied by something that is not this
                // exact action, so admission stops rather than admitting a
                // second one.
                if !matches!(observed.final_directory, ObservedActionDirectoryV1::Missing) {
                    return Err(stop(
                        "a conflicting or ambiguous action occupies the derived final action name",
                    ));
                }
                // Step 3: persist `Idle -> Preparing`.
                ActionAdmissionEdgeV1::WriteAdmissionScratch(&preparing)
            }
            AdmissionDriveV1::Preparing => match owner.classify_handoff(
                &preparing,
                expected,
                &observed.staging,
                &observed.final_directory,
            ) {
                AdmissionHandoffDecisionV1::CreateStaging => {
                    ActionAdmissionEdgeV1::CreateStagingDirectory
                }
                AdmissionHandoffDecisionV1::WriteOrRewriteReservation => {
                    ActionAdmissionEdgeV1::WriteResidentReservation
                }
                AdmissionHandoffDecisionV1::PublishStaging => {
                    ActionAdmissionEdgeV1::PublishStagingAction
                }
                // Step 7: persist `Preparing -> Idle`.
                AdmissionHandoffDecisionV1::ReplacePreparingWithIdle => {
                    ActionAdmissionEdgeV1::WriteAdmissionScratch(&idle)
                }
                AdmissionHandoffDecisionV1::Ambiguous => {
                    return Err(stop(
                        "the staging and final action directories are ambiguous for this reservation",
                    ));
                }
            },
        };
        catalog.execute_admission_edge(edge, expected)?;
    }
    Err(stop("the durable admission sequence did not converge"))
}

/// What the durable `ActionAdmission*` pair says the sequence must do next.
enum AdmissionDriveV1 {
    Idle,
    Preparing,
    InstallScratch,
    RetireActive,
}

/// The durable admission state carried by one slot, read only through the two
/// frozen public predicates on [`ActionDirectoryAdmissionV1`] — equality with
/// `idle()` and `matches_reservation`. A `Preparing` record for a different
/// action is `Other`, so it stops rather than being resumed.
enum AdmissionSlotStateV1 {
    Missing,
    Idle,
    Preparing,
    Other,
}

fn resolve(
    observed: &ActionAdmissionObservationV1,
    expected: &ActionCapacityReservationV1,
) -> Result<AdmissionDriveV1, CheckedFsError> {
    use AdmissionSlotStateV1::{Idle, Missing, Other, Preparing};
    let active = slot_state(&observed.record, expected);
    let scratch = slot_state(&observed.scratch, expected);
    Ok(match (active, scratch) {
        (Missing, Idle | Preparing) => AdmissionDriveV1::InstallScratch,
        (Idle, Preparing) | (Preparing, Idle) => AdmissionDriveV1::RetireActive,
        (Missing | Idle, Missing | Other) => AdmissionDriveV1::Idle,
        (Preparing, Missing | Other) => AdmissionDriveV1::Preparing,
        _ => {
            return Err(stop(
                "the durable admission record and its scratch are ambiguous",
            ));
        }
    })
}

fn slot_state(
    observed: &RecordObservationV1<ActionDirectoryAdmissionV1>,
    expected: &ActionCapacityReservationV1,
) -> AdmissionSlotStateV1 {
    match observed {
        RecordObservationV1::Missing => AdmissionSlotStateV1::Missing,
        RecordObservationV1::Exact(value) if *value == ActionDirectoryAdmissionV1::idle() => {
            AdmissionSlotStateV1::Idle
        }
        RecordObservationV1::Exact(value) if value.matches_reservation(expected) => {
            AdmissionSlotStateV1::Preparing
        }
        _ => AdmissionSlotStateV1::Other,
    }
}

fn stop(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous(ADMISSION_FACT, detail)
}

#[cfg(test)]
mod tests;
