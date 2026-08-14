use crate::checked_artifact::capability::{
    DurableCatalogTargetDigestV1, DurableObjectIdentityV1, DurablePathV1,
    HistoricalCollisionDigestV1, PreCatalogRootKindV1, SupportedFilesystemProfile,
};
use crate::checked_artifact::protocol::{
    CatalogBootstrapRecordV1, CatalogBootstrapRecoveryDecisionV1, ScratchBytesV1,
    classify_expected_prefix,
};

use super::CatalogScratchNameV1;

#[derive(Clone)]
pub(in crate::checked_artifact) struct CatalogAttemptBindingV1 {
    root_kind: PreCatalogRootKindV1,
    support_profile: SupportedFilesystemProfile,
    durable_target_digest: DurableCatalogTargetDigestV1,
    current_historical_collision_digest: HistoricalCollisionDigestV1,
    retained_parent_identity: DurableObjectIdentityV1,
    retained_parent_path: DurablePathV1,
}

impl CatalogAttemptBindingV1 {
    pub(in crate::checked_artifact) fn owner_issue(
        root_kind: PreCatalogRootKindV1,
        support_profile: SupportedFilesystemProfile,
        durable_target_digest: DurableCatalogTargetDigestV1,
        current_historical_collision_digest: HistoricalCollisionDigestV1,
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_path: DurablePathV1,
    ) -> Self {
        Self {
            root_kind,
            support_profile,
            durable_target_digest,
            current_historical_collision_digest,
            retained_parent_identity,
            retained_parent_path,
        }
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn synthetic_for_test(
        root_kind: PreCatalogRootKindV1,
        support_profile: SupportedFilesystemProfile,
        durable_target_digest: DurableCatalogTargetDigestV1,
        current_historical_collision_digest: HistoricalCollisionDigestV1,
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_path: DurablePathV1,
    ) -> Self {
        Self::owner_issue(
            root_kind,
            support_profile,
            durable_target_digest,
            current_historical_collision_digest,
            retained_parent_identity,
            retained_parent_path,
        )
    }

    pub(in crate::checked_artifact) fn record_from_scratch(
        &self,
        scratch: &CatalogScratchNameV1,
    ) -> Result<CatalogBootstrapRecordV1, ()> {
        if scratch.durable_target_digest() != self.durable_target_digest {
            return Err(());
        }
        Ok(CatalogBootstrapRecordV1::owner_issue(
            self.root_kind,
            self.support_profile,
            scratch.durable_target_digest(),
            scratch.historical_collision_digest(),
            self.retained_parent_identity.clone(),
            self.retained_parent_path.clone(),
            scratch.ownership_token(),
        ))
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn with_current_historical_for_test(
        mut self,
        value: HistoricalCollisionDigestV1,
    ) -> Self {
        self.current_historical_collision_digest = value;
        self
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn with_target_for_test(
        mut self,
        value: DurableCatalogTargetDigestV1,
    ) -> Self {
        self.durable_target_digest = value;
        self
    }

    pub(in crate::checked_artifact) fn accepts(&self, record: &CatalogBootstrapRecordV1) -> bool {
        record.matches_attempt(
            self.root_kind,
            self.support_profile,
            self.durable_target_digest,
            &self.retained_parent_identity,
            &self.retained_parent_path,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogRecordFactV1 {
    Missing,
    Scratch {
        name: Box<CatalogScratchNameV1>,
        bytes: Vec<u8>,
    },
    Exact(Box<CatalogBootstrapRecordV1>),
    Other,
}

impl CatalogRecordFactV1 {
    pub(in crate::checked_artifact) fn scratch(name: CatalogScratchNameV1, bytes: Vec<u8>) -> Self {
        Self::Scratch {
            name: Box::new(name),
            bytes,
        }
    }

    pub(in crate::checked_artifact) fn exact(value: CatalogBootstrapRecordV1) -> Self {
        Self::Exact(Box::new(value))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogDirectoryFactV1 {
    Missing,
    ActiveOwnedPrefix,
    ExactOwned,
    Other,
}

pub(in crate::checked_artifact) struct CatalogAggregateFactsV1 {
    scratch: Vec<CatalogRecordFactV1>,
    active: CatalogRecordFactV1,
    staging: CatalogDirectoryFactV1,
    final_directory: CatalogDirectoryFactV1,
    retired: CatalogRecordFactV1,
}

impl CatalogAggregateFactsV1 {
    pub(in crate::checked_artifact) fn new(
        scratch: Vec<CatalogRecordFactV1>,
        active: CatalogRecordFactV1,
        staging: CatalogDirectoryFactV1,
        final_directory: CatalogDirectoryFactV1,
        retired: CatalogRecordFactV1,
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

pub(in crate::checked_artifact) struct CatalogClassificationV1 {
    decision: CatalogBootstrapRecoveryDecisionV1,
    expected: Option<CatalogBootstrapRecordV1>,
}

impl CatalogClassificationV1 {
    pub(in crate::checked_artifact) const fn decision(&self) -> CatalogBootstrapRecoveryDecisionV1 {
        self.decision
    }

    pub(in crate::checked_artifact) fn expected_record(&self) -> Option<&CatalogBootstrapRecordV1> {
        self.expected.as_ref()
    }
}

pub(in crate::checked_artifact) fn classify_catalog_attempt(
    binding: &CatalogAttemptBindingV1,
    aggregate: CatalogAggregateFactsV1,
) -> CatalogClassificationV1 {
    let mut expected: Option<CatalogBootstrapRecordV1> = None;
    let scratch_state = match aggregate.scratch.as_slice() {
        [] => ScratchState::Missing,
        [CatalogRecordFactV1::Scratch { name, bytes }] => {
            let Ok(record) = binding.record_from_scratch(name) else {
                return ambiguous();
            };
            let state = match classify_expected_prefix(bytes, &record.encode_canonical()) {
                ScratchBytesV1::PartialExpectedPrefix => ScratchState::Partial,
                ScratchBytesV1::Exact => ScratchState::Exact,
                ScratchBytesV1::Missing | ScratchBytesV1::Other => return ambiguous(),
            };
            expected = Some(record);
            state
        }
        _ => return ambiguous(),
    };

    if !merge_record(binding, &mut expected, &aggregate.active)
        || !merge_record(binding, &mut expected, &aggregate.retired)
    {
        return ambiguous();
    }
    let active_state = record_state(&aggregate.active);
    let retired_state = record_state(&aggregate.retired);

    let decision = match (
        expected.as_ref(),
        scratch_state,
        active_state,
        aggregate.staging,
        aggregate.final_directory,
        retired_state,
    ) {
        (
            None,
            ScratchState::Missing,
            RecordState::Missing,
            CatalogDirectoryFactV1::Missing,
            CatalogDirectoryFactV1::Missing,
            RecordState::Missing,
        ) => CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch,
        (
            Some(_),
            ScratchState::Partial,
            RecordState::Missing,
            CatalogDirectoryFactV1::Missing,
            CatalogDirectoryFactV1::Missing,
            RecordState::Missing,
        ) => CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch,
        (
            Some(_),
            ScratchState::Exact,
            RecordState::Missing,
            CatalogDirectoryFactV1::Missing,
            CatalogDirectoryFactV1::Missing,
            RecordState::Missing,
        ) => CatalogBootstrapRecoveryDecisionV1::PublishActive,
        (
            Some(_),
            ScratchState::Missing,
            RecordState::Exact,
            CatalogDirectoryFactV1::Missing | CatalogDirectoryFactV1::ActiveOwnedPrefix,
            CatalogDirectoryFactV1::Missing,
            RecordState::Missing,
        ) => CatalogBootstrapRecoveryDecisionV1::PrepareOrRewriteStaging,
        (
            Some(_),
            ScratchState::Missing,
            RecordState::Exact,
            CatalogDirectoryFactV1::ExactOwned,
            CatalogDirectoryFactV1::Missing,
            RecordState::Missing,
        ) => CatalogBootstrapRecoveryDecisionV1::PublishFinal,
        (
            Some(_),
            ScratchState::Missing,
            RecordState::Exact,
            CatalogDirectoryFactV1::Missing,
            CatalogDirectoryFactV1::ExactOwned,
            RecordState::Missing,
        ) => CatalogBootstrapRecoveryDecisionV1::RetireActive,
        (
            Some(_),
            ScratchState::Missing,
            RecordState::Missing,
            CatalogDirectoryFactV1::Missing,
            CatalogDirectoryFactV1::ExactOwned,
            RecordState::Exact,
        ) => CatalogBootstrapRecoveryDecisionV1::Complete,
        _ => CatalogBootstrapRecoveryDecisionV1::Ambiguous,
    };
    CatalogClassificationV1 { decision, expected }
}

#[derive(Clone, Copy)]
enum ScratchState {
    Missing,
    Partial,
    Exact,
}

#[derive(Clone, Copy)]
enum RecordState {
    Missing,
    Exact,
    Other,
}

fn record_state(value: &CatalogRecordFactV1) -> RecordState {
    match value {
        CatalogRecordFactV1::Missing => RecordState::Missing,
        CatalogRecordFactV1::Exact(_) => RecordState::Exact,
        CatalogRecordFactV1::Scratch { .. } | CatalogRecordFactV1::Other => RecordState::Other,
    }
}

fn merge_record(
    binding: &CatalogAttemptBindingV1,
    expected: &mut Option<CatalogBootstrapRecordV1>,
    fact: &CatalogRecordFactV1,
) -> bool {
    let CatalogRecordFactV1::Exact(value) = fact else {
        return matches!(fact, CatalogRecordFactV1::Missing);
    };
    if !binding.accepts(value) {
        return false;
    }
    match expected {
        Some(expected) => expected == value.as_ref(),
        None => {
            *expected = Some(value.as_ref().clone());
            true
        }
    }
}

fn ambiguous() -> CatalogClassificationV1 {
    CatalogClassificationV1 {
        decision: CatalogBootstrapRecoveryDecisionV1::Ambiguous,
        expected: None,
    }
}
