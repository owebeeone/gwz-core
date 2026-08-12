//! Pure, bounded v1 catalog protocol contracts.

mod admission;
mod barrier;
mod bounds;
mod cleanup;
mod codec;
#[allow(clippy::redundant_closure, clippy::needless_question_mark)]
#[rustfmt::skip]
pub(in crate::checked_artifact) mod generated;
mod schedule;
mod slots;

#[allow(
    unused_imports,
    reason = "R1 exports private interfaces before R2 consumers are converted"
)]
pub(in crate::checked_artifact) use admission::*;
#[allow(
    unused_imports,
    reason = "R1 exports private interfaces before R2 consumers are converted"
)]
pub(in crate::checked_artifact) use barrier::*;
#[allow(
    unused_imports,
    reason = "R1 exports private interfaces before R2 consumers are converted"
)]
pub(in crate::checked_artifact) use bounds::*;
#[allow(
    unused_imports,
    reason = "R1 exports private interfaces before R2 consumers are converted"
)]
pub(in crate::checked_artifact) use cleanup::*;
#[allow(
    unused_imports,
    reason = "R1 exports private interfaces before R2 consumers are converted"
)]
pub(in crate::checked_artifact) use codec::*;
#[allow(
    unused_imports,
    reason = "R1 exports private interfaces before R2 consumers are converted"
)]
pub(in crate::checked_artifact) use schedule::*;
#[allow(
    unused_imports,
    reason = "R1 exports private interfaces before R2 consumers are converted"
)]
pub(in crate::checked_artifact) use slots::*;

#[allow(
    unused_imports,
    reason = "R1 exports private interfaces before R2 consumers are converted"
)]
pub(in crate::checked_artifact) use super::capability::CanonicalPathIdentityV1;
