//! Reservation-bound namespace durability contracts.
//!
//! Implementations are intentionally absent from this interface checkpoint.
//! The types prevent path-only publication and require callers to provide the
//! precomputed logical barrier ordinal chosen by the protocol schedule.

use super::capability::{AsciiComponent, CanonicalPathIdentityV1, CheckedFsError};
use super::protocol::ActionSlotV1;
use super::protocol::{AdmittedActionV1, BarrierOrdinalV1, ScheduleErrorV1};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NamespaceObjectKind {
    RegularFile,
    Directory,
}

pub(super) struct RetainedDirectory<DirectoryHandle, Identity, PathProfile> {
    handle: DirectoryHandle,
    identity: Identity,
    path_profile: PathProfile,
}

impl<DirectoryHandle, Identity, PathProfile>
    RetainedDirectory<DirectoryHandle, Identity, PathProfile>
{
    fn new(handle: DirectoryHandle, identity: Identity, path_profile: PathProfile) -> Self {
        Self {
            handle,
            identity,
            path_profile,
        }
    }

    pub(in crate::checked_artifact) fn handle(&self) -> &DirectoryHandle {
        &self.handle
    }

    pub(in crate::checked_artifact) fn identity(&self) -> &Identity {
        &self.identity
    }

    pub(in crate::checked_artifact) fn path_profile(&self) -> &PathProfile {
        &self.path_profile
    }
}

pub(super) struct RetainedNamespaceObject<DirectoryHandle, ObjectHandle, Identity, PathProfile> {
    parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
    leaf: AsciiComponent,
    handle: ObjectHandle,
    identity: Identity,
    kind: NamespaceObjectKind,
}

impl<DirectoryHandle, ObjectHandle, Identity, PathProfile>
    RetainedNamespaceObject<DirectoryHandle, ObjectHandle, Identity, PathProfile>
{
    fn new(
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        handle: ObjectHandle,
        identity: Identity,
        kind: NamespaceObjectKind,
    ) -> Self {
        Self {
            parent,
            leaf,
            handle,
            identity,
            kind,
        }
    }

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
}

pub(super) struct ReservedNamespaceSlot<DirectoryHandle, Identity, PathProfile, Binding> {
    parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
    leaf: AsciiComponent,
    binding: Binding,
}

