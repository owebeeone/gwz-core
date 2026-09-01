//! Pure first-catalog grammar and one-edge recovery classification.
//!
//! R2-C1 deliberately contains no filesystem writer. Physical observation is
//! supplied by the retained provider, and R2-C2 will consume these decisions.

// [2026-09-02, R2-E E4.4-6-B: the E4.2-E4.6 / "awaiting R2-E consumer conversion" range is STALE — E4.4-E4.6 as chartered do not start (GwzM5-8R2E-CapabilityFreeAmendment.md §7); E4.7 EXPIRES or RE-REASONS each, and this package only dates them.] Class members here: the three `dead_code` allows on `bootstrap`, `classifier` and `enumeration`, and the `unused_imports` allow on the re-export below.
// R2-D Phase 4 Step 4.3 (settle item 7) narrowed one subtree-wide
// `allow(dead_code)` in `checked_artifact/mod.rs` down to the three children
// that still carry a frozen surface. `scratch` needs none: every item it
// exports has a production consumer today.
// R2-E Phase E4 Step E4.1 (2026-09-01) ACTIVATED this owner: `recover_or_create`
// now has a production caller — `checked_artifact/entry.rs`'s
// `activate_workspace_catalog`, reached from the forward v1 paths (the
// activated lease and dispatch's pre-upgrade viability window; the plain
// abort lease never activates) — and `interface_tests/catalog_activation_pin.rs` pins the count at
// exactly one. The blanket allow that stood here for the unactivated entry
// point is retired; what remains dead is what `dead_code` still names.
#[allow(
    dead_code,
    reason = "E4.1 activated `recover_or_create`; what stays dead on the retained catalog is the \
              two admitted-action capabilities no activation path uses — \
              `retire_admitted_action` and `observe_roaming_anchor_home` — whose first production \
              consumers are the E4.2-E4.6 conversions"
)]
mod bootstrap;
#[allow(
    dead_code,
    reason = "E4.1 activated the owner above, which consumes this classifier on every drive; what \
              stays dead is the classifier's own unreached vocabulary, whose readers are the \
              E4.2-E4.6 consumers and its interface suites"
)]
mod classifier;
#[allow(
    dead_code,
    reason = "E4.1 activated the owner above, which exercises the catalog-root budgets on every \
              drive; what stays dead is the enumeration surface no activation path reads yet — \
              its first production reader is an E4.2-E4.6 consumer"
)]
mod enumeration;
mod scratch;

// E4.1: `recover_or_create` is re-exported here for its production caller and
// `OpaqueRetainedCatalogV1`/`CatalogOwnerEdgeV1` travel with it, so the blanket
// forward-reference allow is retired down to the one name still unconsumed.
#[allow(
    unused_imports,
    reason = "the owner TYPE is re-exported for the E4.2-E4.6 consumers that will name it; E4.1's \
              caller needs only the free function and the opaque retained result"
)]
pub(in crate::checked_artifact) use bootstrap::{
    CatalogOwnerEdgeV1, CatalogOwnerV1, OpaqueRetainedCatalogV1, recover_or_create,
};
#[allow(
    unused_imports,
    reason = "R2-C1 exports the pure classifier before C2 consumes it"
)]
pub(in crate::checked_artifact) use classifier::*;
#[allow(
    unused_imports,
    reason = "R2-C1 exports parent grammar before the C2 owner consumes it"
)]
pub(in crate::checked_artifact) use enumeration::*;
pub(in crate::checked_artifact) use scratch::*;
