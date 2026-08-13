//! Always-compiled provider shape proving owner-private issuers are usable by
//! a production namespace child without widening raw proof constructors.

#![allow(dead_code, reason = "R1 compile-only platform provider shape")]

use super::backend::{ActionDestination, BackendIssuer, ProviderBinding, RawNamespaceBackend};
use super::managed::{
    ManagedInstallObservationV1, ManagedInstallRequestV1, ManagedMarkerRetirementObservationV1,
    ManagedMarkerRetirementRequestV1,
};
use super::{
    ActionNamespace, BarrierSlots, BootstrapComponentSlots, BootstrapGenerationSlots,
    DurableNamespace, NamespaceObjectKind, PublishedIdentity, RetainedDirectory,
    RetainedNamespaceObject, RetiredIdentity,
};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
    PathComponentMode,
};
use crate::checked_artifact::protocol::{
    BarrierOrdinalV1, ManagedParentBootstrapIntentV1, OwnershipMarkerV1, RecordDigestV1,
};

struct ProductionShapedBackend {
    provider: ProviderBinding,
    action_identity: DurableObjectIdentityV1,
    installed_observation: Option<ManagedInstallFacts>,
    retired_marker_observation: Option<ManagedRetirementFacts>,
}

struct ManagedInstallFacts {
    marker: OwnershipMarkerV1,
    marker_object_identity: DurableObjectIdentityV1,
    installed_identity: DurableObjectIdentityV1,
    installed_mode: PathComponentMode,
    installed_path: CanonicalPathIdentityV1,
}

struct ManagedRetirementFacts {
    marker: OwnershipMarkerV1,
    retired_marker_identity: DurableObjectIdentityV1,
    installed_parent_identity: DurableObjectIdentityV1,
    installed_parent_mode: PathComponentMode,
    installed_parent_path: CanonicalPathIdentityV1,
}

impl ProductionShapedBackend {
    fn issuer(&self) -> BackendIssuer {
        BackendIssuer::new(self.provider)
    }

    fn retain_directory(
        &self,
        handle: u64,
        identity: DurableObjectIdentityV1,
        path: CanonicalPathIdentityV1,
    ) -> RetainedDirectory<u64, DurableObjectIdentityV1, CanonicalPathIdentityV1> {
        self.issuer().retained_directory(handle, identity, path)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "compile fixture mirrors retained facts"
    )]
    fn retain_object(
        &self,
        parent_handle: u64,
        parent_identity: DurableObjectIdentityV1,
        path: CanonicalPathIdentityV1,
        leaf: AsciiComponent,
        handle: u64,
        identity: DurableObjectIdentityV1,
        kind: NamespaceObjectKind,
    ) -> RetainedNamespaceObject<u64, u64, DurableObjectIdentityV1, CanonicalPathIdentityV1> {
        self.issuer().retained_object(
            parent_handle,
            parent_identity,
            path,
            leaf,
            handle,
            identity,
            kind,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "compile fixture exercises every sealed namespace role"
    )]
    fn forward_every_indexed_role(
        namespace: &mut ActionNamespace<Self>,
        barrier_scratch: &RetainedNamespaceObject<
            u64,
            u64,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        barrier_active: &RetainedNamespaceObject<
            u64,
            u64,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        anchor_alias: &RetainedNamespaceObject<
            u64,
            u64,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        barrier: &BarrierSlots<u64, DurableObjectIdentityV1, CanonicalPathIdentityV1>,
        bootstrap_scratch: &RetainedNamespaceObject<
            u64,
            u64,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        bootstrap_active: &RetainedNamespaceObject<
            u64,
            u64,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        generation: &BootstrapGenerationSlots,
        staging: &RetainedNamespaceObject<
            u64,
            u64,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        marker: &RetainedNamespaceObject<
            u64,
            u64,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        component: &BootstrapComponentSlots<u64, DurableObjectIdentityV1, CanonicalPathIdentityV1>,
        install_intent: &ManagedParentBootstrapIntentV1,
        retirement_intent: &ManagedParentBootstrapIntentV1,
    ) -> Result<(), CheckedFsError> {
        namespace.publish_barrier_intent(barrier_scratch, barrier)?;
        namespace.retire_barrier_intent(barrier_active, barrier)?;
        namespace.retire_barrier_target_alias(anchor_alias, barrier)?;
        namespace.publish_bootstrap_generation(bootstrap_scratch, generation)?;
        namespace.retire_bootstrap_generation(bootstrap_active, generation)?;
        namespace.install_bootstrap_component(staging, install_intent, component)?;
        namespace.recover_installed_bootstrap_component(install_intent, component)?;
        namespace.retire_bootstrap_marker(marker, retirement_intent, component)?;
        namespace.recover_retired_bootstrap_marker(retirement_intent, component)?;
        Ok(())
    }
}

