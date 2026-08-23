//! Pure first-catalog grammar and one-edge recovery classification.
//!
//! R2-C1 deliberately contains no filesystem writer. Physical observation is
//! supplied by the retained provider, and R2-C2 will consume these decisions.

// R2-D Phase 4 Step 4.3 (settle item 7) narrowed one subtree-wide
// `allow(dead_code)` in `checked_artifact/mod.rs` down to the three children
// that still carry a frozen surface. `scratch` needs none: every item it
// exports has a production consumer today.
#[allow(
    dead_code,
    reason = "the sealed catalog owner is complete but unactivated: `recover_or_create` gains \
              its first production caller in R2-E, behind the Phase 4.3 coexistence criterion \
              (plan §5 item 2)"
)]
mod bootstrap;
#[allow(
    dead_code,
    reason = "the pure recovery classifier is consumed only by the owner above and by its own \
              suites until that activation"
)]
mod classifier;
#[allow(
    dead_code,
    reason = "the catalog-root enumeration budgets are exercised by the owner above and by \
              interface tests; R2-E's consumer is their first production reader"
)]
mod enumeration;
mod scratch;

#[allow(
    unused_imports,
    reason = "R2-C2 freezes the sealed catalog owner before C3 consumes its retained result"
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
