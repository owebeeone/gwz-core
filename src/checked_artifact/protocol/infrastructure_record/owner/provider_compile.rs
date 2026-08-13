//! Always-compiled proof for the sealed aggregate provider seam.

use super::*;

struct PlatformCatalogProvider;

impl RawCatalogRecoveryProviderV1 for PlatformCatalogProvider {
    fn observe_all(
        &self,
        _expected: &CatalogBootstrapRecordV1,
        _budget: CatalogRecoveryReadBudgetV1,
    ) -> Result<RawCatalogRecoveryObservationV1, CheckedFsError> {
        Err(CheckedFsError::ambiguous(
            "compile-only catalog provider",
            "not executed",
        ))
    }

    fn write_staging_infrastructure_record(
        &self,
        _value: &InfrastructureRecordV1,
    ) -> Result<(), CheckedFsError> {
        Err(CheckedFsError::ambiguous(
            "compile-only catalog provider",
            "not executed",
        ))
    }
}

#[allow(dead_code, reason = "compile-only owner construction and call proof")]
fn compile_recovery_call(
    expected: &CatalogBootstrapRecordV1,
) -> Result<CatalogBootstrapRecoveryObservationV1, CheckedFsError> {
    CatalogInfrastructureOwnerV1::from_provider(PlatformCatalogProvider).recover(expected)
}
