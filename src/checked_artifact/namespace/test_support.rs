//! Test-only capability issuance and legacy contract compatibility.

use super::backend::{BackendIssuer, ProviderBinding, RawNamespaceBackend};
use super::managed::{
    ManagedInstallObservationV1, ManagedInstallRequestV1, ManagedMarkerRetirementObservationV1,
    ManagedMarkerRetirementRequestV1,
};
use super::*;
use crate::checked_artifact::capability::{CanonicalPathIdentityV1, DurableObjectIdentityV1};
use crate::checked_artifact::protocol::ActionCapacityReservationV1;
use crate::checked_artifact::protocol::OwnershipMarkerV1;

pub(in crate::checked_artifact) fn retained_directory<DirectoryHandle, Identity, PathProfile>(
    handle: DirectoryHandle,
    identity: Identity,
    path_profile: PathProfile,
) -> RetainedDirectory<DirectoryHandle, Identity, PathProfile> {
    BackendIssuer::new(ProviderBinding::owner_new([0; 32])).retained_directory(
        handle,
        identity,
        path_profile,
    )
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
    let issuer = BackendIssuer::new(parent.provider());
    issuer.retained_object_from_parent(parent, leaf, handle, identity, kind)
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
    let issuer = BackendIssuer::new(parent.provider());
    issuer.reserved_slot(parent, leaf, binding)
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
    let issuer = BackendIssuer::new(parent.provider());
    issuer.reserved_retirement_slot(parent, leaf, binding)
}

pub(in crate::checked_artifact) fn published_identity<Identity>(
    identity: Identity,
) -> PublishedIdentity<Identity> {
    BackendIssuer::new(ProviderBinding::owner_new([0; 32])).published(identity)
}

pub(in crate::checked_artifact) fn retired_identity<Identity>(
    identity: Identity,
) -> RetiredIdentity<Identity> {
    BackendIssuer::new(ProviderBinding::owner_new([0; 32])).retired(identity)
}

pub(in crate::checked_artifact) fn durable_namespace() -> DurableNamespace {
    BackendIssuer::new(ProviderBinding::owner_new([0; 32])).durable()
}

/// Legacy test-only backend shape retained while old characterization tests
/// are migrated. It is absent from non-test builds.
pub(in crate::checked_artifact) trait NamespaceProtocol {
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

// Compatibility for pre-remediation interface characterization. It makes the
// legacy test backend usable only for schedule/role derivation; any attempt to
// forward a raw operation fails closed. Non-test builds have no such adapter.
impl<Protocol: NamespaceProtocol> RawNamespaceBackend for Protocol {
    type DirectoryHandle = Protocol::DirectoryHandle;
    type ObjectHandle = Protocol::ObjectHandle;
    type Identity = Protocol::Identity;
    type PathProfile = Protocol::PathProfile;

    fn provider_binding(&self) -> ProviderBinding {
        ProviderBinding::owner_new([0; 32])
    }

    fn revalidate_action_directory(
        &mut self,
        _expected_identity: &DurableObjectIdentityV1,
        _expected_reservation: RecordDigestV1,
    ) -> Result<(), CheckedFsError> {
        Err(legacy_forwarding_error())
    }

    fn publish_no_replace(
        &mut self,
        _source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        _destination: &ActionDestination,
    ) -> Result<PublishedIdentity<Self::Identity>, CheckedFsError> {
        Err(legacy_forwarding_error())
    }

    fn retire_exact(
        &mut self,
        _source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        _destination: &ActionDestination,
    ) -> Result<RetiredIdentity<Self::Identity>, CheckedFsError> {
        Err(legacy_forwarding_error())
    }

