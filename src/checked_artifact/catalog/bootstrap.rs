//! Sealed physical owner for first-catalog recovery.

use crate::checked_artifact::bootstrap::{CatalogLeaseTargetWitnessV1, CatalogMutationLeaseV1};
use crate::checked_artifact::capability::{
    AsciiComponent, CatalogPreflightV1, CheckedFsError, CompletedCatalogPermitV1,
    ManagedPrefixObservationV1, RetainedActionNamespaceV1, RetainedManagedParentV1,
    RoamingAnchorHomeWitnessV1, preflight_catalog_target,
};
use crate::checked_artifact::protocol::CatalogBootstrapOwnershipTokenV1;
use crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1;
use crate::checked_artifact::protocol::{
    ActionAdmissionEdgeV1, ActionAdmissionObservationV1, ActionCapacityReservationV1,
    AdmittedActionV1, RecordDigestV1,
};

/// The only physical first-catalog owner.
pub(in crate::checked_artifact) struct CatalogOwnerV1;

/// Unforgeable one-edge authority minted only by [`CatalogOwnerV1`].
pub(in crate::checked_artifact) struct CatalogOwnerEdgeV1 {
    kind: CatalogOwnerEdgeKindV1,
}

enum CatalogOwnerEdgeKindV1 {
    CreatePrivateParent,
    WriteOrRewriteScratch(CatalogBootstrapOwnershipTokenV1),
    PublishActive,
    PrepareOrRewriteStaging,
    PublishFinal,
    RetireActive,
    Complete,
}

impl CatalogOwnerEdgeV1 {
    fn create_private_parent() -> Self {
        Self {
            kind: CatalogOwnerEdgeKindV1::CreatePrivateParent,
        }
    }

    fn write_scratch(token: CatalogBootstrapOwnershipTokenV1) -> Self {
        Self {
            kind: CatalogOwnerEdgeKindV1::WriteOrRewriteScratch(token),
        }
    }

    fn publish_active() -> Self {
        Self {
            kind: CatalogOwnerEdgeKindV1::PublishActive,
        }
    }

    fn prepare_staging() -> Self {
        Self {
            kind: CatalogOwnerEdgeKindV1::PrepareOrRewriteStaging,
        }
    }

    fn publish_final() -> Self {
        Self {
            kind: CatalogOwnerEdgeKindV1::PublishFinal,
        }
    }

    fn retire_active() -> Self {
        Self {
            kind: CatalogOwnerEdgeKindV1::RetireActive,
        }
    }

    fn complete() -> Self {
        Self {
            kind: CatalogOwnerEdgeKindV1::Complete,
        }
    }

    pub(in crate::checked_artifact) fn require_create_private_parent(
        self,
    ) -> Result<(), CheckedFsError> {
        match self.kind {
            CatalogOwnerEdgeKindV1::CreatePrivateParent => Ok(()),
            _ => Err(edge_mismatch()),
        }
    }

    pub(in crate::checked_artifact) fn require_scratch_token(
        self,
    ) -> Result<CatalogBootstrapOwnershipTokenV1, CheckedFsError> {
        match self.kind {
            CatalogOwnerEdgeKindV1::WriteOrRewriteScratch(token) => Ok(token),
            _ => Err(edge_mismatch()),
        }
    }

    pub(in crate::checked_artifact) fn require_publish_active(self) -> Result<(), CheckedFsError> {
        match self.kind {
            CatalogOwnerEdgeKindV1::PublishActive => Ok(()),
            _ => Err(edge_mismatch()),
        }
    }

    pub(in crate::checked_artifact) fn require_prepare_or_rewrite_staging(
        self,
    ) -> Result<(), CheckedFsError> {
        match self.kind {
            CatalogOwnerEdgeKindV1::PrepareOrRewriteStaging => Ok(()),
            _ => Err(edge_mismatch()),
        }
    }

    pub(in crate::checked_artifact) fn require_publish_final(self) -> Result<(), CheckedFsError> {
        match self.kind {
            CatalogOwnerEdgeKindV1::PublishFinal => Ok(()),
            _ => Err(edge_mismatch()),
        }
    }

    pub(in crate::checked_artifact) fn require_retire_active(self) -> Result<(), CheckedFsError> {
        match self.kind {
            CatalogOwnerEdgeKindV1::RetireActive => Ok(()),
            _ => Err(edge_mismatch()),
        }
    }

