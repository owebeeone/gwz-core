//! Reservation-bound, role-typed namespace durability contracts.
//!
//! Checked-artifact consumers receive only `ActionNamespace`. The raw platform
//! backend and its capability issuer are private children of this module.

mod backend;
mod evidence;
mod provider_compile;
mod roles;

#[cfg(test)]
pub(super) mod test_support;

use super::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
};
use super::protocol::{
    ActionDigestV1, ActionSlotV1, AdmittedActionV1, BarrierIntentV1, BarrierOrdinalV1,
    BaseActionSlotV1, CleanupAliasV1, ProtocolCodecErrorV1, RecordDigestV1, ScheduleErrorV1,
};
use backend::{ActionDestination, ProviderBinding, RawNamespaceBackend};
pub(in crate::checked_artifact) use evidence::{
    InstalledManagedComponentV1, RetiredManagedMarkerV1,
};
pub(super) use roles::*;

pub(super) use backend::{
    DurableNamespace, NamespaceObjectKind, PublishedIdentity, RetainedDirectory,
    RetainedNamespaceObject, RetiredIdentity,
};
#[cfg(test)]
pub(super) use backend::{ReservedNamespaceSlot, ReservedRetirementSlot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActionBinding {
    action: ActionDigestV1,
    reservation: RecordDigestV1,
    provider: ProviderBinding,
}

pub(in crate::checked_artifact) struct NamespaceBarrierAuthority {
    _sealed: (),
}

impl NamespaceBarrierAuthority {
    const fn owner() -> Self {
        Self { _sealed: () }
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) const fn test_only() -> Self {
        Self::owner()
    }
}

/// The only namespace capability passed to checked-artifact consumers.
pub(super) struct ActionNamespace<Implementation> {
    backend: Implementation,
    admitted_action: AdmittedActionV1,
}

impl<Implementation> ActionNamespace<Implementation> {
    pub(super) fn from_admitted(
        backend: Implementation,
        admitted_action: AdmittedActionV1,
    ) -> Self {
        Self {
            backend,
            admitted_action,
        }
    }

    pub(super) fn admitted_action(&self) -> &AdmittedActionV1 {
        &self.admitted_action
    }
}

impl<Implementation: RawNamespaceBackend> ActionNamespace<Implementation> {
    fn binding(&self) -> ActionBinding {
        ActionBinding {
            action: self.admitted_action.reservation().action_digest(),
            reservation: self.admitted_action.reservation().record_digest(),
            provider: self.backend.provider_binding(),
        }
    }

    pub(super) fn publish_destination(&self, role: PublishRoleV1) -> PublishDestination {
        let binding = self.binding();
        PublishDestination {
            binding,
            destination: action_destination(binding, ActionSlotV1::Base(role.slot())),
        }
    }

    pub(super) fn cleanup_retirement(
        &self,
        alias: CleanupAliasV1,
    ) -> Result<CleanupRetirementDestination, ScheduleErrorV1> {
        let bit = match alias {
            CleanupAliasV1::Source => 1,
            CleanupAliasV1::Goal => 2,
            CleanupAliasV1::Authority => 4,
        };
        if self
            .admitted_action
            .reservation()
            .schedule()
            .cleanup_aliases()
            .mask()
            & bit
            == 0
        {
            return Err(ScheduleErrorV1::OutOfBounds);
        }
        let slot = match alias {
            CleanupAliasV1::Source => BaseActionSlotV1::RetiredSourceAlias,
            CleanupAliasV1::Goal => BaseActionSlotV1::RetiredGoalAlias,
            CleanupAliasV1::Authority => BaseActionSlotV1::RetiredAuthorityAlias,
        };
        let binding = self.binding();
        Ok(CleanupRetirementDestination {
            binding,
            alias,
            destination: action_destination(binding, ActionSlotV1::Base(slot)),
        })
    }

    pub(super) fn scheduled_barrier(
        &self,
        index: usize,
    ) -> Result<ScheduledBarrierOrdinal, ScheduleErrorV1> {
        if index
            >= self
                .admitted_action
                .reservation()
                .schedule()
                .barrier_count()
        {
            return Err(ScheduleErrorV1::OutOfBounds);
        }
        Ok(ScheduledBarrierOrdinal {
            binding: self.binding(),
            ordinal: BarrierOrdinalV1::new(index)?,
        })
    }

    pub(super) fn barrier_slots<DirectoryHandle, Identity, PathProfile>(
        &self,
        scheduled: ScheduledBarrierOrdinal,
        target: BarrierTarget<DirectoryHandle, Identity, PathProfile>,
    ) -> Result<BarrierSlots<DirectoryHandle, Identity, PathProfile>, ScheduleErrorV1> {
        if scheduled.binding != self.binding()
            || target.parent.provider() != self.binding().provider
            || target.action != scheduled.binding.action
            || target.reservation != scheduled.binding.reservation
            || target.ordinal != scheduled.ordinal
        {
            return Err(ScheduleErrorV1::OutOfBounds);
        }
        let index = scheduled.ordinal.index() as u8;
        let binding = scheduled.binding;
        Ok(BarrierSlots {
            binding,
            ordinal: scheduled.ordinal,
            active: action_destination(binding, ActionSlotV1::BarrierIntentActive(index)),
            retired: action_destination(binding, ActionSlotV1::BarrierIntentRetired(index)),
            retired_anchor_alias: action_destination(
                binding,
                ActionSlotV1::RetiredRoamingAnchorAlias(index),
            ),
            target,
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the barrier record binds each independent retained namespace fact"
    )]
    pub(super) fn barrier_intent<DirectoryHandle>(
        &self,
        slots: &BarrierSlots<DirectoryHandle, DurableObjectIdentityV1, CanonicalPathIdentityV1>,
        catalog_anchor_identity: DurableObjectIdentityV1,
        private_home_parent_identity: DurableObjectIdentityV1,
        private_home_name: AsciiComponent,
    ) -> Result<BarrierIntentV1, ProtocolCodecErrorV1> {
        if slots.binding != self.binding() {
            return Err(ProtocolCodecErrorV1::Invalid(
                "barrier slots do not belong to the admitted action",
            ));
        }
        BarrierIntentV1::issue(
            &NamespaceBarrierAuthority::owner(),
            self.admitted_action.reservation(),
            slots.ordinal,
            catalog_anchor_identity,
            private_home_parent_identity,
            private_home_name,
            slots.target.parent.identity().clone(),
            slots.target.parent.path_profile().clone(),
            slots.target.leaf.clone(),
        )
    }

    pub(super) fn bootstrap_slots(&self, index: usize) -> Result<BootstrapSlots, ScheduleErrorV1> {
        let row = self
            .admitted_action
            .reservation()
            .schedule()
            .bootstrap_rows()
            .get(index)
            .ok_or(ScheduleErrorV1::OutOfBounds)?;
        Ok(BootstrapSlots {
            binding: self.binding(),
            bootstrap_ordinal: row.ordinal(),
            generation_range: row.generation_range(),
            component_range: row.component_range(),
        })
    }

    pub(super) fn publish_no_replace(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            Implementation::Identity,
            Implementation::PathProfile,
        >,
        destination: &PublishDestination,
    ) -> Result<PublishedIdentity<Implementation::Identity>, CheckedFsError> {
        self.validate_operation(source.provider(), destination.binding)?;
        self.backend
            .publish_no_replace(source, &destination.destination)
    }

    pub(super) fn retire_exact(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            Implementation::Identity,
            Implementation::PathProfile,
        >,
        destination: &CleanupRetirementDestination,
    ) -> Result<RetiredIdentity<Implementation::Identity>, CheckedFsError> {
        self.validate_operation(source.provider(), destination.binding)?;
        self.backend.retire_exact(source, &destination.destination)
    }

    pub(super) fn barrier_namespace(
        &mut self,
        parent: &RetainedDirectory<
            Implementation::DirectoryHandle,
            Implementation::Identity,
            Implementation::PathProfile,
        >,
        slots: &BarrierSlots<
            Implementation::DirectoryHandle,
            Implementation::Identity,
            Implementation::PathProfile,
        >,
    ) -> Result<DurableNamespace, CheckedFsError> {
        self.validate_operation(parent.provider(), slots.binding)?;
        if slots.target.parent.provider() != parent.provider()
            || slots.target.parent.identity() != parent.identity()
        {
            return Err(binding_error("barrier target parent changed"));
        }
        self.backend.barrier(parent, slots.ordinal)
    }

    fn validate_operation(
        &mut self,
        provider: ProviderBinding,
        binding: ActionBinding,
    ) -> Result<(), CheckedFsError> {
        if binding != self.binding() || provider != self.backend.provider_binding() {
            return Err(binding_error("namespace capability binding mismatch"));
        }
        self.backend.revalidate_action_directory(
            self.admitted_action.directory_identity(),
            self.admitted_action.reservation().record_digest(),
        )
    }
}

fn action_destination(binding: ActionBinding, slot: ActionSlotV1) -> ActionDestination {
    ActionDestination::new(
        AsciiComponent::parse(slot.name(binding.action).as_bytes())
            .expect("fixed action slot name is valid"),
        binding.reservation,
    )
}

fn binding_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("action namespace", detail)
}

#[cfg(not(test))]
pub(super) trait NamespaceProtocol: backend::SealedActionNamespace {
    type DirectoryHandle;
    type ObjectHandle;
    type Identity: Clone + Eq;
    type PathProfile;
    type ReservationBinding;
    type BarrierOrdinal;

    fn barrier(
        &mut self,
        parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        ordinal: Self::BarrierOrdinal,
    ) -> Result<DurableNamespace, CheckedFsError>;
}

#[cfg(not(test))]
impl<Implementation: RawNamespaceBackend> backend::SealedActionNamespace
    for ActionNamespace<Implementation>
{
}

#[cfg(not(test))]
impl<Implementation: RawNamespaceBackend> NamespaceProtocol for ActionNamespace<Implementation> {
    type DirectoryHandle = Implementation::DirectoryHandle;
    type ObjectHandle = Implementation::ObjectHandle;
    type Identity = Implementation::Identity;
    type PathProfile = Implementation::PathProfile;
    type ReservationBinding = RecordDigestV1;
    type BarrierOrdinal = BarrierSlots<
        Implementation::DirectoryHandle,
        Implementation::Identity,
        Implementation::PathProfile,
    >;

    fn barrier(
        &mut self,
        parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        ordinal: Self::BarrierOrdinal,
    ) -> Result<DurableNamespace, CheckedFsError> {
        self.barrier_namespace(parent, &ordinal)
    }
}

#[cfg(test)]
pub(super) use test_support::NamespaceProtocol;
