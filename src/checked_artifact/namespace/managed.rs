//! Owner-private managed mutation and restart observation capabilities.

use super::backend::ProviderBinding;
use super::{BootstrapComponentSlots, binding_error};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
    DurablePathV1, PathComponentMode,
};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, BootstrapComponentOrdinalV1, BootstrapOrdinalV1,
    ManagedBootstrapPhaseV1, ManagedParentBootstrapIntentV1, OwnershipMarkerV1, RecordDigestV1,
    read_and_bind_ownership_marker,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedBindingV1 {
    provider: ProviderBinding,
    action: ActionDigestV1,
    reservation: RecordDigestV1,
    intent_id: [u8; 32],
    bootstrap_ordinal: BootstrapOrdinalV1,
    component_ordinal: BootstrapComponentOrdinalV1,
    target_parent_identity: DurableObjectIdentityV1,
    target_parent_path: CanonicalPathIdentityV1,
    staging_leaf: AsciiComponent,
    final_leaf: AsciiComponent,
    marker_retirement_leaf: AsciiComponent,
}

pub(in crate::checked_artifact) struct ManagedInstallRequestV1 {
    binding: ManagedBindingV1,
    expected_marker: OwnershipMarkerV1,
}

pub(in crate::checked_artifact) struct ManagedMarkerRetirementRequestV1 {
    binding: ManagedBindingV1,
    expected_marker_id: [u8; 32],
    expected_marker_object_identity: DurableObjectIdentityV1,
    installed_parent_identity: DurableObjectIdentityV1,
    installed_parent_mode: PathComponentMode,
    installed_parent_path: DurablePathV1,
    /// The intent this retirement is bound to, kept so the backend can turn the
    /// *durable* marker bytes it observed into the `OwnershipMarkerV1` the
    /// observation must carry. `read_and_bind_ownership_marker` is the only
    /// construction path for that value and it validates against exactly this
    /// intent and cursor, so the marker a retirement reports is always the one
    /// the intent recorded — never one the backend chose.
    intent: ManagedParentBootstrapIntentV1,
    local_component: usize,
}

pub(in crate::checked_artifact) struct ManagedInstallObservationV1 {
    binding: ManagedBindingV1,
    marker: OwnershipMarkerV1,
    marker_object_identity: DurableObjectIdentityV1,
    installed_identity: DurableObjectIdentityV1,
    installed_mode: PathComponentMode,
    installed_path: CanonicalPathIdentityV1,
}

pub(in crate::checked_artifact) struct ManagedMarkerRetirementObservationV1 {
    binding: ManagedBindingV1,
    marker: OwnershipMarkerV1,
    retired_marker_identity: DurableObjectIdentityV1,
    installed_parent_identity: DurableObjectIdentityV1,
    installed_parent_mode: PathComponentMode,
    installed_parent_path: CanonicalPathIdentityV1,
}

impl ManagedInstallRequestV1 {
    pub(super) fn bind<DirectoryHandle>(
        provider: ProviderBinding,
        reservation: &ActionCapacityReservationV1,
        intent: &ManagedParentBootstrapIntentV1,
        slots: &BootstrapComponentSlots<
            DirectoryHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<Self, CheckedFsError> {
        let binding = bind_current_component(
            provider,
            reservation,
            intent,
            slots,
            ManagedBootstrapPhaseV1::InstallComponents,
        )?;
        let expected_marker = OwnershipMarkerV1::for_current_component(intent)
            .map_err(|_| binding_error("managed install intent cannot issue its marker"))?;
        Ok(Self {
            binding,
            expected_marker,
        })
    }

    /// The marker the staged component must already carry, and the two frozen
    /// managed names its edge moves between. The backend needs all three to run
    /// edge E15 and its restart observation; none of them is caller-chosen.
    pub(super) const fn expected_marker(&self) -> &OwnershipMarkerV1 {
        &self.expected_marker
    }

    pub(super) fn staging_leaf(&self) -> &AsciiComponent {
        &self.binding.staging_leaf
    }

    pub(super) fn final_leaf(&self) -> &AsciiComponent {
        &self.binding.final_leaf
    }

    pub(super) const fn reservation(&self) -> RecordDigestV1 {
        self.binding.reservation
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the private provider reports each independently observed durable fact"
    )]
    pub(super) fn complete(
        &self,
        provider: ProviderBinding,
        marker: OwnershipMarkerV1,
        marker_object_identity: DurableObjectIdentityV1,
        installed_identity: DurableObjectIdentityV1,
        installed_mode: PathComponentMode,
        installed_path: CanonicalPathIdentityV1,
    ) -> Result<ManagedInstallObservationV1, CheckedFsError> {
        if provider != self.binding.provider || marker != self.expected_marker {
            return Err(binding_error(
                "managed install observation binding mismatch",
            ));
        }
        let components = installed_path.components();
        let parent = self.binding.target_parent_path.components();
        let Some(installed) = components.strip_prefix(parent) else {
            return Err(binding_error("managed install observation changed root"));
        };
        if installed.len() != 1
            || installed[0].original() != &self.binding.final_leaf
            || installed[0].parent_durable_identity() != &self.binding.target_parent_identity
            || marker_object_identity.support_profile() != installed_identity.support_profile()
            || installed_identity.support_profile()
                != self.binding.target_parent_identity.support_profile()
        {
            return Err(binding_error("managed install observation is not exact"));
        }
        Ok(ManagedInstallObservationV1 {
            binding: self.binding.clone(),
            marker,
            marker_object_identity,
            installed_identity,
            installed_mode,
            installed_path,
        })
    }
}

