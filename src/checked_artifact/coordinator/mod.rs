//! Closed derivation boundary between merge-owned durable facts and R1.

/// R2-D Phase 3 Step 3.3 — the coordinator execution glue.
///
/// Deliberately **not** re-exported from this module (Step-3.3 review [P3-5]):
/// a `pub use execution::*` would need an `unused_imports` allow of its own with
/// no consumer to justify it, which is the pattern this step spent its budget
/// correcting elsewhere. R2-E's consumer names `coordinator::execution::…`
/// directly, exactly as `tests_execution.rs` already does.
#[allow(
    dead_code,
    reason = "Step 3.3 wires the machinery; consumer conversion is R2-E (plan §4 Step 3.3, §5 item 1)"
)]
mod execution;
mod identity;
mod schedule;

pub(in crate::checked_artifact) use identity::*;
/// R2-D Step 3.3 discharges the R2 forward reference this allow carried ("before
/// production consumers are converted"): the schedule facade's production
/// consumer is now `execution::schedule_checked_action`, which reaches it
/// through `super::schedule` rather than through this hop. The crate-wide
/// re-export stays because `interface_tests/coordinator_contract.rs` and
/// `coordinator_remediation.rs` pin the facade's contract through it, and those
/// are `#[cfg(test)]` — so the item is genuinely unused in a release build.
#[allow(
    unused_imports,
    reason = "the schedule facade's production consumer is coordinator-internal; this hop serves the interface-test contracts"
)]
pub(in crate::checked_artifact) use schedule::*;

#[cfg(test)]
mod tests_execution;