impl<DirectoryHandle, Identity, PathProfile, Binding>
    ReservedNamespaceSlot<DirectoryHandle, Identity, PathProfile, Binding>
{
    fn new(
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        binding: Binding,
    ) -> Self {
        Self {
            parent,
            leaf,
            binding,
        }
    }

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

pub(super) struct ReservedRetirementSlot<DirectoryHandle, Identity, PathProfile, Binding>(
    ReservedNamespaceSlot<DirectoryHandle, Identity, PathProfile, Binding>,
);

impl<DirectoryHandle, Identity, PathProfile, Binding>
    ReservedRetirementSlot<DirectoryHandle, Identity, PathProfile, Binding>
{
    fn new(
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        binding: Binding,
    ) -> Self {
        Self(ReservedNamespaceSlot::new(parent, leaf, binding))
    }

    pub(in crate::checked_artifact) fn slot(
        &self,
    ) -> &ReservedNamespaceSlot<DirectoryHandle, Identity, PathProfile, Binding> {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PublishedIdentity<Identity>(Identity);

impl<Identity> PublishedIdentity<Identity> {
    fn new(identity: Identity) -> Self {
        Self(identity)
    }

    pub(in crate::checked_artifact) fn identity(&self) -> &Identity {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RetiredIdentity<Identity>(Identity);

impl<Identity> RetiredIdentity<Identity> {
    fn new(identity: Identity) -> Self {
        Self(identity)
    }

    pub(in crate::checked_artifact) fn identity(&self) -> &Identity {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DurableNamespace {
    _sealed: (),
}

/// The only namespace capability passed to checked-artifact consumers. It is
/// issued from an admitted action and owns the exact reservation binding used
/// to derive slots and barrier ordinals.
pub(super) struct ActionNamespace<Implementation> {
    implementation: Implementation,
    admitted_action: AdmittedActionV1,
}

impl<Implementation> ActionNamespace<Implementation> {
    pub(super) fn from_admitted(
        implementation: Implementation,
        admitted_action: AdmittedActionV1,
    ) -> Self {
        Self {
            implementation,
            admitted_action,
        }
    }

    pub(super) fn admitted_action(&self) -> &AdmittedActionV1 {
        &self.admitted_action
    }

    pub(super) fn implementation(&self) -> &Implementation {
        &self.implementation
    }

    pub(super) fn implementation_mut(&mut self) -> &mut Implementation {
        &mut self.implementation
    }

    pub(super) fn scheduled_barrier_ordinal(
        &self,
        index: usize,
    ) -> Result<BarrierOrdinalV1, ScheduleErrorV1> {
        if index
            >= self
                .admitted_action
                .reservation()
                .schedule()
                .barrier_count()
        {
            return Err(ScheduleErrorV1::OutOfBounds);
        }
        BarrierOrdinalV1::new(index)
    }

    pub(super) fn reserve_action_slot<DirectoryHandle, Identity>(
        &self,
        parent: RetainedDirectory<DirectoryHandle, Identity, CanonicalPathIdentityV1>,
        slot: ActionSlotV1,
    ) -> ReservedNamespaceSlot<
        DirectoryHandle,
        Identity,
        CanonicalPathIdentityV1,
        super::protocol::RecordDigestV1,
    > {
        ReservedNamespaceSlot::new(
            parent,
            AsciiComponent::parse(
                slot.name(self.admitted_action.reservation().action_digest())
                    .as_bytes(),
            )
            .expect("fixed action slot name is valid"),
            self.admitted_action.reservation().record_digest(),
        )
    }

    pub(super) fn reserve_action_retirement_slot<DirectoryHandle, Identity>(
        &self,
        parent: RetainedDirectory<DirectoryHandle, Identity, CanonicalPathIdentityV1>,
        slot: ActionSlotV1,
    ) -> ReservedRetirementSlot<
        DirectoryHandle,
        Identity,
        CanonicalPathIdentityV1,
        super::protocol::RecordDigestV1,
    > {
        ReservedRetirementSlot::new(
            parent,
            AsciiComponent::parse(
                slot.name(self.admitted_action.reservation().action_digest())
                    .as_bytes(),
            )
            .expect("fixed action slot name is valid"),
            self.admitted_action.reservation().record_digest(),
        )
    }
}

impl DurableNamespace {
    fn new() -> Self {
        Self { _sealed: () }
    }
}

pub(super) trait NamespaceProtocol {
    type DirectoryHandle;
    type ObjectHandle;
    type Identity: Clone + Eq;
    type PathProfile;
    type ReservationBinding;
    type BarrierOrdinal: Clone + Eq;

    fn publish_no_replace(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        destination: &ReservedNamespaceSlot<
            Self::DirectoryHandle,
            Self::Identity,
            Self::PathProfile,
            Self::ReservationBinding,
        >,
    ) -> Result<PublishedIdentity<Self::Identity>, CheckedFsError>;

    fn retire_exact(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        destination: &ReservedRetirementSlot<
            Self::DirectoryHandle,
            Self::Identity,
            Self::PathProfile,
            Self::ReservationBinding,
        >,
    ) -> Result<RetiredIdentity<Self::Identity>, CheckedFsError>;

    fn barrier(
        &mut self,
        parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        ordinal: Self::BarrierOrdinal,
    ) -> Result<DurableNamespace, CheckedFsError>;
}

#[cfg(test)]
pub(super) mod test_support {
    use super::*;

    pub(in crate::checked_artifact) fn retained_directory<
        DirectoryHandle,
        Identity,
        PathProfile,
    >(
        handle: DirectoryHandle,
        identity: Identity,
        path_profile: PathProfile,
    ) -> RetainedDirectory<DirectoryHandle, Identity, PathProfile> {
        RetainedDirectory::new(handle, identity, path_profile)
    }

    pub(in crate::checked_artifact) fn retained_object<
        DirectoryHandle,
        ObjectHandle,
        Identity,
        PathProfile,
    >(
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        handle: ObjectHandle,
        identity: Identity,
        kind: NamespaceObjectKind,
    ) -> RetainedNamespaceObject<DirectoryHandle, ObjectHandle, Identity, PathProfile> {
        RetainedNamespaceObject::new(parent, leaf, handle, identity, kind)
    }

    pub(in crate::checked_artifact) fn reserved_slot<
        DirectoryHandle,
        Identity,
        PathProfile,
        Binding,
    >(
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        binding: Binding,
    ) -> ReservedNamespaceSlot<DirectoryHandle, Identity, PathProfile, Binding> {
        ReservedNamespaceSlot::new(parent, leaf, binding)
    }

    pub(in crate::checked_artifact) fn reserved_retirement_slot<
        DirectoryHandle,
        Identity,
        PathProfile,
        Binding,
    >(
        parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
        leaf: AsciiComponent,
        binding: Binding,
    ) -> ReservedRetirementSlot<DirectoryHandle, Identity, PathProfile, Binding> {
        ReservedRetirementSlot::new(parent, leaf, binding)
    }

    pub(in crate::checked_artifact) fn published_identity<Identity>(
        identity: Identity,
    ) -> PublishedIdentity<Identity> {
        PublishedIdentity::new(identity)
    }

    pub(in crate::checked_artifact) fn retired_identity<Identity>(
        identity: Identity,
    ) -> RetiredIdentity<Identity> {
        RetiredIdentity::new(identity)
    }

    pub(in crate::checked_artifact) fn durable_namespace() -> DurableNamespace {
        DurableNamespace::new()
    }
}
