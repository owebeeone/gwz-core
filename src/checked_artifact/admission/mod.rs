//! Physical action admission, reservation, and handoff owner.
//!
//! This is the cohesive owner named by `GwzM5-8R4bR2ConsumerCheckpoint.md` §11
//! and by the R2-C amendment's §7 owner list. Its seam is frozen by
//! `dev-docs/GwzM5-8R2DInterfaceFreeze.md` (R2-D Step 0.1); the physical driver
//! itself lands in R2-D Phase 1 (formally R2-C's tail, adopted plan §9.1).
//!
//! Two properties of this seam are structural rather than advisory:
//!
//! * the owner is constructible only from an `OpaqueRetainedCatalogV1`, which
//!   is itself obtainable only from the sealed catalog owner's completed
//!   recovery, so admission can never run against a caller-supplied root,
//!   lease bytes, raw role rows, or a synthetic observation; and
//! * the owner hands back only `AdmittedActionV1`, never a raw handle or a
//!   mutation capability, per amendment §7 ("without returning raw handles or
//!   mutation capability to callers").
//!
//! Phase 1.2 fills `resume_or_admit` with the nine-step durable sequence in
//! `GwzM5-8R4bR2ConsumerCheckpoint.md` §7 and routes every namespace edge
//! through the sealed source-associated publication family (amendment §4.1,
//! §8.13). The sequence itself lives in the private `driver` submodule, so the
//! frozen seam above stays the module's whole exposed surface.

mod driver;

use super::capability::CheckedFsError;
use super::catalog::OpaqueRetainedCatalogV1;
use super::protocol::{ActionCapacityReservationV1, AdmittedActionV1};

/// Sole owner of the physical admission, reservation, and handoff sequence.
pub(in crate::checked_artifact) struct ActionAdmissionOwnerV1<'lease> {
    catalog: OpaqueRetainedCatalogV1<'lease>,
}

impl<'lease> ActionAdmissionOwnerV1<'lease> {
    /// The only constructor: admission consumes the opaque retained catalog
    /// and nothing else.
    pub(in crate::checked_artifact) const fn from_retained_catalog(
        catalog: OpaqueRetainedCatalogV1<'lease>,
    ) -> Self {
        Self { catalog }
    }

    /// Resumes an exact existing action, or executes the durable
    /// `Idle -> Preparing -> staging -> resident reservation -> no-replace
    /// publish -> Preparing -> Idle -> reobserve` sequence, returning the
    /// opaque handoff only from idle + missing staging + exact final
    /// reservation with no extra children.
    pub(in crate::checked_artifact) fn resume_or_admit(
        &mut self,
        expected: &ActionCapacityReservationV1,
    ) -> Result<AdmittedActionV1, CheckedFsError> {
        driver::resume_or_admit(&self.catalog, expected)
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_fault_matrix;
