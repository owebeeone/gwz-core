//! Raw platform namespace seam.
//!
//! This module is deliberately private below `namespace`. Platform providers
//! are children of `namespace`; checked-artifact consumers cannot name this
//! trait or its issuer.

use super::super::capability::{AsciiComponent, CheckedFsError, DurableObjectIdentityV1};
use super::super::protocol::{BarrierOrdinalV1, RecordDigestV1};
use crate::checked_artifact::capability::{CanonicalPathIdentityV1, PathComponentMode};
use crate::checked_artifact::protocol::OwnershipMarkerV1;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::checked_artifact) struct ProviderBinding([u8; 32]);

impl ProviderBinding {
    pub(super) const fn owner_new(value: [u8; 32]) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum NamespaceObjectKind {
    RegularFile,
    Directory,
}

pub(in crate::checked_artifact) struct RetainedDirectory<DirectoryHandle, Identity, PathProfile> {
    handle: DirectoryHandle,
    identity: Identity,
    path_profile: PathProfile,
    provider: ProviderBinding,
}

impl<DirectoryHandle, Identity, PathProfile>
    RetainedDirectory<DirectoryHandle, Identity, PathProfile>
{
    pub(in crate::checked_artifact) fn handle(&self) -> &DirectoryHandle {
        &self.handle
    }

    pub(in crate::checked_artifact) fn identity(&self) -> &Identity {
        &self.identity
    }

    pub(in crate::checked_artifact) fn path_profile(&self) -> &PathProfile {
        &self.path_profile
    }

    pub(super) const fn provider(&self) -> ProviderBinding {
        self.provider
    }
}

pub(in crate::checked_artifact) struct RetainedNamespaceObject<
    DirectoryHandle,
    ObjectHandle,
    Identity,
    PathProfile,
> {
    parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
    leaf: AsciiComponent,
    handle: ObjectHandle,
    identity: Identity,
    kind: NamespaceObjectKind,
}

impl<DirectoryHandle, ObjectHandle, Identity, PathProfile>
    RetainedNamespaceObject<DirectoryHandle, ObjectHandle, Identity, PathProfile>
{
    pub(in crate::checked_artifact) fn parent(
        &self,
    ) -> &RetainedDirectory<DirectoryHandle, Identity, PathProfile> {
        &self.parent
    }

    pub(in crate::checked_artifact) fn leaf(&self) -> &AsciiComponent {
        &self.leaf
    }

    pub(in crate::checked_artifact) fn handle(&self) -> &ObjectHandle {
        &self.handle
    }

    pub(in crate::checked_artifact) fn identity(&self) -> &Identity {
        &self.identity
    }

    pub(in crate::checked_artifact) fn kind(&self) -> NamespaceObjectKind {
        self.kind
    }

    pub(super) const fn provider(&self) -> ProviderBinding {
        self.parent.provider()
    }
}

pub(in crate::checked_artifact) struct ReservedNamespaceSlot<
    DirectoryHandle,
    Identity,
    PathProfile,
    Binding,
> {
    parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
    leaf: AsciiComponent,
    binding: Binding,
}

impl<DirectoryHandle, Identity, PathProfile, Binding>
    ReservedNamespaceSlot<DirectoryHandle, Identity, PathProfile, Binding>
{
    pub(in crate::checked_artifact) fn parent(
        &self,
    ) -> &RetainedDirectory<DirectoryHandle, Identity, PathProfile> {
        &self.parent
    }

    pub(in crate::checked_artifact) fn leaf(&self) -> &AsciiComponent {
        &self.leaf
    }

    pub(in crate::checked_artifact) fn binding(&self) -> &Binding {
        &self.binding
    }
}

pub(in crate::checked_artifact) struct ReservedRetirementSlot<
    DirectoryHandle,
    Identity,
    PathProfile,
    Binding,
>(ReservedNamespaceSlot<DirectoryHandle, Identity, PathProfile, Binding>);

impl<DirectoryHandle, Identity, PathProfile, Binding>
    ReservedRetirementSlot<DirectoryHandle, Identity, PathProfile, Binding>
{
    pub(in crate::checked_artifact) fn slot(
        &self,
    ) -> &ReservedNamespaceSlot<DirectoryHandle, Identity, PathProfile, Binding> {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct PublishedIdentity<Identity>(Identity);

impl<Identity> PublishedIdentity<Identity> {
    pub(in crate::checked_artifact) fn identity(&self) -> &Identity {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct RetiredIdentity<Identity>(Identity);

impl<Identity> RetiredIdentity<Identity> {
    pub(in crate::checked_artifact) fn identity(&self) -> &Identity {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct DurableNamespace {
    _sealed: (),
}

pub(in crate::checked_artifact) struct ActionDestination {
    leaf: AsciiComponent,
    reservation: RecordDigestV1,
}

impl ActionDestination {
    pub(super) fn new(leaf: AsciiComponent, reservation: RecordDigestV1) -> Self {
        Self { leaf, reservation }
    }

    pub(super) fn leaf(&self) -> &AsciiComponent {
        &self.leaf
    }

    pub(super) const fn reservation(&self) -> RecordDigestV1 {
        self.reservation
    }
}

/// Issuer available only to namespace provider children. It permits a real
/// platform adapter to create retained capabilities and success proofs without
/// exposing those constructors to checked-artifact consumers.
pub(in crate::checked_artifact) struct BackendIssuer {
    provider: ProviderBinding,
}

impl BackendIssuer {
    pub(super) const fn new(provider: ProviderBinding) -> Self {
        Self { provider }
    }

    pub(super) fn retained_directory<DirectoryHandle, Identity, PathProfile>(
        &self,
        handle: DirectoryHandle,
        identity: Identity,
        path_profile: PathProfile,
    ) -> RetainedDirectory<DirectoryHandle, Identity, PathProfile> {
        RetainedDirectory {
            handle,
            identity,
            path_profile,
            provider: self.provider,
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "retained object binds every observed fact"
    )]
    pub(super) fn retained_object<DirectoryHandle, ObjectHandle, Identity, PathProfile>(
        &self,
        parent_handle: DirectoryHandle,
        parent_identity: Identity,
        parent_path_profile: PathProfile,
        leaf: AsciiComponent,
        handle: ObjectHandle,
        identity: Identity,
        kind: NamespaceObjectKind,
    ) -> RetainedNamespaceObject<DirectoryHandle, ObjectHandle, Identity, PathProfile> {
        RetainedNamespaceObject {
            parent: self.retained_directory(parent_handle, parent_identity, parent_path_profile),
            leaf,
            handle,
            identity,
            kind,
        }
    }

    pub(super) fn retained_object_from_parent<
        DirectoryHandle,
        ObjectHandle,
        Identity,
        PathProfile,
    >(
        &self,
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        handle: ObjectHandle,
        identity: Identity,
        kind: NamespaceObjectKind,
    ) -> RetainedNamespaceObject<DirectoryHandle, ObjectHandle, Identity, PathProfile> {
        debug_assert_eq!(parent.provider(), self.provider);
        RetainedNamespaceObject {
            parent,
            leaf,
            handle,
            identity,
            kind,
        }
    }

    pub(super) fn reserved_slot<DirectoryHandle, Identity, PathProfile, Binding>(
        &self,
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        binding: Binding,
    ) -> ReservedNamespaceSlot<DirectoryHandle, Identity, PathProfile, Binding> {
        ReservedNamespaceSlot {
            parent,
            leaf,
            binding,
        }
    }

    pub(super) fn reserved_retirement_slot<DirectoryHandle, Identity, PathProfile, Binding>(
        &self,
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        binding: Binding,
    ) -> ReservedRetirementSlot<DirectoryHandle, Identity, PathProfile, Binding> {
        ReservedRetirementSlot(self.reserved_slot(parent, leaf, binding))
    }

    pub(super) fn barrier_target<DirectoryHandle, Identity, PathProfile>(
        &self,
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        action: super::ActionDigestV1,
        reservation: RecordDigestV1,
        ordinal: BarrierOrdinalV1,
    ) -> super::BarrierTarget<DirectoryHandle, Identity, PathProfile> {
        debug_assert_eq!(parent.provider(), self.provider);
        super::BarrierTarget {
            parent,
            leaf,
            action,
            reservation,
            ordinal,
        }
    }

    pub(super) fn bootstrap_target<DirectoryHandle, Identity, PathProfile>(
        &self,
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        action: super::ActionDigestV1,
        reservation: RecordDigestV1,
        global_component_ordinal: usize,
        final_leaf: AsciiComponent,
    ) -> Result<super::BootstrapTarget<DirectoryHandle, Identity, PathProfile>, CheckedFsError>
    {
        debug_assert_eq!(parent.provider(), self.provider);
        let staging_leaf =
            super::super::protocol::managed_staging_name(action, global_component_ordinal)
                .map_err(|_| {
                    CheckedFsError::ambiguous(
                        "bootstrap target",
                        "component ordinal is not scheduled",
                    )
                })?;
        Ok(super::BootstrapTarget {
            parent,
            staging_leaf,
            final_leaf,
            action,
            reservation,
            component_ordinal: global_component_ordinal,
        })
    }

    pub(super) fn published<Identity>(&self, identity: Identity) -> PublishedIdentity<Identity> {
        PublishedIdentity(identity)
    }

    pub(super) fn installed_managed_component<DirectoryHandle, Identity, PathProfile>(
        &self,
        slots: &super::BootstrapComponentSlots<DirectoryHandle, Identity, PathProfile>,
        marker: OwnershipMarkerV1,
        marker_object_identity: DurableObjectIdentityV1,
        installed_identity: DurableObjectIdentityV1,
        installed_mode: PathComponentMode,
        installed_path: CanonicalPathIdentityV1,
    ) -> Result<super::InstalledManagedComponentV1, CheckedFsError> {
        if slots.target.parent.provider() != self.provider {
            return Err(CheckedFsError::ambiguous(
                "managed component evidence",
                "provider binding mismatch",
            ));
        }
        Ok(super::evidence::installed(
            slots,
            marker,
            marker_object_identity,
            installed_identity,
            installed_mode,
            installed_path,
        ))
    }

    pub(super) fn retired_managed_marker<DirectoryHandle, Identity, PathProfile>(
        &self,
        slots: &super::BootstrapComponentSlots<DirectoryHandle, Identity, PathProfile>,
        marker: OwnershipMarkerV1,
        retired_marker_identity: DurableObjectIdentityV1,
        installed_parent_identity: DurableObjectIdentityV1,
        installed_parent_mode: PathComponentMode,
        installed_parent_path: CanonicalPathIdentityV1,
    ) -> Result<super::RetiredManagedMarkerV1, CheckedFsError> {
        if slots.target.parent.provider() != self.provider {
            return Err(CheckedFsError::ambiguous(
                "retired marker evidence",
                "provider binding mismatch",
            ));
        }
        Ok(super::evidence::retired_marker(
            slots,
            marker,
            retired_marker_identity,
            installed_parent_identity,
            installed_parent_mode,
            installed_parent_path,
        ))
    }

    pub(super) fn retired<Identity>(&self, identity: Identity) -> RetiredIdentity<Identity> {
        RetiredIdentity(identity)
    }

    pub(super) const fn durable(&self) -> DurableNamespace {
        DurableNamespace { _sealed: () }
    }
}

pub(in crate::checked_artifact) trait RawNamespaceBackend {
    type DirectoryHandle;
    type ObjectHandle;
    type Identity: Clone + Eq;
    type PathProfile;

    fn provider_binding(&self) -> ProviderBinding;

    fn revalidate_action_directory(
        &mut self,
        expected_identity: &DurableObjectIdentityV1,
        expected_reservation: RecordDigestV1,
    ) -> Result<(), CheckedFsError>;

    fn publish_no_replace(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        destination: &ActionDestination,
    ) -> Result<PublishedIdentity<Self::Identity>, CheckedFsError>;

    fn retire_exact(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        destination: &ActionDestination,
    ) -> Result<RetiredIdentity<Self::Identity>, CheckedFsError>;

    fn barrier(
        &mut self,
        parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        ordinal: BarrierOrdinalV1,
    ) -> Result<DurableNamespace, CheckedFsError>;
}

pub(in crate::checked_artifact) trait SealedActionNamespace {}
