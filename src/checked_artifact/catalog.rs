//! Pure first-catalog grammar and one-edge recovery classification.
//!
//! R2-C1 deliberately contains no filesystem writer. Physical observation is
//! supplied by the retained provider, and R2-C2 will consume these decisions.

mod bootstrap;
mod classifier;
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