impl RawNamespaceBackend for ProductionShapedBackend {
    type DirectoryHandle = u64;
    type ObjectHandle = u64;
    type Identity = DurableObjectIdentityV1;
    type PathProfile = CanonicalPathIdentityV1;

    fn provider_binding(&self) -> ProviderBinding {
        self.provider
    }

    fn revalidate_action_directory(
        &mut self,
        expected_identity: &DurableObjectIdentityV1,
        _expected_reservation: RecordDigestV1,
    ) -> Result<(), CheckedFsError> {
        if &self.action_identity == expected_identity {
            Ok(())
        } else {
            Err(CheckedFsError::ambiguous(
                "action namespace",
                "action directory identity changed",
            ))
        }
    }

    fn publish_no_replace(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        _destination: &ActionDestination,
    ) -> Result<PublishedIdentity<Self::Identity>, CheckedFsError> {
        Ok(self.issuer().published(source.identity().clone()))
    }

    fn retire_exact(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        _destination: &ActionDestination,
    ) -> Result<RetiredIdentity<Self::Identity>, CheckedFsError> {
        Ok(self.issuer().retired(source.identity().clone()))
    }

    fn install_managed_component(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        destination: &ActionDestination,
        request: &ManagedInstallRequestV1,
    ) -> Result<ManagedInstallObservationV1, CheckedFsError> {
        self.publish_no_replace(source, destination)?;
        self.observe_installed_managed_component(request)
    }

    fn observe_installed_managed_component(
        &mut self,
        request: &ManagedInstallRequestV1,
    ) -> Result<ManagedInstallObservationV1, CheckedFsError> {
        let facts = self.installed_observation.as_ref().ok_or_else(|| {
            CheckedFsError::ambiguous(
                "managed install observation",
                "installed state is not present",
            )
        })?;
        request.complete(
            self.provider,
            facts.marker.clone(),
            facts.marker_object_identity.clone(),
            facts.installed_identity.clone(),
            facts.installed_mode,
            facts.installed_path.clone(),
        )
    }

    fn retire_managed_marker(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        destination: &ActionDestination,
        request: &ManagedMarkerRetirementRequestV1,
    ) -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError> {
        self.retire_exact(source, destination)?;
        self.observe_retired_managed_marker(request)
    }

    fn observe_retired_managed_marker(
        &mut self,
        request: &ManagedMarkerRetirementRequestV1,
    ) -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError> {
        let facts = self.retired_marker_observation.as_ref().ok_or_else(|| {
            CheckedFsError::ambiguous(
                "managed marker retirement observation",
                "retired marker state is not present",
            )
        })?;
        request.complete(
            self.provider,
            facts.marker.clone(),
            facts.retired_marker_identity.clone(),
            facts.installed_parent_identity.clone(),
            facts.installed_parent_mode,
            facts.installed_parent_path.clone(),
        )
    }

    fn barrier(
        &mut self,
        _parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        _ordinal: BarrierOrdinalV1,
    ) -> Result<DurableNamespace, CheckedFsError> {
        Ok(self.issuer().durable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checked_artifact::capability::{CanonicalComponent, PathComponentMode};

    fn assert_backend<Backend: RawNamespaceBackend>() {}

    #[test]
    fn production_shaped_backend_uses_only_owner_private_issuers() {
        assert_backend::<ProductionShapedBackend>();
        let identity = DurableObjectIdentityV1::linux_ext4([1; 16], 1, vec![1]).unwrap();
        let path = CanonicalPathIdentityV1::new(vec![CanonicalComponent::new(
            AsciiComponent::parse(b"action").unwrap(),
            PathComponentMode::Sensitive,
        )])
        .unwrap();
        let provider = ProductionShapedBackend {
            provider: ProviderBinding::owner_new([2; 32]),
            action_identity: identity.clone(),
            installed_observation: None,
            retired_marker_observation: None,
        };
        let retained = provider.retain_directory(1, identity.clone(), path.clone());
        assert_eq!(retained.identity(), &identity);
        let object = provider.retain_object(
            1,
            identity.clone(),
            path,
            AsciiComponent::parse(b"source").unwrap(),
            2,
            identity,
            NamespaceObjectKind::RegularFile,
        );
        assert_eq!(object.handle(), &2);
    }
}
