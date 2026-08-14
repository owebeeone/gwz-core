//! Aggregate, role-bound first-catalog recovery owner.

use super::*;
use crate::checked_artifact::capability::{CheckedFsError, DurablePathV1};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{
    CatalogBootstrapRecordV1, CatalogBootstrapRecoveryDecisionV1, CatalogRecordObservationV1,
    InfrastructureSlotV1, ProtocolRecordKindV1,
};

mod provider_compile;

#[cfg(test)]
mod test_support;

#[cfg(test)]
pub(in crate::checked_artifact) use test_support::*;

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

#[derive(Clone)]
struct RawOwnedCatalogCandidateV1 {
    observed_leaf: AsciiComponent,
    marker_bootstrap_record_id: [u8; 32],
    marker_ownership_token: [u8; 32],
    retained_parent_identity: DurableObjectIdentityV1,
    retained_parent_path: DurablePathV1,
    directory_identity: DurableObjectIdentityV1,
    identities: ObservedInfrastructureIdentitiesV1,
    stored_record: Option<InfrastructureRecordV1>,
}

#[derive(Clone)]
enum RawCatalogDirectoryObservationV1 {
    Missing,
    PartialExpectedContents,
    OwnedCandidate(Box<RawOwnedCatalogCandidateV1>),
    Other,
}

#[derive(Clone)]
struct RawCatalogRecoveryObservationV1 {
    scratch: CatalogRecordObservationV1,
    active: CatalogRecordObservationV1,
    staging: RawCatalogDirectoryObservationV1,
    final_directory: RawCatalogDirectoryObservationV1,
    retired: CatalogRecordObservationV1,
}

#[derive(Clone, Copy)]
struct CatalogRecoveryReadBudgetV1 {
    infrastructure_record_bytes: usize,
}

impl CatalogRecoveryReadBudgetV1 {
    const fn checked_v1() -> Self {
        Self {
            infrastructure_record_bytes: ProtocolRecordKindV1::Infrastructure.max_bytes(),
        }
    }
}

trait RawCatalogRecoveryProviderV1 {
    fn observe_all(
        &self,
        expected: &CatalogBootstrapRecordV1,
        budget: CatalogRecoveryReadBudgetV1,
    ) -> Result<RawCatalogRecoveryObservationV1, CheckedFsError>;

    fn write_staging_infrastructure_record(
        &self,
        value: &InfrastructureRecordV1,
    ) -> Result<(), CheckedFsError>;
}

pub(in crate::checked_artifact) struct CatalogInfrastructureOwnerV1 {
    provider: Box<dyn RawCatalogRecoveryProviderV1>,
}

impl CatalogInfrastructureOwnerV1 {
    fn from_provider(provider: impl RawCatalogRecoveryProviderV1 + 'static) -> Self {
        Self {
            provider: Box::new(provider),
        }
    }

    /// Observes and classifies every fixed catalog role before mutation. A
    /// permitted staging-record write is followed by a fresh aggregate
    /// observation; the write itself never fabricates exact evidence.
    pub(in crate::checked_artifact) fn recover(
        &self,
        expected: &CatalogBootstrapRecordV1,
    ) -> Result<CatalogBootstrapRecoveryObservationV1, CheckedFsError> {
        let validated = self.observe_and_validate(expected)?;
        let write = validated.staging.missing_record().cloned();
        let result = classify_catalog_bootstrap_recovery(expected, validated);

        if result.decision() == CatalogBootstrapRecoveryDecisionV1::PrepareOrRewriteStaging
            && let Some(value) = write
        {
            self.provider.write_staging_infrastructure_record(&value)?;
            return Ok(classify_catalog_bootstrap_recovery(
                expected,
                self.observe_and_validate(expected)?,
            ));
        }
        Ok(result)
    }

