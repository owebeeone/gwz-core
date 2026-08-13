//! Sealed physical-observation owner for first-catalog infrastructure.

use super::*;
use crate::checked_artifact::capability::{CanonicalPathIdentityV1, CheckedFsError};
use crate::checked_artifact::protocol::{
    CatalogBootstrapOwnershipTokenV1, CatalogBootstrapRecordV1, InfrastructureSlotV1,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ObservedInfrastructureIdentitiesV1 {
    catalog_root_identity: DurableObjectIdentityV1,
    catalog_anchor_identity: DurableObjectIdentityV1,
    roaming_anchor_identity: DurableObjectIdentityV1,
    retired_root_identity: DurableObjectIdentityV1,
}

impl ObservedInfrastructureIdentitiesV1 {
    #[cfg(test)]
    pub(in crate::checked_artifact) fn new(
        catalog_root_identity: DurableObjectIdentityV1,
        catalog_anchor_identity: DurableObjectIdentityV1,
        roaming_anchor_identity: DurableObjectIdentityV1,
        retired_root_identity: DurableObjectIdentityV1,
    ) -> Self {
        Self {
            catalog_root_identity,
            catalog_anchor_identity,
            roaming_anchor_identity,
            retired_root_identity,
        }
    }
}

struct RawCatalogInfrastructureObservationV1 {
    marker_bootstrap_record_id: [u8; 32],
    marker_ownership_token: [u8; 32],
    retained_parent_identity: DurableObjectIdentityV1,
    retained_parent_path: CanonicalPathIdentityV1,
    staging_name: AsciiComponent,
    staging_directory_identity: DurableObjectIdentityV1,
    identities: ObservedInfrastructureIdentitiesV1,
    stored_record: Option<InfrastructureRecordV1>,
}

trait RawCatalogInfrastructureProviderV1 {
    fn observe(
        &self,
        active: &CatalogBootstrapRecordV1,
    ) -> Result<RawCatalogInfrastructureObservationV1, CheckedFsError>;

    fn write_infrastructure_record(
        &self,
        value: &InfrastructureRecordV1,
    ) -> Result<(), CheckedFsError>;
}

pub(in crate::checked_artifact) struct CatalogInfrastructureOwnerV1 {
    provider: Box<dyn RawCatalogInfrastructureProviderV1>,
}

impl CatalogInfrastructureOwnerV1 {
    fn from_provider(provider: impl RawCatalogInfrastructureProviderV1 + 'static) -> Self {
        Self {
            provider: Box::new(provider),
        }
    }

    /// Creates or validates the infrastructure record only after the physical
    /// marker and every observed identity are bound to the durable active
    /// bootstrap record.
    pub(in crate::checked_artifact) fn recover_or_create(
        &self,
        active: &CatalogBootstrapRecordV1,
    ) -> Result<BoundCatalogInfrastructureObservationV1, CheckedFsError> {
        let observed = self.provider.observe(active)?;
        if observed.marker_bootstrap_record_id != active.record_id()
            || observed.marker_ownership_token != *active.bootstrap_ownership_token().as_bytes()
            || observed.retained_parent_identity != *active.retained_parent_identity()
            || observed.retained_parent_path != *active.retained_parent_path()
            || observed.staging_name != *active.staging_name()
        {
            return Err(CheckedFsError::ambiguous(
                "catalog infrastructure ownership marker",
                "marker is not rooted in the durable active bootstrap record",
            ));
        }

        let expected = InfrastructureRecordV1::from_fields(
            observed.identities.catalog_root_identity,
            observed.identities.catalog_anchor_identity,
            observed.identities.roaming_anchor_identity,
            observed.identities.retired_root_identity,
            observed.staging_directory_identity,
            active.record_id(),
            active.bootstrap_ownership_token(),
            slot_component(InfrastructureSlotV1::ActionAdmissionActive),
            slot_component(InfrastructureSlotV1::ActionAdmissionScratch),
            slot_component(InfrastructureSlotV1::ActionAdmissionStaging),
        );
        expected.validate_profiles().map_err(|_| {
            CheckedFsError::ambiguous(
                "catalog infrastructure",
                "observed identities do not share the active support profile",
            )
        })?;
        if expected.catalog_root_identity.support_profile() != active.support_profile() {
            return Err(CheckedFsError::ambiguous(
                "catalog infrastructure",
                "observed identities do not match the active bootstrap profile",
            ));
        }

        match observed.stored_record {
            Some(stored) if stored != expected => {
                return Err(CheckedFsError::ambiguous(
                    "catalog infrastructure record",
                    "stored record does not match physical identities and active ownership",
                ));
            }
            Some(_) => {}
            None => self.provider.write_infrastructure_record(&expected)?,
        }

        Ok(BoundCatalogInfrastructureObservationV1 {
            active_record_id: active.record_id(),
            active_ownership_token: active.bootstrap_ownership_token(),
            infrastructure: expected,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct BoundCatalogInfrastructureObservationV1 {
    active_record_id: [u8; 32],
    active_ownership_token: CatalogBootstrapOwnershipTokenV1,
    infrastructure: InfrastructureRecordV1,
}

impl BoundCatalogInfrastructureObservationV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &InfrastructureRecordV1 {
        &self.infrastructure
    }

    pub(in crate::checked_artifact) fn is_bound_to(
        &self,
        active: &CatalogBootstrapRecordV1,
    ) -> bool {
        self.active_record_id == active.record_id()
            && self.active_ownership_token == active.bootstrap_ownership_token()
            && self.infrastructure.catalog_bootstrap_record_id() == active.record_id()
            && self.infrastructure.bootstrap_ownership_token() == active.bootstrap_ownership_token()
    }
}

#[cfg(test)]
struct SyntheticCatalogInfrastructureProviderV1 {
    observation: RawCatalogInfrastructureObservationV1,
    writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(test)]
impl RawCatalogInfrastructureProviderV1 for SyntheticCatalogInfrastructureProviderV1 {
    fn observe(
        &self,
        _active: &CatalogBootstrapRecordV1,
    ) -> Result<RawCatalogInfrastructureObservationV1, CheckedFsError> {
        Ok(RawCatalogInfrastructureObservationV1 {
            marker_bootstrap_record_id: self.observation.marker_bootstrap_record_id,
            marker_ownership_token: self.observation.marker_ownership_token,
            retained_parent_identity: self.observation.retained_parent_identity.clone(),
            retained_parent_path: self.observation.retained_parent_path.clone(),
            staging_name: self.observation.staging_name.clone(),
            staging_directory_identity: self.observation.staging_directory_identity.clone(),
            identities: self.observation.identities.clone(),
            stored_record: self.observation.stored_record.clone(),
        })
    }

    fn write_infrastructure_record(
        &self,
        _value: &InfrastructureRecordV1,
    ) -> Result<(), CheckedFsError> {
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(in crate::checked_artifact) struct SyntheticCatalogInfrastructureProbeV1(
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
);

#[cfg(test)]
impl SyntheticCatalogInfrastructureProbeV1 {
    pub(in crate::checked_artifact) fn writes(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
enum SyntheticStoredRecordV1 {
    Matching,
    Missing,
    Mismatched,
}

#[cfg(test)]
fn synthetic_owner(
    active: &CatalogBootstrapRecordV1,
    marker_bootstrap_record_id: [u8; 32],
    marker_ownership_token: [u8; 32],
    staging_directory_identity: DurableObjectIdentityV1,
    identities: ObservedInfrastructureIdentitiesV1,
    stored: SyntheticStoredRecordV1,
) -> (
    CatalogInfrastructureOwnerV1,
    SyntheticCatalogInfrastructureProbeV1,
) {
    let token = CatalogBootstrapOwnershipTokenV1::try_from_random_bytes(marker_ownership_token)
        .unwrap_or_else(|_| {
            CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([1; 32]).unwrap()
        });
    let make_record = |staging_directory_identity| {
        InfrastructureRecordV1::from_fields(
            identities.catalog_root_identity.clone(),
            identities.catalog_anchor_identity.clone(),
            identities.roaming_anchor_identity.clone(),
            identities.retired_root_identity.clone(),
            staging_directory_identity,
            marker_bootstrap_record_id,
            token,
            slot_component(InfrastructureSlotV1::ActionAdmissionActive),
            slot_component(InfrastructureSlotV1::ActionAdmissionScratch),
            slot_component(InfrastructureSlotV1::ActionAdmissionStaging),
        )
    };
    let stored_record = match stored {
        SyntheticStoredRecordV1::Matching => Some(make_record(staging_directory_identity.clone())),
        SyntheticStoredRecordV1::Missing => None,
        SyntheticStoredRecordV1::Mismatched => {
            Some(make_record(identities.catalog_anchor_identity.clone()))
        }
    };
    let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    (
        CatalogInfrastructureOwnerV1::from_provider(SyntheticCatalogInfrastructureProviderV1 {
            observation: RawCatalogInfrastructureObservationV1 {
                marker_bootstrap_record_id,
                marker_ownership_token,
                retained_parent_identity: active.retained_parent_identity().clone(),
                retained_parent_path: active.retained_parent_path().clone(),
                staging_name: active.staging_name().clone(),
                staging_directory_identity,
                identities,
                stored_record,
            },
            writes: writes.clone(),
        }),
        SyntheticCatalogInfrastructureProbeV1(writes),
    )
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_catalog_infrastructure_owner(
    active: &CatalogBootstrapRecordV1,
    marker_bootstrap_record_id: [u8; 32],
    marker_ownership_token: [u8; 32],
    staging_directory_identity: DurableObjectIdentityV1,
    identities: ObservedInfrastructureIdentitiesV1,
) -> CatalogInfrastructureOwnerV1 {
    synthetic_owner(
        active,
        marker_bootstrap_record_id,
        marker_ownership_token,
        staging_directory_identity,
        identities,
        SyntheticStoredRecordV1::Matching,
    )
    .0
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_catalog_infrastructure_owner_missing_record(
    active: &CatalogBootstrapRecordV1,
    staging_directory_identity: DurableObjectIdentityV1,
    identities: ObservedInfrastructureIdentitiesV1,
) -> (
    CatalogInfrastructureOwnerV1,
    SyntheticCatalogInfrastructureProbeV1,
) {
    synthetic_owner(
        active,
        active.record_id(),
        *active.bootstrap_ownership_token().as_bytes(),
        staging_directory_identity,
        identities,
        SyntheticStoredRecordV1::Missing,
    )
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_catalog_infrastructure_owner_mismatched_record(
    active: &CatalogBootstrapRecordV1,
    staging_directory_identity: DurableObjectIdentityV1,
    identities: ObservedInfrastructureIdentitiesV1,
) -> CatalogInfrastructureOwnerV1 {
    synthetic_owner(
        active,
        active.record_id(),
        *active.bootstrap_ownership_token().as_bytes(),
        staging_directory_identity,
        identities,
        SyntheticStoredRecordV1::Mismatched,
    )
    .0
}

#[allow(
    dead_code,
    reason = "always-compiled proof that a platform provider fits the sealed catalog owner seam"
)]
mod production_provider_compile {
    use super::*;

    struct PlatformCatalogProvider;

    impl RawCatalogInfrastructureProviderV1 for PlatformCatalogProvider {
        fn observe(
            &self,
            _active: &CatalogBootstrapRecordV1,
        ) -> Result<RawCatalogInfrastructureObservationV1, CheckedFsError> {
            Err(CheckedFsError::ambiguous(
                "compile-only catalog provider",
                "not executed",
            ))
        }

        fn write_infrastructure_record(
            &self,
            _value: &InfrastructureRecordV1,
        ) -> Result<(), CheckedFsError> {
            Err(CheckedFsError::ambiguous(
                "compile-only catalog provider",
                "not executed",
            ))
        }
    }

    fn production_owner() -> CatalogInfrastructureOwnerV1 {
        CatalogInfrastructureOwnerV1::from_provider(PlatformCatalogProvider)
    }

    fn compile_recovery_call(
        active: &CatalogBootstrapRecordV1,
    ) -> Result<BoundCatalogInfrastructureObservationV1, CheckedFsError> {
        production_owner().recover_or_create(active)
    }
}
