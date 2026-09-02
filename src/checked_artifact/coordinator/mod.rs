//! Closed derivation boundary between merge-owned durable facts and R1.

/// R2-D Phase 3 Step 3.3 — the coordinator execution glue.
///
/// Deliberately **not** re-exported from this module (Step-3.3 review [P3-5]):
/// a `pub use execution::*` would need an `unused_imports` allow of its own with
/// no consumer to justify it, which is the pattern this step spent its budget
/// correcting elsewhere. R2-E's consumer names `coordinator::execution::…`
/// directly, exactly as `tests_execution.rs` already does.
///
/// *[E4.2, 2026-09-01: unimplementable as written — `mod execution;` was
/// module-private, so `coordinator::execution::…` was nameable only inside
/// `coordinator`; widened to the subsystem here, not by the rejected re-export.]*
// [2026-09-02, R2-E E4.4-6-B: the E4.2-E4.6 / "awaiting R2-E consumer conversion" range is STALE — E4.4-E4.6 as chartered do not start (GwzM5-8R2E-CapabilityFreeAmendment.md §7); E4.7 EXPIRES or RE-REASONS each, and this package only dates them.]
#[allow(
    dead_code,
    reason = "Step 3.3 wired the machinery; E4.2 converted the merge-start consumer \
              (`entry.rs` names `admit_`/`execute_merge_start_managed_parents`); the REMAINING \
              interior gains no consumer — E4.4-E4.6 do not start (dev-docs/\
              GwzM5-8R2E-CapabilityFreeAmendment.md §7). PERMANENT pending DR-1, re-measured at \
              E4.7 (2026-09-02): removal reddens `clippy -D warnings` on the E0.2 §7.4 interior."
)]
pub(in crate::checked_artifact) mod execution;
/// R2-D Phase 4 Step 4.3 (settle item 7): the subtree-wide allow that used to
/// cover this module moved here, because `identity` is the whole of the
/// coordinator's remaining frozen surface — `execution` carries its own above
/// and `schedule` needs none.
#[allow(
    dead_code,
    reason = "the merge-owned identity derivations are consumed by `execution` and by the \
              interface-test contract pins; the merge consumer that would read the REST does not \
              arrive — E4.4-E4.6 do not start (dev-docs/\
              GwzM5-8R2E-CapabilityFreeAmendment.md §7), so plan §5 item 1 is spent here. \
              PERMANENT pending DR-1, re-measured at E4.7 (2026-09-02)."
)]
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