impl ManagedMarkerRetirementRequestV1 {
    pub(super) fn bind<DirectoryHandle>(
        provider: ProviderBinding,
        reservation: &ActionCapacityReservationV1,
        intent: &ManagedParentBootstrapIntentV1,
        slots: &BootstrapComponentSlots<
            DirectoryHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<Self, CheckedFsError> {
        let binding = bind_current_component(
            provider,
            reservation,
            intent,
            slots,
            ManagedBootstrapPhaseV1::RetireMarkers,
        )?;
        let component = &intent.components()[intent.cursor()];
        Ok(Self {
            binding,
            expected_marker_id: component
                .ownership_marker_id()
                .ok_or_else(|| binding_error("managed marker identity is missing"))?,
            expected_marker_object_identity: component
                .ownership_marker_object_identity()
                .ok_or_else(|| binding_error("managed marker object identity is missing"))?
                .clone(),
            installed_parent_identity: component
                .installed_identity()
                .ok_or_else(|| binding_error("managed installed identity is missing"))?
                .clone(),
            installed_parent_mode: component
                .installed_mode()
                .ok_or_else(|| binding_error("managed installed mode is missing"))?,
            installed_parent_path: component
                .installed_path()
                .ok_or_else(|| binding_error("managed installed path is missing"))?
                .clone(),
            intent: intent.clone(),
            local_component: intent.cursor(),
        })
    }

    /// The frozen retirement destination row, and the installed component the
    /// marker retires out of.
    pub(super) fn marker_retirement_leaf(&self) -> &AsciiComponent {
        &self.binding.marker_retirement_leaf
    }

    pub(super) fn final_leaf(&self) -> &AsciiComponent {
        &self.binding.final_leaf
    }

    pub(super) const fn reservation(&self) -> RecordDigestV1 {
        self.binding.reservation
    }

    /// Binds durable ownership-marker bytes back into this retirement's intent.
    /// The bytes are the ones the provider read from the retired row; the
    /// protocol re-derives the marker's own id from them and rejects any marker
    /// that is not this intent's component, so a substituted or replayed marker
    /// is a typed refusal rather than accepted evidence.
    pub(super) fn bind_marker_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<OwnershipMarkerV1, CheckedFsError> {
        read_and_bind_ownership_marker(
            std::io::Cursor::new(bytes),
            &self.intent,
            self.local_component,
        )
        .map(|bound| bound.value().clone())
        .map_err(|_| binding_error("retired ownership marker does not bind to the intent"))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the private provider reports each independently observed durable fact"
    )]
    pub(super) fn complete(
        &self,
        provider: ProviderBinding,
        marker: OwnershipMarkerV1,
        retired_marker_identity: DurableObjectIdentityV1,
        installed_parent_identity: DurableObjectIdentityV1,
        installed_parent_mode: PathComponentMode,
        installed_parent_path: CanonicalPathIdentityV1,
    ) -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError> {
        let durable_installed_parent_path = DurablePathV1::from_live(&installed_parent_path)?;
        if provider != self.binding.provider
            || marker.marker_id() != self.expected_marker_id
            || retired_marker_identity != self.expected_marker_object_identity
            || installed_parent_identity != self.installed_parent_identity
            || installed_parent_mode != self.installed_parent_mode
            || durable_installed_parent_path != self.installed_parent_path
        {
            return Err(binding_error(
                "managed marker retirement observation binding mismatch",
            ));
        }
        Ok(ManagedMarkerRetirementObservationV1 {
            binding: self.binding.clone(),
            marker,
            retired_marker_identity,
            installed_parent_identity,
            installed_parent_mode,
            installed_parent_path,
        })
    }

    pub(super) fn matches_retained_marker(
        &self,
        marker_identity: &DurableObjectIdentityV1,
        installed_parent_identity: &DurableObjectIdentityV1,
        installed_parent_path: &CanonicalPathIdentityV1,
    ) -> bool {
        marker_identity == &self.expected_marker_object_identity
            && installed_parent_identity == &self.installed_parent_identity
            && DurablePathV1::from_live(installed_parent_path)
                .is_ok_and(|path| path == self.installed_parent_path)
    }
}

