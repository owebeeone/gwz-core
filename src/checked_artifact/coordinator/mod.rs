//! Closed derivation boundary between merge-owned durable facts and R1.

mod identity;
mod schedule;

pub(in crate::checked_artifact) use identity::*;
#[allow(
    unused_imports,
    reason = "R2 freezes the private schedule facade before production consumers are converted"
)]
pub(in crate::checked_artifact) use schedule::*;
