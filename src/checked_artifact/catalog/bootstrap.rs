//! Sealed physical owner for first-catalog recovery.

use crate::checked_artifact::bootstrap::{CatalogLeaseTargetWitnessV1, CatalogMutationLeaseV1};
use crate::checked_artifact::capability::{
    CatalogPermitV1, CatalogPreflightV1, CheckedFsError, preflight_catalog_target,
};
use crate::checked_artifact::protocol::CatalogBootstrapOwnershipTokenV1;
use crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1;

/// The only physical first-catalog owner.
pub(in crate::checked_artifact) struct CatalogOwnerV1;

/// A complete catalog retained under its target mutation lease.
pub(in crate::checked_artifact) struct OpaqueRetainedCatalogV1<'lease> {
    _permit: Box<CatalogPermitV1<'lease>>,
}

enum CatalogOwnerStepV1<'lease> {
    Retry(CatalogLeaseTargetWitnessV1<'lease>),
    Complete(OpaqueRetainedCatalogV1<'lease>),
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
                CatalogOwnerStepV1::Complete(catalog) => return Ok(catalog),
            }
        }
    }

    fn execute_one(
        witness: CatalogLeaseTargetWitnessV1<'_>,
    ) -> Result<CatalogOwnerStepV1<'_>, CheckedFsError> {
        match preflight_catalog_target(witness)? {
            CatalogPreflightV1::MissingGitPrivateParent(permit) => Ok(CatalogOwnerStepV1::Retry(
                permit.execute_create_and_retry()?,
            )),
            CatalogPreflightV1::Ready(permit) => {
                let classification = permit.classify_observed();
                match classification.decision() {
                    CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch => {
                        let token = match classification.expected_record() {
                            Some(expected) => expected.bootstrap_ownership_token(),
                            None => fresh_token()?,
                        };
                        Ok(CatalogOwnerStepV1::Retry(
                            permit.execute_write_or_rewrite_scratch(token)?,
                        ))
                    }
                    CatalogBootstrapRecoveryDecisionV1::PublishActive => {
                        Ok(CatalogOwnerStepV1::Retry(permit.execute_publish_active()?))
                    }
                    CatalogBootstrapRecoveryDecisionV1::Complete => {
                        Ok(CatalogOwnerStepV1::Complete(OpaqueRetainedCatalogV1 {
                            _permit: permit,
                        }))
                    }
                    CatalogBootstrapRecoveryDecisionV1::Ambiguous => {
                        Err(CheckedFsError::ambiguous(
                            "catalog bootstrap owner",
                            "aggregate catalog facts are ambiguous",
                        ))
                    }
                    _ => Err(CheckedFsError::ambiguous(
                        "catalog bootstrap owner",
                        "later R2-C2 recovery edge is not implemented",
                    )),
                }
            }
        }
    }
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
