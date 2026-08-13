//! Always-compiled provider shape proving owner-private issuers are usable by
//! a production namespace child without widening raw proof constructors.

#![allow(dead_code, reason = "R1 compile-only platform provider shape")]

use super::backend::{ActionDestination, BackendIssuer, ProviderBinding, RawNamespaceBackend};
use super::{
    DurableNamespace, NamespaceObjectKind, PublishedIdentity, RetainedDirectory,
    RetainedNamespaceObject, RetiredIdentity,
};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
};
use crate::checked_artifact::protocol::OwnershipMarkerV1;
use crate::checked_artifact::protocol::{BarrierOrdinalV1, RecordDigestV1};

struct ProductionShapedBackend {
    provider: ProviderBinding,
    action_identity: DurableObjectIdentityV1,
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

    fn issue_installed_component(
        &self,
        slots: &super::BootstrapComponentSlots<
            u64,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        marker: OwnershipMarkerV1,
        identity: DurableObjectIdentityV1,
        mode: crate::checked_artifact::capability::PathComponentMode,
        path: CanonicalPathIdentityV1,
    ) -> Result<super::InstalledManagedComponentV1, CheckedFsError> {
        self.issuer()
            .installed_managed_component(slots, marker, identity, mode, path)
    }

    fn issue_retired_marker(
        &self,
        slots: &super::BootstrapComponentSlots<
            u64,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        marker: OwnershipMarkerV1,
        identity: DurableObjectIdentityV1,
        mode: crate::checked_artifact::capability::PathComponentMode,
        path: CanonicalPathIdentityV1,
    ) -> Result<super::RetiredManagedMarkerV1, CheckedFsError> {
        self.issuer()
            .retired_managed_marker(slots, marker, identity, mode, path)
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