    pub(in crate::checked_artifact) fn require_complete(self) -> Result<(), CheckedFsError> {
        match self.kind {
            CatalogOwnerEdgeKindV1::Complete => Ok(()),
            _ => Err(edge_mismatch()),
        }
    }
}

/// A complete catalog retained under its target mutation lease.
pub(in crate::checked_artifact) struct OpaqueRetainedCatalogV1<'lease> {
    permit: CompletedCatalogPermitV1<'lease>,
}

impl OpaqueRetainedCatalogV1<'_> {
    #[cfg(test)]
    fn revalidate_for_test(&self) -> Result<(), CheckedFsError> {
        self.permit.revalidate()
    }

    /// The R2-D Phase 1 admission observation, forwarded through the retained
    /// completion permit. The opaque catalog stays opaque: the admission owner
    /// receives typed observations and never the permit, a root, or a handle
    /// (`GwzM5-8R2DInterfaceFreeze.md` §3.1).
    pub(in crate::checked_artifact) fn observe_admission(
        &self,
        expected: &ActionCapacityReservationV1,
    ) -> Result<ActionAdmissionObservationV1, CheckedFsError> {
        self.permit.observe_admission(expected)
    }

    /// One bounded durable admission edge, forwarded through the same permit.
    pub(in crate::checked_artifact) fn execute_admission_edge(
        &self,
        edge: ActionAdmissionEdgeV1<'_>,
        expected: &ActionCapacityReservationV1,
    ) -> Result<(), CheckedFsError> {
        self.permit.execute_admission_edge(edge, expected)
    }

    /// R2-D Phase 2 Step 2.2's namespace capability, forwarded through the same
    /// permit. The catalog stays opaque: the namespace owner receives a
    /// retained, reservation-bound action-directory capability and never the
    /// permit, a root, or a handle (`GwzM5-8R2DInterfaceFreeze.md` §3.1).
    pub(in crate::checked_artifact) fn retain_action_namespace(
        &self,
        admitted: &AdmittedActionV1,
    ) -> Result<RetainedActionNamespaceV1, CheckedFsError> {
        self.permit.retain_action_namespace(admitted)
    }

    /// R2-E E3.1's terminal retirement, forwarded through the same permit under
    /// the same rule: the terminal owner receives a typed result and never the
    /// permit, a root, or a handle (`GwzM5-8R2DInterfaceFreeze.md` §3.1).
    pub(in crate::checked_artifact) fn retire_admitted_action(
        &self,
        admitted: &AdmittedActionV1,
    ) -> Result<(), CheckedFsError> {
        self.permit.retire_admitted_action(admitted)
    }

    /// R2-E Phase E2's roaming-anchor-home observation (O6), forwarded through
    /// the same permit under the same rule. The catalog stays opaque: the
    /// barrier owner receives a typed witness it cannot construct and never the
    /// permit, a root, or a handle (`GwzM5-8R2DInterfaceFreeze.md` §3.1).
    pub(in crate::checked_artifact) fn observe_roaming_anchor_home(
        &self,
    ) -> Result<RoamingAnchorHomeWitnessV1, CheckedFsError> {
        self.permit.observe_roaming_anchor_home()
    }

    /// R2-D Phase 3 Step 3.1's managed-parent prefix observation, forwarded
    /// through the same permit. The catalog stays opaque: the managed-parent
    /// provider receives typed durable facts and never the permit, a root, or a
    /// handle (`GwzM5-8R2DInterfaceFreeze.md` §3.1).
    pub(in crate::checked_artifact) fn observe_managed_prefix(
        &self,
        components: &[AsciiComponent],
    ) -> Result<ManagedPrefixObservationV1, CheckedFsError> {
        self.permit.observe_managed_prefix(components)
    }

    /// R2-D Phase 3 Step 3.1's retained managed parent, forwarded through the
    /// same permit under the same rule.
    pub(in crate::checked_artifact) fn retain_managed_prefix(
        &self,
        components: &[AsciiComponent],
        depth: usize,
        reservation: RecordDigestV1,
    ) -> Result<RetainedManagedParentV1, CheckedFsError> {
        self.permit
            .retain_managed_prefix(components, depth, reservation)
    }

    /// R2-D Phase 3 Step 3.1's provider instance binding, forwarded through the
    /// same permit.
    pub(in crate::checked_artifact) fn managed_provider_instance(
        &self,
    ) -> Result<[u8; 32], CheckedFsError> {
        self.permit.managed_provider_instance()
    }
}