    fn observe_and_validate(
        &self,
        expected: &CatalogBootstrapRecordV1,
    ) -> Result<ValidatedCatalogRecoveryObservationV1, CheckedFsError> {
        let budget = CatalogRecoveryReadBudgetV1::checked_v1();
        let observed = self.provider.observe_all(expected, budget)?;
        Ok(ValidatedCatalogRecoveryObservationV1 {
            scratch: observed.scratch,
            active: observed.active,
            staging: validate_staging(expected, observed.staging, budget),
            final_directory: validate_final(expected, observed.final_directory, budget),
            retired: observed.retired,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ExactStagingInfrastructureV1(Box<InfrastructureRecordV1>);

impl ExactStagingInfrastructureV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &InfrastructureRecordV1 {
        &self.0
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ExactFinalInfrastructureV1(Box<InfrastructureRecordV1>);

impl ExactFinalInfrastructureV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &InfrastructureRecordV1 {
        &self.0
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogBootstrapRecoveryObservationV1 {
    WriteOrRewriteScratch,
    PublishActive,
    PrepareOrRewriteStaging,
    PublishFinal(ExactStagingInfrastructureV1),
    RetireActive(ExactFinalInfrastructureV1),
    Complete(ExactFinalInfrastructureV1),
    Ambiguous,
}

impl CatalogBootstrapRecoveryObservationV1 {
    pub(in crate::checked_artifact) const fn decision(&self) -> CatalogBootstrapRecoveryDecisionV1 {
        match self {
            Self::WriteOrRewriteScratch => {
                CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch
            }
            Self::PublishActive => CatalogBootstrapRecoveryDecisionV1::PublishActive,
            Self::PrepareOrRewriteStaging => {
                CatalogBootstrapRecoveryDecisionV1::PrepareOrRewriteStaging
            }
            Self::PublishFinal(_) => CatalogBootstrapRecoveryDecisionV1::PublishFinal,
            Self::RetireActive(_) => CatalogBootstrapRecoveryDecisionV1::RetireActive,
            Self::Complete(_) => CatalogBootstrapRecoveryDecisionV1::Complete,
            Self::Ambiguous => CatalogBootstrapRecoveryDecisionV1::Ambiguous,
        }
    }
}

enum ValidatedStagingObservationV1 {
    Missing,
    PartialExpectedContents,
    OwnedMissingRecord(Box<InfrastructureRecordV1>),
    Exact(ExactStagingInfrastructureV1),
    Other,
}

impl ValidatedStagingObservationV1 {
    fn missing_record(&self) -> Option<&InfrastructureRecordV1> {
        match self {
            Self::OwnedMissingRecord(value) => Some(value),
            _ => None,
        }
    }
}

enum ValidatedFinalObservationV1 {
    Missing,
    PartialExpectedContents,
    Exact(ExactFinalInfrastructureV1),
    Other,
}

struct ValidatedCatalogRecoveryObservationV1 {
    scratch: CatalogRecordObservationV1,
    active: CatalogRecordObservationV1,
    staging: ValidatedStagingObservationV1,
    final_directory: ValidatedFinalObservationV1,
    retired: CatalogRecordObservationV1,
}

fn validate_staging(
    expected: &CatalogBootstrapRecordV1,
    observed: RawCatalogDirectoryObservationV1,
    budget: CatalogRecoveryReadBudgetV1,
) -> ValidatedStagingObservationV1 {
    match observed {
        RawCatalogDirectoryObservationV1::Missing => ValidatedStagingObservationV1::Missing,
        RawCatalogDirectoryObservationV1::PartialExpectedContents => {
            ValidatedStagingObservationV1::PartialExpectedContents
        }
        RawCatalogDirectoryObservationV1::OwnedCandidate(candidate) => {
            let Some((value, stored)) = validate_candidate(
                expected,
                CatalogPrivateNameV1::BootstrapStaging,
                *candidate,
                budget,
            ) else {
                return ValidatedStagingObservationV1::Other;
            };
            match stored {
                Some(stored) if stored == value => ValidatedStagingObservationV1::Exact(
                    ExactStagingInfrastructureV1(Box::new(value)),
                ),
                Some(_) => ValidatedStagingObservationV1::Other,
                None => ValidatedStagingObservationV1::OwnedMissingRecord(Box::new(value)),
            }
        }
        RawCatalogDirectoryObservationV1::Other => ValidatedStagingObservationV1::Other,
    }
}

fn validate_final(
    expected: &CatalogBootstrapRecordV1,
    observed: RawCatalogDirectoryObservationV1,
    budget: CatalogRecoveryReadBudgetV1,
) -> ValidatedFinalObservationV1 {
    match observed {
        RawCatalogDirectoryObservationV1::Missing => ValidatedFinalObservationV1::Missing,
        RawCatalogDirectoryObservationV1::PartialExpectedContents => {
            ValidatedFinalObservationV1::PartialExpectedContents
        }
        RawCatalogDirectoryObservationV1::OwnedCandidate(candidate) => {
            let Some((value, Some(stored))) =
                validate_candidate(expected, CatalogPrivateNameV1::Final, *candidate, budget)
            else {
                return ValidatedFinalObservationV1::Other;
            };
            if stored == value {
                ValidatedFinalObservationV1::Exact(ExactFinalInfrastructureV1(Box::new(value)))
            } else {
                ValidatedFinalObservationV1::Other
            }
        }
        RawCatalogDirectoryObservationV1::Other => ValidatedFinalObservationV1::Other,
    }
}

fn validate_candidate(
    expected: &CatalogBootstrapRecordV1,
    role: CatalogPrivateNameV1,
    candidate: RawOwnedCatalogCandidateV1,
    budget: CatalogRecoveryReadBudgetV1,
) -> Option<(InfrastructureRecordV1, Option<InfrastructureRecordV1>)> {
    if candidate.observed_leaf.as_bytes() != role.leaf_bytes()
        || candidate.marker_bootstrap_record_id != expected.record_id()
        || candidate.marker_ownership_token != *expected.bootstrap_ownership_token().as_bytes()
        || candidate.retained_parent_identity != *expected.retained_parent_identity()
        || candidate.retained_parent_path != *expected.retained_parent_path()
    {
        return None;
    }
    let value = InfrastructureRecordV1::from_fields(
        candidate.identities.catalog_root_identity,
        candidate.identities.catalog_anchor_identity,
        candidate.identities.roaming_anchor_identity,
        candidate.identities.retired_root_identity,
        candidate.directory_identity,
        expected.record_id(),
        expected.bootstrap_ownership_token(),
        slot_component(InfrastructureSlotV1::ActionAdmissionActive),
        slot_component(InfrastructureSlotV1::ActionAdmissionScratch),
        slot_component(InfrastructureSlotV1::ActionAdmissionStaging),
    );
    if value.validate_profiles().is_err()
        || value.catalog_root_identity.support_profile() != expected.support_profile()
        || value.encode_canonical().len() > budget.infrastructure_record_bytes
    {
        return None;
    }
    Some((value, candidate.stored_record))
}

fn classify_catalog_bootstrap_recovery(
    expected: &CatalogBootstrapRecordV1,
    observed: ValidatedCatalogRecoveryObservationV1,
) -> CatalogBootstrapRecoveryObservationV1 {
    use CatalogBootstrapRecoveryObservationV1 as Result;
    use CatalogRecordObservationV1 as Record;
    use ValidatedFinalObservationV1 as Final;
    use ValidatedStagingObservationV1 as Staging;

    match (
        observed.scratch,
        observed.active,
        observed.staging,
        observed.final_directory,
        observed.retired,
    ) {
        (
            Record::Missing | Record::PartialExpectedPrefix,
            Record::Missing,
            Staging::Missing,
            Final::Missing,
            Record::Missing,
        ) => Result::WriteOrRewriteScratch,
        (
            Record::Exact(value),
            Record::Missing,
            Staging::Missing,
            Final::Missing,
            Record::Missing,
        ) if value.as_ref() == expected => Result::PublishActive,
        (
            Record::Missing,
            Record::Exact(value),
            Staging::Missing | Staging::PartialExpectedContents | Staging::OwnedMissingRecord(_),
            Final::Missing,
            Record::Missing,
        ) if value.as_ref() == expected => Result::PrepareOrRewriteStaging,
        (
            Record::Missing,
            Record::Exact(value),
            Staging::Exact(exact),
            Final::Missing,
            Record::Missing,
        ) if value.as_ref() == expected => Result::PublishFinal(exact),
        (
            Record::Missing,
            Record::Exact(value),
            Staging::Missing,
            Final::Exact(exact),
            Record::Missing,
        ) if value.as_ref() == expected => Result::RetireActive(exact),
        (
            Record::Missing,
            Record::Missing,
            Staging::Missing,
            Final::Exact(exact),
            Record::Exact(value),
        ) if value.as_ref() == expected => Result::Complete(exact),
        _ => Result::Ambiguous,
    }
}
