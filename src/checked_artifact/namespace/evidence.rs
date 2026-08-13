//! Sealed provider-issued evidence for managed-parent state transitions.

use super::backend::ProviderBinding;
use super::managed::{ManagedInstallObservationV1, ManagedMarkerRetirementObservationV1};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, DurableObjectIdentityV1, PathComponentMode,
};
use crate::checked_artifact::protocol::{
    ActionDigestV1, BootstrapComponentOrdinalV1, BootstrapOrdinalV1, OwnershipMarkerV1,
    RecordDigestV1,
};

pub(in crate::checked_artifact) struct InstalledManagedComponentV1 {
    provider: ProviderBinding,
    intent_id: [u8; 32],
    action: ActionDigestV1,
    reservation: RecordDigestV1,
    bootstrap_ordinal: BootstrapOrdinalV1,
    component_ordinal: BootstrapComponentOrdinalV1,
    staging_leaf: AsciiComponent,
    final_leaf: AsciiComponent,
    marker: OwnershipMarkerV1,
    marker_object_identity: DurableObjectIdentityV1,
    installed_identity: DurableObjectIdentityV1,
    installed_mode: PathComponentMode,
    installed_path: CanonicalPathIdentityV1,
}

pub(in crate::checked_artifact) struct RetiredManagedMarkerV1 {
    provider: ProviderBinding,
    intent_id: [u8; 32],
    action: ActionDigestV1,
    reservation: RecordDigestV1,
    bootstrap_ordinal: BootstrapOrdinalV1,
    component_ordinal: BootstrapComponentOrdinalV1,
    marker_retirement_leaf: AsciiComponent,
    marker: OwnershipMarkerV1,
    retired_marker_identity: DurableObjectIdentityV1,
    installed_parent_identity: DurableObjectIdentityV1,
    installed_parent_mode: PathComponentMode,
    installed_parent_path: CanonicalPathIdentityV1,
}

macro_rules! binding_getters {
    ($type:ident) => {
        impl $type {
            pub(super) const fn provider_binding(&self) -> ProviderBinding {
                self.provider
            }

            pub(in crate::checked_artifact) const fn intent_id(&self) -> [u8; 32] {
                self.intent_id
            }

            pub(in crate::checked_artifact) const fn action_digest(&self) -> ActionDigestV1 {
                self.action
            }

            pub(in crate::checked_artifact) const fn reservation_digest(&self) -> RecordDigestV1 {
                self.reservation
            }

            pub(in crate::checked_artifact) const fn bootstrap_ordinal(
                &self,
            ) -> BootstrapOrdinalV1 {
                self.bootstrap_ordinal
            }

            pub(in crate::checked_artifact) const fn component_ordinal(
                &self,
            ) -> BootstrapComponentOrdinalV1 {
                self.component_ordinal
            }

            pub(in crate::checked_artifact) fn marker(&self) -> &OwnershipMarkerV1 {
                &self.marker
            }
        }
    };
}

binding_getters!(InstalledManagedComponentV1);
binding_getters!(RetiredManagedMarkerV1);

impl InstalledManagedComponentV1 {
    pub(in crate::checked_artifact) fn staging_leaf(&self) -> &AsciiComponent {
        &self.staging_leaf
    }

    pub(in crate::checked_artifact) fn final_leaf(&self) -> &AsciiComponent {
        &self.final_leaf
    }

    pub(in crate::checked_artifact) fn installed_identity(&self) -> &DurableObjectIdentityV1 {
        &self.installed_identity
    }

    pub(in crate::checked_artifact) fn marker_object_identity(&self) -> &DurableObjectIdentityV1 {
        &self.marker_object_identity
    }

    pub(in crate::checked_artifact) const fn installed_mode(&self) -> PathComponentMode {
        self.installed_mode
    }

    pub(in crate::checked_artifact) fn installed_path(&self) -> &CanonicalPathIdentityV1 {
        &self.installed_path
    }
}

impl RetiredManagedMarkerV1 {
    pub(in crate::checked_artifact) fn marker_retirement_leaf(&self) -> &AsciiComponent {
        &self.marker_retirement_leaf
    }

    pub(in crate::checked_artifact) fn retired_marker_identity(&self) -> &DurableObjectIdentityV1 {
        &self.retired_marker_identity
    }

    pub(in crate::checked_artifact) fn installed_parent_identity(
        &self,
    ) -> &DurableObjectIdentityV1 {
        &self.installed_parent_identity
    }

    pub(in crate::checked_artifact) const fn installed_parent_mode(&self) -> PathComponentMode {
        self.installed_parent_mode
    }

    pub(in crate::checked_artifact) fn installed_parent_path(&self) -> &CanonicalPathIdentityV1 {
        &self.installed_parent_path
    }
}

pub(super) fn installed(observation: ManagedInstallObservationV1) -> InstalledManagedComponentV1 {
    let (binding, marker, marker_object_identity, identity, mode, path) =
        observation.into_evidence_parts();
    InstalledManagedComponentV1 {
        provider: binding.provider,
        intent_id: binding.intent_id,
        action: binding.action,
        reservation: binding.reservation,
        bootstrap_ordinal: binding.bootstrap_ordinal,
        component_ordinal: binding.component_ordinal,
        staging_leaf: binding.staging_leaf,
        final_leaf: binding.final_leaf,
        marker,
        marker_object_identity,
        installed_identity: identity,
        installed_mode: mode,
        installed_path: path,
    }
}

pub(super) fn retired_marker(
    observation: ManagedMarkerRetirementObservationV1,
) -> RetiredManagedMarkerV1 {
    let (binding, marker, retired_marker_identity, installed_parent_identity, mode, path) =
        observation.into_evidence_parts();
    RetiredManagedMarkerV1 {
        provider: binding.provider,
        intent_id: binding.intent_id,
        action: binding.action,
        reservation: binding.reservation,
        bootstrap_ordinal: binding.bootstrap_ordinal,
        component_ordinal: binding.component_ordinal,
        marker_retirement_leaf: binding.marker_retirement_leaf,
        marker,
        retired_marker_identity,
        installed_parent_identity,
        installed_parent_mode: mode,
        installed_parent_path: path,
    }
}
