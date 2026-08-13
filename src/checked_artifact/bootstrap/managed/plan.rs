//! Immutable managed-parent plan and admitted binding types.

use std::ops::Range;

use super::*;
use crate::checked_artifact::protocol::{
    ActionDigestV1, AdmittedActionV1, BootstrapOrdinalV1, ManagedBootstrapInputV1, RecordDigestV1,
    RequestOwnerBindingV1, ScheduleDigestV1,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ManagedParentProviderBindingV1(pub(super) [u8; 32]);

impl ManagedParentProviderBindingV1 {
    pub(super) fn try_new(value: [u8; 32]) -> Result<Self, CheckedFsError> {
        if value == [0; 32] {
            return Err(CheckedFsError::ambiguous(
                "managed-parent provider",
                "provider instance binding must be nonzero",
            ));
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedParentPlanRowV1 {
    pub(super) declared_order: usize,
    pub(super) purpose: ManagedParentPurpose,
    pub(super) retained_existing_parent_count: usize,
    pub(super) retained_parent_identity: DurableObjectIdentityV1,
    pub(super) retained_parent_mode: PathComponentMode,
    pub(super) retained_parent_path: CanonicalPathIdentityV1,
    pub(super) components: Vec<AsciiComponent>,
    pub(super) missing_suffix: Vec<AsciiComponent>,
    pub(super) spec_digest: [u8; 32],
}

impl ManagedParentPlanRowV1 {
    pub(in crate::checked_artifact) const fn declared_order(&self) -> usize {
        self.declared_order
    }

    pub(in crate::checked_artifact) const fn purpose(&self) -> ManagedParentPurpose {
        self.purpose
    }

    pub(in crate::checked_artifact) const fn retained_existing_parent_count(&self) -> usize {
        self.retained_existing_parent_count
    }

    pub(in crate::checked_artifact) fn retained_parent_identity(&self) -> &DurableObjectIdentityV1 {
        &self.retained_parent_identity
    }

    pub(in crate::checked_artifact) const fn retained_parent_mode(&self) -> PathComponentMode {
        self.retained_parent_mode
    }

    pub(in crate::checked_artifact) fn retained_parent_path(&self) -> &CanonicalPathIdentityV1 {
        &self.retained_parent_path
    }

    pub(in crate::checked_artifact) fn components(&self) -> &[AsciiComponent] {
        &self.components
    }

    pub(in crate::checked_artifact) fn missing_suffix(&self) -> &[AsciiComponent] {
        &self.missing_suffix
    }

    pub(in crate::checked_artifact) const fn spec_digest(&self) -> [u8; 32] {
        self.spec_digest
    }
}

/// Opaque aggregate inputs produced only from one immutable preflight plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedParentScheduleInputsV1 {
    pub(super) plan_digest: [u8; 32],
    pub(super) rows: Vec<ManagedBootstrapInputV1>,
}

impl ManagedParentScheduleInputsV1 {
    pub(in crate::checked_artifact) const fn plan_digest(&self) -> [u8; 32] {
        self.plan_digest
    }

    pub(in crate::checked_artifact) fn rows(&self) -> &[ManagedBootstrapInputV1] {
        &self.rows
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedParentPlanV1 {
    pub(super) provider_instance: ManagedParentProviderBindingV1,
    pub(super) action_digest: ActionDigestV1,
    pub(super) request_owner_binding: RequestOwnerBindingV1,
    pub(super) rows: Vec<ManagedParentPlanRowV1>,
    pub(super) digest: [u8; 32],
    pub(super) schedule_inputs: ManagedParentScheduleInputsV1,
}

impl ManagedParentPlanV1 {
    pub(in crate::checked_artifact) const fn action_digest(&self) -> ActionDigestV1 {
        self.action_digest
    }

    pub(in crate::checked_artifact) const fn request_owner_binding(&self) -> RequestOwnerBindingV1 {
        self.request_owner_binding
    }

    pub(in crate::checked_artifact) fn rows(&self) -> &[ManagedParentPlanRowV1] {
        &self.rows
    }

    pub(in crate::checked_artifact) const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    pub(in crate::checked_artifact) fn schedule_inputs(&self) -> &ManagedParentScheduleInputsV1 {
        &self.schedule_inputs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct BoundManagedParentPlanRowV1 {
    pub(super) plan_row: ManagedParentPlanRowV1,
    pub(super) bootstrap_ordinal: BootstrapOrdinalV1,
    pub(super) generation_range: Range<usize>,
    pub(super) component_range: Range<usize>,
}

impl BoundManagedParentPlanRowV1 {
    pub(in crate::checked_artifact) const fn purpose(&self) -> ManagedParentPurpose {
        self.plan_row.purpose()
    }

    pub(in crate::checked_artifact) const fn declared_order(&self) -> usize {
        self.plan_row.declared_order()
    }

    pub(in crate::checked_artifact) const fn retained_existing_parent_count(&self) -> usize {
        self.plan_row.retained_existing_parent_count()
    }

    pub(in crate::checked_artifact) fn retained_parent_identity(&self) -> &DurableObjectIdentityV1 {
        self.plan_row.retained_parent_identity()
    }

    pub(in crate::checked_artifact) const fn retained_parent_mode(&self) -> PathComponentMode {
        self.plan_row.retained_parent_mode()
    }

    pub(in crate::checked_artifact) fn retained_parent_path(&self) -> &CanonicalPathIdentityV1 {
        self.plan_row.retained_parent_path()
    }

    pub(in crate::checked_artifact) fn components(&self) -> &[AsciiComponent] {
        self.plan_row.components()
    }

    pub(in crate::checked_artifact) fn missing_suffix(&self) -> &[AsciiComponent] {
        self.plan_row.missing_suffix()
    }

    pub(in crate::checked_artifact) const fn spec_digest(&self) -> [u8; 32] {
        self.plan_row.spec_digest()
    }

    pub(in crate::checked_artifact) const fn bootstrap_ordinal(&self) -> BootstrapOrdinalV1 {
        self.bootstrap_ordinal
    }

    pub(in crate::checked_artifact) fn generation_range(&self) -> Range<usize> {
        self.generation_range.clone()
    }

    pub(in crate::checked_artifact) fn component_range(&self) -> Range<usize> {
        self.component_range.clone()
    }
}

/// Opaque authority proving that one immutable plan is the exact resident
/// schedule of one admitted action directory.
pub(in crate::checked_artifact) struct BoundManagedParentPlanV1 {
    pub(super) provider_instance: ManagedParentProviderBindingV1,
    pub(super) admitted_action: AdmittedActionV1,
    pub(super) plan: ManagedParentPlanV1,
    pub(super) rows: Vec<BoundManagedParentPlanRowV1>,
}

impl BoundManagedParentPlanV1 {
    pub(in crate::checked_artifact) fn reservation(
        &self,
    ) -> &crate::checked_artifact::protocol::ActionCapacityReservationV1 {
        self.admitted_action.reservation()
    }

    pub(in crate::checked_artifact) fn plan(&self) -> &ManagedParentPlanV1 {
        &self.plan
    }

    pub(in crate::checked_artifact) fn rows(&self) -> &[BoundManagedParentPlanRowV1] {
        &self.rows
    }

    pub(in crate::checked_artifact) fn scheduled_row(
        &self,
        purpose: ManagedParentPurpose,
    ) -> Option<&BoundManagedParentPlanRowV1> {
        self.rows.iter().find(|row| row.purpose() == purpose)
    }

    pub(in crate::checked_artifact) const fn action_digest(&self) -> ActionDigestV1 {
        self.plan.action_digest()
    }

    pub(in crate::checked_artifact) const fn request_owner_binding(&self) -> RequestOwnerBindingV1 {
        self.plan.request_owner_binding()
    }

    pub(in crate::checked_artifact) fn schedule_digest(&self) -> ScheduleDigestV1 {
        self.reservation().schedule().digest()
    }

    pub(in crate::checked_artifact) fn reservation_digest(&self) -> RecordDigestV1 {
        self.reservation().record_digest()
    }
}