fn bind_current_component<DirectoryHandle>(
    provider: ProviderBinding,
    reservation: &ActionCapacityReservationV1,
    intent: &ManagedParentBootstrapIntentV1,
    slots: &BootstrapComponentSlots<
        DirectoryHandle,
        DurableObjectIdentityV1,
        CanonicalPathIdentityV1,
    >,
    phase: ManagedBootstrapPhaseV1,
) -> Result<ManagedBindingV1, CheckedFsError> {
    let component = intent
        .components()
        .get(intent.cursor())
        .ok_or_else(|| binding_error("managed intent has no current component"))?;
    let component_ordinal = BootstrapComponentOrdinalV1::new(slots.global_component_ordinal)
        .map_err(|_| binding_error("managed component ordinal is invalid"))?;
    if provider != slots.binding.provider
        || !intent.matches_reservation(reservation)
        || intent.phase() != phase
        || slots.binding.action != reservation.action_digest()
        || slots.binding.reservation != reservation.record_digest()
        || component.global_component_ordinal() != component_ordinal
        || component.staging_name() != &slots.target.staging_leaf
        || component.final_name() != &slots.target.final_leaf
        || !intent.matches_component_parent(
            intent.cursor(),
            slots.target.parent.identity(),
            slots.target.parent.path_profile(),
        )
    {
        return Err(binding_error(
            "managed intent and namespace slots do not match",
        ));
    }
    Ok(ManagedBindingV1 {
        provider,
        action: slots.binding.action,
        reservation: slots.binding.reservation,
        intent_id: intent.intent_id(),
        bootstrap_ordinal: slots.bootstrap_ordinal,
        component_ordinal,
        target_parent_identity: slots.target.parent.identity().clone(),
        target_parent_path: slots.target.parent.path_profile().clone(),
        staging_leaf: slots.target.staging_leaf.clone(),
        final_leaf: slots.target.final_leaf.clone(),
        marker_retirement_leaf: slots.marker_retired.leaf().clone(),
    })
}

macro_rules! observation_accessors {
    ($type:ident) => {
        impl $type {
            pub(super) fn binding_matches_install(
                &self,
                request: &ManagedInstallRequestV1,
            ) -> bool {
                self.binding == request.binding
            }

            pub(super) const fn provider(&self) -> ProviderBinding {
                self.binding.provider
            }
        }
    };
}

observation_accessors!(ManagedInstallObservationV1);

impl ManagedMarkerRetirementObservationV1 {
    pub(super) fn binding_matches_retirement(
        &self,
        request: &ManagedMarkerRetirementRequestV1,
    ) -> bool {
        self.binding == request.binding
    }

    pub(super) const fn provider(&self) -> ProviderBinding {
        self.binding.provider
    }
}

impl ManagedInstallObservationV1 {
    pub(super) fn into_evidence_parts(
        self,
    ) -> (
        ManagedBindingFactsV1,
        OwnershipMarkerV1,
        DurableObjectIdentityV1,
        DurableObjectIdentityV1,
        PathComponentMode,
        CanonicalPathIdentityV1,
    ) {
        (
            self.binding.into_facts(),
            self.marker,
            self.marker_object_identity,
            self.installed_identity,
            self.installed_mode,
            self.installed_path,
        )
    }
}

impl ManagedMarkerRetirementObservationV1 {
    pub(super) fn into_evidence_parts(
        self,
    ) -> (
        ManagedBindingFactsV1,
        OwnershipMarkerV1,
        DurableObjectIdentityV1,
        DurableObjectIdentityV1,
        PathComponentMode,
        CanonicalPathIdentityV1,
    ) {
        (
            self.binding.into_facts(),
            self.marker,
            self.retired_marker_identity,
            self.installed_parent_identity,
            self.installed_parent_mode,
            self.installed_parent_path,
        )
    }
}

pub(super) struct ManagedBindingFactsV1 {
    pub(super) provider: ProviderBinding,
    pub(super) action: ActionDigestV1,
    pub(super) reservation: RecordDigestV1,
    pub(super) intent_id: [u8; 32],
    pub(super) bootstrap_ordinal: BootstrapOrdinalV1,
    pub(super) component_ordinal: BootstrapComponentOrdinalV1,
    pub(super) staging_leaf: AsciiComponent,
    pub(super) final_leaf: AsciiComponent,
    pub(super) marker_retirement_leaf: AsciiComponent,
}

impl ManagedBindingV1 {
    fn into_facts(self) -> ManagedBindingFactsV1 {
        ManagedBindingFactsV1 {
            provider: self.provider,
            action: self.action,
            reservation: self.reservation,
            intent_id: self.intent_id,
            bootstrap_ordinal: self.bootstrap_ordinal,
            component_ordinal: self.component_ordinal,
            staging_leaf: self.staging_leaf,
            final_leaf: self.final_leaf,
            marker_retirement_leaf: self.marker_retirement_leaf,
        }
    }
}
