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
    CatalogAdmissionOccupancyV1, CatalogAdmissionOwnerV1, CatalogOccupancyV1,
    ObservedActionDirectoryV1, RecordObservationV1,
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
        // Defence in depth, deliberately kept unreachable: `interior::exact_row`
        // refuses a malformed-recognized or foreign child before the census can
        // charge one, so no observation that reaches here can carry an unowned
        // row (`provider/interior.rs`, the unowned-child refusal). The stop
        // stands so a future widening of that refusal cannot silently make the
        // driver admit against an unclassified root.
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
                // §7's capacity rule, applied to **new** admissions only. The
                // catalog root is zero-headroom by construction
                // (`MAX_ROOT_ENTRIES = MAX_INFRASTRUCTURE_ENTRIES +
                // MAX_ACTIVE_ACTION_DIRS`), so a 65th active action would
                // publish a row that the very next bounded observation refuses
                // — and keeps refusing, because no Phase-1 edge removes an
                // action row and every sealed path, `recover_or_create`
                // included, runs through that observation. Refusing before the
                // first durable write is the only place the catalog is still
                // recoverable.
                //
                // Resume is unaffected: `admit` above has already returned for
                // an exact existing action, and a *resumed in-flight* admission
                // never reaches this arm (it drives `Preparing`). That path is
                // closed structurally instead, at the commit point, by the
                // `AdmissionCatalogInterior` destination recheck
                // (`provider/publication.rs`), which re-proves the same bound
                // inside the acquisition window.
                //
                // The Phase-4 debt this comment used to record is **paid here**,
                // by R2-E E3.1 — the phase that lands retirement, exactly as it
                // said. `CatalogOccupancyV1::can_admit_new` charges the frozen
                // retirement-credit inequality against the retired root, and
                // the observation now carries that root's bounded count because
                // E3.1's T1 widening is what made it readable at all.
                //
                // The note's closing clause — "unlike the active bound,
                // exhausting retirement credit cannot make the catalog
                // unobservable" — is **refuted on this tree and withdrawn**
                // (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §2.3): before
                // the widening a *single* retired child made the catalog
                // unobservable, and therefore unrecoverable. That is why the
                // widening is the precondition of this gate rather than a
                // convenience beside it.
                //
                // This is the frozen occupancy type's first production caller.
                // It is a strict strengthening of the bare active-row stop it
                // replaces: the same 65th-active-row refusal, plus
                // `RetiredLimitExceeded` and the credit rule that reserves one
                // retired slot for every action still outstanding.
                let occupancy = CatalogOccupancyV1::new(
                    observed.census.active_actions,
                    observed.retired_action_dirs,
                    CatalogAdmissionOccupancyV1::Idle,
                )
                .map_err(|_| stop("the catalog root is outside its frozen occupancy bounds"))?;
                if !occupancy.can_admit_new() {
                    return Err(stop(
                        "the catalog root already holds the frozen active-action budget",
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