enum CatalogOwnerStepV1<'lease> {
    Retry(CatalogLeaseTargetWitnessV1<'lease>),
    Complete(Box<OpaqueRetainedCatalogV1<'lease>>),
}

/// Recovers or creates one catalog using only a target-bound lease.
pub(in crate::checked_artifact) fn recover_or_create(
    lease: CatalogMutationLeaseV1<'_>,
) -> Result<OpaqueRetainedCatalogV1<'_>, CheckedFsError> {
    CatalogOwnerV1::recover_or_create(lease)
}

impl CatalogOwnerV1 {
    fn recover_or_create(
        lease: CatalogMutationLeaseV1<'_>,
    ) -> Result<OpaqueRetainedCatalogV1<'_>, CheckedFsError> {
        let mut witness = lease.begin_preflight()?;
        loop {
            match Self::execute_one(witness)? {
                CatalogOwnerStepV1::Retry(next) => witness = next,
                CatalogOwnerStepV1::Complete(catalog) => return Ok(*catalog),
            }
        }
    }

    fn execute_one(
        witness: CatalogLeaseTargetWitnessV1<'_>,
    ) -> Result<CatalogOwnerStepV1<'_>, CheckedFsError> {
        match preflight_catalog_target(witness)? {
            CatalogPreflightV1::MissingGitPrivateParent(permit) => Ok(CatalogOwnerStepV1::Retry(
                permit
                    .execute_owner_create_and_retry(CatalogOwnerEdgeV1::create_private_parent())?,
            )),
            CatalogPreflightV1::Ready(permit) => {
                let classification = permit.classify_observed();
                match classification.decision() {
                    CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch => {
                        let token = match classification.expected_record() {
                            Some(expected) => expected.bootstrap_ownership_token(),
                            None => fresh_token()?,
                        };
                        Ok(CatalogOwnerStepV1::Retry(permit.execute_owner_scratch(
                            CatalogOwnerEdgeV1::write_scratch(token),
                        )?))
                    }
                    CatalogBootstrapRecoveryDecisionV1::PublishActive => {
                        Ok(CatalogOwnerStepV1::Retry(
                            permit.execute_owner_publish_active(
                                CatalogOwnerEdgeV1::publish_active(),
                            )?,
                        ))
                    }
                    CatalogBootstrapRecoveryDecisionV1::PrepareOrRewriteStaging => {
                        Ok(CatalogOwnerStepV1::Retry(
                            permit.execute_owner_prepare_or_rewrite_staging(
                                CatalogOwnerEdgeV1::prepare_staging(),
                            )?,
                        ))
                    }
                    CatalogBootstrapRecoveryDecisionV1::PublishFinal => {
                        Ok(CatalogOwnerStepV1::Retry(
                            permit
                                .execute_owner_publish_final(CatalogOwnerEdgeV1::publish_final())?,
                        ))
                    }
                    CatalogBootstrapRecoveryDecisionV1::RetireActive => {
                        Ok(CatalogOwnerStepV1::Retry(
                            permit
                                .execute_owner_retire_active(CatalogOwnerEdgeV1::retire_active())?,
                        ))
                    }
                    CatalogBootstrapRecoveryDecisionV1::Complete => Ok(
                        CatalogOwnerStepV1::Complete(Box::new(OpaqueRetainedCatalogV1 {
                            permit: permit.execute_owner_complete(CatalogOwnerEdgeV1::complete())?,
                        })),
                    ),
                    CatalogBootstrapRecoveryDecisionV1::Ambiguous => {
                        Err(CheckedFsError::ambiguous(
                            "catalog bootstrap owner",
                            "aggregate catalog facts are ambiguous",
                        ))
                    }
                }
            }
        }
    }
}

fn edge_mismatch() -> CheckedFsError {
    CheckedFsError::ambiguous(
        "catalog owner edge",
        "owner authority does not match the requested physical transition",
    )
}

fn fresh_token() -> Result<CatalogBootstrapOwnershipTokenV1, CheckedFsError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|source| {
        CheckedFsError::io(
            "generate catalog bootstrap ownership token",
            std::io::Error::other(source.to_string()),
        )
    })?;
    CatalogBootstrapOwnershipTokenV1::try_from_random_bytes(bytes).map_err(|_| {
        CheckedFsError::ambiguous(
            "catalog bootstrap ownership token",
            "cryptographic random source returned the reserved zero token",
        )
    })
}

#[cfg(test)]
mod tests;