    fn barrier(
        &mut self,
        _parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        _ordinal: BarrierOrdinalV1,
    ) -> Result<DurableNamespace, CheckedFsError> {
        Err(legacy_forwarding_error())
    }
}

fn legacy_forwarding_error() -> CheckedFsError {
    CheckedFsError::ambiguous(
        "legacy namespace test backend",
        "raw forwarding is unavailable",
    )
}

impl<Implementation> ActionNamespace<Implementation> {
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
    ) -> ReservedNamespaceSlot<DirectoryHandle, Identity, CanonicalPathIdentityV1, RecordDigestV1>
    {
        let issuer = BackendIssuer::new(parent.provider());
        issuer.reserved_slot(
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
    ) -> ReservedRetirementSlot<DirectoryHandle, Identity, CanonicalPathIdentityV1, RecordDigestV1>
    {
        let issuer = BackendIssuer::new(parent.provider());
        issuer.reserved_retirement_slot(
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum RecordingNamespaceEvent {
    Publish(Vec<u8>),
    Retire(Vec<u8>),
    Barrier(usize),
}

pub(in crate::checked_artifact) struct RecordingNamespaceBackend {
    provider: ProviderBinding,
    action_identity: DurableObjectIdentityV1,
    events: Vec<RecordingNamespaceEvent>,
    installed_observation: Option<ManagedInstallFacts>,
    retired_marker_observation: Option<ManagedRetirementFacts>,
}

struct ManagedInstallFacts {
    marker: OwnershipMarkerV1,
    marker_object_identity: DurableObjectIdentityV1,
    installed_identity: DurableObjectIdentityV1,
    installed_mode: crate::checked_artifact::capability::PathComponentMode,
    installed_path: CanonicalPathIdentityV1,
}

struct ManagedRetirementFacts {
    marker: OwnershipMarkerV1,
    retired_marker_identity: DurableObjectIdentityV1,
    installed_parent_identity: DurableObjectIdentityV1,
    installed_parent_mode: crate::checked_artifact::capability::PathComponentMode,
    installed_parent_path: CanonicalPathIdentityV1,
}

pub(in crate::checked_artifact) fn recording_backend(
    provider: [u8; 32],
    action_identity: DurableObjectIdentityV1,
) -> RecordingNamespaceBackend {
    RecordingNamespaceBackend {
        provider: ProviderBinding::owner_new(provider),
        action_identity,
        events: Vec::new(),
        installed_observation: None,
        retired_marker_observation: None,
    }
}

pub(in crate::checked_artifact) fn retained_directory_for<
    DirectoryHandle,
    Identity,
    PathProfile,
>(
    backend: &RecordingNamespaceBackend,
    handle: DirectoryHandle,
    identity: Identity,
    path_profile: PathProfile,
) -> RetainedDirectory<DirectoryHandle, Identity, PathProfile> {
    BackendIssuer::new(backend.provider).retained_directory(handle, identity, path_profile)
}

#[allow(
    clippy::too_many_arguments,
    reason = "test retained object binds every observed fact"
)]
pub(in crate::checked_artifact) fn retained_object_for<
    DirectoryHandle,
    ObjectHandle,
    Identity,
    PathProfile,
>(
    backend: &RecordingNamespaceBackend,
    parent_handle: DirectoryHandle,
    parent_identity: Identity,
    parent_path_profile: PathProfile,
    leaf: AsciiComponent,
    handle: ObjectHandle,
    identity: Identity,
    kind: NamespaceObjectKind,
) -> RetainedNamespaceObject<DirectoryHandle, ObjectHandle, Identity, PathProfile> {
    BackendIssuer::new(backend.provider).retained_object(
        parent_handle,
        parent_identity,
        parent_path_profile,
        leaf,
        handle,
        identity,
        kind,
    )
}

pub(in crate::checked_artifact) fn barrier_target<DirectoryHandle, Identity, PathProfile>(
    backend: &RecordingNamespaceBackend,
    parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
    leaf: AsciiComponent,
    reservation: &ActionCapacityReservationV1,
    ordinal: usize,
) -> BarrierTarget<DirectoryHandle, Identity, PathProfile> {
    assert_eq!(parent.provider(), backend.provider);
    BackendIssuer::new(backend.provider).barrier_target(
        parent,
        leaf,
        reservation.action_digest(),
        reservation.record_digest(),
        BarrierOrdinalV1::new(ordinal).unwrap(),
    )
}

pub(in crate::checked_artifact) fn bootstrap_target<DirectoryHandle, Identity, PathProfile>(
    backend: &RecordingNamespaceBackend,
    parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
    reservation: &ActionCapacityReservationV1,
    component_ordinal: usize,
    final_leaf: AsciiComponent,
) -> Result<BootstrapTarget<DirectoryHandle, Identity, PathProfile>, CheckedFsError> {
    assert_eq!(parent.provider(), backend.provider);
    BackendIssuer::new(backend.provider).bootstrap_target(
        parent,
        reservation.action_digest(),
        reservation.record_digest(),
        component_ordinal,
        final_leaf,
    )
}

pub(in crate::checked_artifact) fn backend_events(
    namespace: &ActionNamespace<RecordingNamespaceBackend>,
) -> &[RecordingNamespaceEvent] {
    &namespace.backend.events
}

pub(in crate::checked_artifact) fn seed_installed_component_observation(
    backend: &mut RecordingNamespaceBackend,
    marker: OwnershipMarkerV1,
    marker_object_identity: DurableObjectIdentityV1,
    installed_identity: DurableObjectIdentityV1,
    installed_mode: crate::checked_artifact::capability::PathComponentMode,
    installed_path: CanonicalPathIdentityV1,
) {
    backend.installed_observation = Some(ManagedInstallFacts {
        marker,
        marker_object_identity,
        installed_identity,
        installed_mode,
        installed_path,
    });
}

pub(in crate::checked_artifact) fn seed_retired_marker_observation(
    backend: &mut RecordingNamespaceBackend,
    marker: OwnershipMarkerV1,
    retired_marker_identity: DurableObjectIdentityV1,
    installed_parent_identity: DurableObjectIdentityV1,
    installed_parent_mode: crate::checked_artifact::capability::PathComponentMode,
    installed_parent_path: CanonicalPathIdentityV1,
) {
    backend.retired_marker_observation = Some(ManagedRetirementFacts {
        marker,
        retired_marker_identity,
        installed_parent_identity,
        installed_parent_mode,
        installed_parent_path,
    });
}

impl RawNamespaceBackend for RecordingNamespaceBackend {
    type DirectoryHandle = u8;
    type ObjectHandle = u8;
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
        destination: &ActionDestination,
    ) -> Result<PublishedIdentity<Self::Identity>, CheckedFsError> {
        let _ = (destination.leaf(), destination.reservation());
        self.events.push(RecordingNamespaceEvent::Publish(
            destination.leaf().as_bytes().to_vec(),
        ));
        Ok(BackendIssuer::new(self.provider).published(source.identity().clone()))
    }

    fn retire_exact(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        destination: &ActionDestination,
    ) -> Result<RetiredIdentity<Self::Identity>, CheckedFsError> {
        let _ = (destination.leaf(), destination.reservation());
        self.events.push(RecordingNamespaceEvent::Retire(
            destination.leaf().as_bytes().to_vec(),
        ));
        Ok(BackendIssuer::new(self.provider).retired(source.identity().clone()))
    }

    fn install_managed_component(
        &mut self,
        _source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        destination: &ActionDestination,
        request: &ManagedInstallRequestV1,
    ) -> Result<ManagedInstallObservationV1, CheckedFsError> {
        self.events.push(RecordingNamespaceEvent::Publish(
            destination.leaf().as_bytes().to_vec(),
        ));
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
        _source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        destination: &ActionDestination,
        request: &ManagedMarkerRetirementRequestV1,
    ) -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError> {
        self.events.push(RecordingNamespaceEvent::Retire(
            destination.leaf().as_bytes().to_vec(),
        ));
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
        ordinal: BarrierOrdinalV1,
    ) -> Result<DurableNamespace, CheckedFsError> {
        self.events
            .push(RecordingNamespaceEvent::Barrier(ordinal.index()));
        Ok(BackendIssuer::new(self.provider).durable())
    }
}
