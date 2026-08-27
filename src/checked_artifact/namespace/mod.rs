//! Reservation-bound, role-typed namespace durability contracts.
//!
//! Checked-artifact consumers receive only `ActionNamespace`. The raw platform
//! backend and its capability issuer are private children of this module.

mod backend;
mod evidence;
/// R2-D Phase 2 Step 2.2 — the retained-handle production backend.
///
/// **The allow's expiry, re-anchored at R2-E Phase E2** (E2 review [P3-5]). Its
/// reason has said "entry-point reachability is R2-E" since Step 2.2. Two things
/// have moved under it since: E2 adds six barrier role methods with no
/// production caller (only `tests_barrier_matrix` reaches them), and Phase E4 —
/// the step that was to supply every entry point — was re-scheduled behind
/// R2-F's quarantine/relocation package at the E0 close, under the operator's
/// 2026-08-27 ruling (a). So the allow's expiry is **E4's landing, wherever
/// R2-F puts it**, not "R2-E" generically; if E4 has not landed by the R2-E
/// settle, E7 owes this allow a dated re-owning rather than letting it become
/// permanent by silence.
#[allow(
    dead_code,
    reason = "the managed-parent provider has driven this backend since Step 3.1; entry-point reachability is Phase E4, itself sequenced behind R2-F's relocation"
)]
mod host;
mod managed;
mod operations;
mod provider_compile;
mod roles;

#[cfg(test)]
pub(super) mod test_support;
#[cfg(test)]
mod tests_backend;
/// R2-E Phase E2.3 — the `barrier.*` family's executed matrix, all sixteen keys.
#[cfg(test)]
mod tests_barrier_matrix;
/// R2-E Phase E1 Step E1.2 — the executed `cleanup.*` matrix.
#[cfg(test)]
mod tests_cleanup_matrix;
#[cfg(test)]
mod tests_fault_matrix;
#[cfg(test)]
mod tests_managed;
#[cfg(test)]
mod tests_managed_matrix;

#[allow(
    unused_imports,
    reason = "the sole production namespace backend; the Step-3.1 provider binds it, and entry-point reachability is R2-E"
)]
pub(in crate::checked_artifact) use host::{HostActionNamespaceV1, retain_action_namespace};

use super::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
    RoamingAnchorHomeWitnessV1,
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
        // R2-E Phase E1 Step E1.1 — the row this alias retires *out of*. The
        // pairing is the coordinator's own: `derive_new_reservation` reserves
        // `Source` for the request's expected leaf, `Goal` for its goal leaf and
        // `Authority` for the action's authority record
        // (`coordinator/schedule.rs:39-49`). Both names come from the same frozen
        // `BaseActionSlotV1` vocabulary, so this mints nothing.
        let source_slot = match alias {
            CleanupAliasV1::Source => BaseActionSlotV1::SourcePayload,
            CleanupAliasV1::Goal => BaseActionSlotV1::GoalPayload,
            CleanupAliasV1::Authority => BaseActionSlotV1::Authority,
        };
        let binding = self.binding();
        Ok(CleanupRetirementDestination {
            binding,
            alias,
            source: action_destination(binding, ActionSlotV1::Base(source_slot)),
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
            scratch: action_destination(
                binding,
                ActionSlotV1::Base(BaseActionSlotV1::BarrierIntentScratch),
            ),
            active: action_destination(binding, ActionSlotV1::BarrierIntentActive(index)),
            retired: action_destination(binding, ActionSlotV1::BarrierIntentRetired(index)),
            retired_anchor_alias: action_destination(
                binding,
                ActionSlotV1::RetiredRoamingAnchorAlias(index),
            ),
            target,
        })
    }

    /// R2-E Phase E2 (O6). The three roaming-anchor-home facts are no longer
    /// forwarded from this owner's caller: they arrive as the pre-catalog
    /// provider owner's own [`RoamingAnchorHomeWitnessV1`], which the caller
    /// receives from `OpaqueRetainedCatalogV1::observe_roaming_anchor_home` and
    /// cannot construct. The target parent's identity and path profile were
    /// already provider-observed (`BackendIssuer::barrier_target` mints them
    /// from `retained_parent()`), so after this change no field of
    /// `BarrierIntentV1` is a caller restatement.
    pub(super) fn barrier_intent<DirectoryHandle>(
        &self,
        slots: &BarrierSlots<DirectoryHandle, DurableObjectIdentityV1, CanonicalPathIdentityV1>,
        home: &RoamingAnchorHomeWitnessV1,
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
            home,
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
