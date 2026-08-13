//! Synthetic aggregate observations for interface tests.

use std::collections::VecDeque;

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum SyntheticCatalogRecordStateV1 {
    Missing,
    PartialExpectedPrefix,
    Exact,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum SyntheticCatalogDirectoryStateV1 {
    Missing,
    PartialExpectedContents,
    Exact,
    OwnedMissingRecord,
    SubstitutedName,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct SyntheticCatalogRecoveryLayoutV1 {
    scratch: SyntheticCatalogRecordStateV1,
    active: SyntheticCatalogRecordStateV1,
    staging: SyntheticCatalogDirectoryStateV1,
    final_directory: SyntheticCatalogDirectoryStateV1,
    retired: SyntheticCatalogRecordStateV1,
}

impl SyntheticCatalogRecoveryLayoutV1 {
    pub(in crate::checked_artifact) const fn new(
        scratch: SyntheticCatalogRecordStateV1,
        active: SyntheticCatalogRecordStateV1,
        staging: SyntheticCatalogDirectoryStateV1,
        final_directory: SyntheticCatalogDirectoryStateV1,
        retired: SyntheticCatalogRecordStateV1,
    ) -> Self {
        Self {
            scratch,
            active,
            staging,
            final_directory,
            retired,
        }
    }
}

#[derive(Default)]
struct SyntheticProbeStateV1 {
    observations: usize,
    writes: usize,
}

struct SyntheticCatalogRecoveryProviderV1 {
    observations: std::sync::Mutex<VecDeque<RawCatalogRecoveryObservationV1>>,
    probe: std::sync::Arc<std::sync::Mutex<SyntheticProbeStateV1>>,
}

impl RawCatalogRecoveryProviderV1 for SyntheticCatalogRecoveryProviderV1 {
    fn observe_all(
        &self,
        _expected: &CatalogBootstrapRecordV1,
        _budget: CatalogRecoveryReadBudgetV1,
    ) -> Result<RawCatalogRecoveryObservationV1, CheckedFsError> {
        self.probe.lock().unwrap().observations += 1;
        let mut observations = self.observations.lock().unwrap();
        let value = if observations.len() > 1 {
            observations.pop_front()
        } else {
            observations.front().cloned()
        };
        value.ok_or_else(|| CheckedFsError::ambiguous("catalog recovery", "no observation"))
    }

    fn write_staging_infrastructure_record(
        &self,
        _value: &InfrastructureRecordV1,
    ) -> Result<(), CheckedFsError> {
        self.probe.lock().unwrap().writes += 1;
        Ok(())
    }
}

#[derive(Clone)]
pub(in crate::checked_artifact) struct SyntheticCatalogRecoveryProbeV1(
    std::sync::Arc<std::sync::Mutex<SyntheticProbeStateV1>>,
);

impl SyntheticCatalogRecoveryProbeV1 {
    pub(in crate::checked_artifact) fn observations(&self) -> usize {
        self.0.lock().unwrap().observations
    }

    pub(in crate::checked_artifact) fn writes(&self) -> usize {
        self.0.lock().unwrap().writes
    }
}

pub(in crate::checked_artifact) fn synthetic_catalog_recovery_owner(
    expected: &CatalogBootstrapRecordV1,
    before: SyntheticCatalogRecoveryLayoutV1,
    after_write: Option<SyntheticCatalogRecoveryLayoutV1>,
    directory_identity: DurableObjectIdentityV1,
    identities: ObservedInfrastructureIdentitiesV1,
) -> (
    CatalogInfrastructureOwnerV1,
    SyntheticCatalogRecoveryProbeV1,
) {
    let mut observations = VecDeque::from([synthetic_observation(
        expected,
        before,
        directory_identity.clone(),
        identities.clone(),
    )]);
    if let Some(after) = after_write {
        observations.push_back(synthetic_observation(
            expected,
            after,
            directory_identity,
            identities,
        ));
    }
    let probe = std::sync::Arc::new(std::sync::Mutex::new(SyntheticProbeStateV1::default()));
    (
        CatalogInfrastructureOwnerV1::from_provider(SyntheticCatalogRecoveryProviderV1 {
            observations: std::sync::Mutex::new(observations),
            probe: probe.clone(),
        }),
        SyntheticCatalogRecoveryProbeV1(probe),
    )
}

fn synthetic_observation(
    expected: &CatalogBootstrapRecordV1,
    layout: SyntheticCatalogRecoveryLayoutV1,
    directory_identity: DurableObjectIdentityV1,
    identities: ObservedInfrastructureIdentitiesV1,
) -> RawCatalogRecoveryObservationV1 {
    RawCatalogRecoveryObservationV1 {
        scratch: synthetic_record(expected, layout.scratch),
        active: synthetic_record(expected, layout.active),
        staging: synthetic_directory(
            expected,
            CatalogPrivateNameV1::BootstrapStaging,
            layout.staging,
            directory_identity.clone(),
            identities.clone(),
        ),
        final_directory: synthetic_directory(
            expected,
            CatalogPrivateNameV1::Final,
            layout.final_directory,
            directory_identity,
            identities,
        ),
        retired: synthetic_record(expected, layout.retired),
    }
}

fn synthetic_record(
    expected: &CatalogBootstrapRecordV1,
    state: SyntheticCatalogRecordStateV1,
) -> CatalogRecordObservationV1 {
    match state {
        SyntheticCatalogRecordStateV1::Missing => CatalogRecordObservationV1::Missing,
        SyntheticCatalogRecordStateV1::PartialExpectedPrefix => {
            CatalogRecordObservationV1::PartialExpectedPrefix
        }
        SyntheticCatalogRecordStateV1::Exact => {
            CatalogRecordObservationV1::Exact(Box::new(expected.clone()))
        }
        SyntheticCatalogRecordStateV1::Other => CatalogRecordObservationV1::Other,
    }
}

fn synthetic_directory(
    expected: &CatalogBootstrapRecordV1,
    role: CatalogPrivateNameV1,
    state: SyntheticCatalogDirectoryStateV1,
    directory_identity: DurableObjectIdentityV1,
    identities: ObservedInfrastructureIdentitiesV1,
) -> RawCatalogDirectoryObservationV1 {
    match state {
        SyntheticCatalogDirectoryStateV1::Missing => RawCatalogDirectoryObservationV1::Missing,
        SyntheticCatalogDirectoryStateV1::PartialExpectedContents => {
            RawCatalogDirectoryObservationV1::PartialExpectedContents
        }
        SyntheticCatalogDirectoryStateV1::Other => RawCatalogDirectoryObservationV1::Other,
        candidate_state => {
            let observed_role =
                if candidate_state == SyntheticCatalogDirectoryStateV1::SubstitutedName {
                    match role {
                        CatalogPrivateNameV1::BootstrapStaging => CatalogPrivateNameV1::Final,
                        _ => CatalogPrivateNameV1::BootstrapStaging,
                    }
                } else {
                    role
                };
            let value = InfrastructureRecordV1::from_fields(
                identities.catalog_root_identity.clone(),
                identities.catalog_anchor_identity.clone(),
                identities.roaming_anchor_identity.clone(),
                identities.retired_root_identity.clone(),
                directory_identity.clone(),
                expected.record_id(),
                expected.bootstrap_ownership_token(),
                slot_component(InfrastructureSlotV1::ActionAdmissionActive),
                slot_component(InfrastructureSlotV1::ActionAdmissionScratch),
                slot_component(InfrastructureSlotV1::ActionAdmissionStaging),
            );
            RawCatalogDirectoryObservationV1::OwnedCandidate(Box::new(RawOwnedCatalogCandidateV1 {
                observed_leaf: AsciiComponent::parse(observed_role.leaf_bytes())
                    .expect("fixed name is valid"),
                marker_bootstrap_record_id: expected.record_id(),
                marker_ownership_token: *expected.bootstrap_ownership_token().as_bytes(),
                retained_parent_identity: expected.retained_parent_identity().clone(),
                retained_parent_path: expected.retained_parent_path().clone(),
                directory_identity,
                identities,
                stored_record: (candidate_state
                    != SyntheticCatalogDirectoryStateV1::OwnedMissingRecord)
                    .then_some(value),
            }))
        }
    }
}
