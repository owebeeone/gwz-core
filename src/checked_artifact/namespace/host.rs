//! The retained-handle production `ActionNamespace` backend.
//!
//! R2-D Phase 2 Step 2.2 (`GwzM5-8R2D-Plan.md` §4): the real
//! `NamespaceProtocol`/`ActionNamespace` implementation of `publish_exact`,
//! `retire_exact` and `barrier` over the scheduled namespace roles of
//! `namespace/roles.rs`, routed through the sealed publication primitive family
//! and provenanced from the permit-retained root only
//! (`GwzM5-8R2C2PublicationAudit.md` :39-44).
//!
//! Three properties are structural here rather than advisory.
//!
//! * **One retained handle for the whole life of the backend.** The action
//!   directory is opened once, through a single identity-proved no-follow hop
//!   from the permit-retained completed catalog, and that one capability serves
//!   every observation, publication, retirement, barrier and revalidation.
//! * **Only a retained source can be published.** A namespace edge consumes the
//!   source this backend itself retained and still holds; a
//!   `RetainedNamespaceObject` that names a different leaf, a different
//!   identity, a different parent or a foreign handle is refused before any
//!   physical edge, and the retention is cleared by the edge that consumes it.
//! * **Consumers never receive a path or an OS handle.** The retained proofs
//!   carry [`ActionNamespaceHandleV1`], an opaque reservation-derived token;
//!   the real `Dir` never leaves the pre-catalog provider owner
//!   (ConsumerCheckpoint §9 :264-266).
//!
//! The four managed operations are *stated*, as the frozen seam requires
//! (`GwzM5-8R2DInterfaceFreeze.md` §3.2), and fail closed pending Step 2.3.
//! Plan §4 assigns them to Step 2.3, and §4.4 Class 1 assigns the managed
//! source-interior and managed destination recheck arms their edges (E15-E17)
//! need to Phase 2.3/3 — so implementing them here would need an arm §4.3 does
//! not assign to this step.

use super::backend::{
    ActionDestination, BackendIssuer, NamespaceObjectKind, ProviderBinding, RawNamespaceBackend,
};
use super::managed::{
    ManagedInstallObservationV1, ManagedInstallRequestV1, ManagedMarkerRetirementObservationV1,
    ManagedMarkerRetirementRequestV1,
};
use super::{
    ActionNamespace, DurableNamespace, PublishedIdentity, RetainedDirectory,
    RetainedNamespaceObject, RetiredIdentity, binding_error,
};
use crate::checked_artifact::capability::{
    ActionNamespaceEdgeV1, AsciiComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, ObservedNamespaceObjectV1, RetainedActionNamespaceV1,
};
use crate::checked_artifact::catalog::OpaqueRetainedCatalogV1;
use crate::checked_artifact::protocol::{
    AdmittedActionV1, BarrierOrdinalV1, ProtocolRecordKindV1, RecordDigestV1,
};

/// Opaque proof token carried by every retained namespace capability this
/// backend issues. It is derived from the admitted action's own digest, so it
/// mints no name and reveals no path; its only job is to make a capability
/// forged against another backend fail closed at `validate_operation`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ActionNamespaceHandleV1([u8; 32]);

/// The one namespace source this backend currently holds retained.
struct RetainedSourceV1 {
    leaf: AsciiComponent,
    kind: ProtocolRecordKindV1,
    observed: ObservedNamespaceObjectV1,
}

/// The production `RawNamespaceBackend` over one admitted action directory.
pub(in crate::checked_artifact) struct HostActionNamespaceV1 {
    retained: RetainedActionNamespaceV1,
    provider: ProviderBinding,
    handle: ActionNamespaceHandleV1,
    source: Option<RetainedSourceV1>,
}

/// The only constructor: an opaque retained catalog plus the admitted action it
/// admitted. There is no path argument and no caller-supplied handle, so a
/// namespace backend cannot exist over anything but a permit-retained,
/// reservation-bound action directory.
pub(in crate::checked_artifact) fn retain_action_namespace(
    catalog: &OpaqueRetainedCatalogV1<'_>,
    admitted: AdmittedActionV1,
) -> Result<ActionNamespace<HostActionNamespaceV1>, CheckedFsError> {
    let retained = catalog.retain_action_namespace(&admitted)?;
    let action = admitted.reservation().action_digest().bytes();
    let backend = HostActionNamespaceV1 {
        retained,
        provider: ProviderBinding::owner_new(admitted.reservation().record_digest().bytes()),
        handle: ActionNamespaceHandleV1(action),
        source: None,
    };
    Ok(ActionNamespace::from_admitted(backend, admitted))
}

/// Role wiring for the host backend. These live on the wrapper rather than on
/// the backend so a consumer holding only an `ActionNamespace` can retain a
/// scheduled source and bind a scheduled barrier without ever naming the
/// backend, a handle, or a path.
impl ActionNamespace<HostActionNamespaceV1> {
    /// The retained action directory as a consumer-visible proof.
    pub(in crate::checked_artifact) fn retained_parent(
        &self,
    ) -> RetainedDirectory<ActionNamespaceHandleV1, DurableObjectIdentityV1, CanonicalPathIdentityV1>
    {
        self.backend.retained_parent()
    }

    /// Retains the scheduled role named by `leaf` as this backend's one source.
    pub(in crate::checked_artifact) fn retain_scheduled_source(
        &mut self,
        leaf: AsciiComponent,
        kind: ProtocolRecordKindV1,
    ) -> Result<
        RetainedNamespaceObject<
            ActionNamespaceHandleV1,
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        CheckedFsError,
    > {
        self.backend.retain_source(leaf, kind)
    }

    /// Whether a scheduled role's row is resident in the action directory.
    pub(in crate::checked_artifact) fn scheduled_row_is_resident(
        &self,
        leaf: &AsciiComponent,
    ) -> bool {
        self.backend.row_is_resident(leaf)
    }

    /// Binds one scheduled barrier ordinal to a retained target inside this
    /// action directory. Both the ordinal and `target_leaf` are schedule-derived
    /// (`namespace/roles.rs`); nothing is minted here.
    pub(in crate::checked_artifact) fn scheduled_barrier_slots(
        &self,
        index: usize,
        target_leaf: AsciiComponent,
    ) -> Result<
        super::BarrierSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        CheckedFsError,
    > {
        let scheduled = self
            .scheduled_barrier(index)
            .map_err(|_| binding_error("barrier ordinal is not scheduled"))?;
        let reservation = self.admitted_action().reservation();
        let target = BackendIssuer::new(self.backend.provider).barrier_target(
            self.backend.retained_parent(),
            target_leaf,
            reservation.action_digest(),
            reservation.record_digest(),
            scheduled.ordinal(),
        );
        self.barrier_slots(scheduled, target)
            .map_err(|_| binding_error("barrier slots are not scheduled"))
    }
}

impl HostActionNamespaceV1 {
    fn issuer(&self) -> BackendIssuer {
        BackendIssuer::new(self.provider)
    }

    /// The retained action directory as a consumer-visible proof.
    pub(in crate::checked_artifact) fn retained_parent(
        &self,
    ) -> RetainedDirectory<ActionNamespaceHandleV1, DurableObjectIdentityV1, CanonicalPathIdentityV1>
    {
        self.issuer().retained_directory(
            self.handle,
            self.retained.identity().clone(),
            self.retained.path_profile().clone(),
        )
    }

    /// Retains one exact regular-file namespace source and returns its proof.
    /// This is edge `namespace.source_retain`; the observation it captures is
    /// the source association the sealed primitive re-verifies at the edge.
    pub(in crate::checked_artifact) fn retain_source(
        &mut self,
        leaf: AsciiComponent,
        kind: ProtocolRecordKindV1,
    ) -> Result<
        RetainedNamespaceObject<
            ActionNamespaceHandleV1,
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        CheckedFsError,
    > {
        let observed = self.retained.retain_source(&leaf, kind)?;
        let object = self.issuer().retained_object_from_parent(
            self.retained_parent(),
            leaf.clone(),
            self.handle,
            observed.identity().clone(),
            NamespaceObjectKind::RegularFile,
        );
        self.source = Some(RetainedSourceV1 {
            leaf,
            kind,
            observed,
        });
        Ok(object)
    }

    /// Whether a namespace row is currently resident. Restart resolution is
    /// Step 3.3's coordinator glue; this backend only reports what its one
    /// retained capability observes.
    pub(in crate::checked_artifact) fn row_is_resident(&self, leaf: &AsciiComponent) -> bool {
        self.retained.row_is_resident(leaf)
    }

    fn execute(
        &mut self,
        edge: ActionNamespaceEdgeV1,
        source: &RetainedNamespaceObject<
            ActionNamespaceHandleV1,
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        destination: &ActionDestination,
    ) -> Result<DurableObjectIdentityV1, CheckedFsError> {
        let Some(retained) = self.source.as_ref() else {
            return Err(binding_error(
                "namespace edge has no retained source capability",
            ));
        };
        if source.leaf() != &retained.leaf
            || source.identity() != retained.observed.identity()
            || source.handle() != &self.handle
            || source.parent().identity() != self.retained.identity()
            || source.parent().path_profile() != self.retained.path_profile()
            || source.kind() != NamespaceObjectKind::RegularFile
        {
            return Err(binding_error(
                "namespace source is not the capability this backend retained",
            ));
        }
        if destination.reservation() != self.retained.reservation() {
            return Err(binding_error(
                "namespace destination is not bound to the admitted reservation",
            ));
        }
        let identity = self.retained.execute_edge(
            edge,
            &retained.leaf,
            destination.leaf(),
            &retained.observed,
            retained.kind,
        )?;
        self.source = None;
        Ok(identity)
    }

    fn managed_unavailable<T>(&self) -> Result<T, CheckedFsError> {
        Err(CheckedFsError::ambiguous(
            "managed namespace operation",
            "managed component operations land in R2-D step 2.3",
        ))
    }
}

impl RawNamespaceBackend for HostActionNamespaceV1 {
    type DirectoryHandle = ActionNamespaceHandleV1;
    type ObjectHandle = ActionNamespaceHandleV1;
    type Identity = DurableObjectIdentityV1;
    type PathProfile = CanonicalPathIdentityV1;

    fn provider_binding(&self) -> ProviderBinding {
        self.provider
    }

    fn revalidate_action_directory(
        &mut self,
        expected_identity: &DurableObjectIdentityV1,
        expected_reservation: RecordDigestV1,
    ) -> Result<(), CheckedFsError> {
        self.retained
            .revalidate(expected_identity, expected_reservation)
    }

    /// Edge E12 (`GwzM5-8R2DInterfaceFreeze.md` §4.3), primitive family P1.
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
        let identity = self.execute(ActionNamespaceEdgeV1::Publish, source, destination)?;
        Ok(self.issuer().published(identity))
    }

    /// Edge E13, primitive family P1.
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
        let identity = self.execute(ActionNamespaceEdgeV1::Retire, source, destination)?;
        Ok(self.issuer().retired(identity))
    }

    fn install_managed_component(
        &mut self,
        _source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        _destination: &ActionDestination,
        _request: &ManagedInstallRequestV1,
    ) -> Result<ManagedInstallObservationV1, CheckedFsError> {
        self.managed_unavailable()
    }

    fn observe_installed_managed_component(
        &mut self,
        _request: &ManagedInstallRequestV1,
    ) -> Result<ManagedInstallObservationV1, CheckedFsError> {
        self.managed_unavailable()
    }

    fn retire_managed_marker(
        &mut self,
        _source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        _destination: &ActionDestination,
        _request: &ManagedMarkerRetirementRequestV1,
    ) -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError> {
        self.managed_unavailable()
    }

    fn observe_retired_managed_marker(
        &mut self,
        _request: &ManagedMarkerRetirementRequestV1,
    ) -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError> {
        self.managed_unavailable()
    }

    /// Edge E14, primitive family P5. The ordinal is the schedule's proof that
    /// this barrier is one the admitted action reserved; `barrier_slots`
    /// already bound it to this action and this retained target
    /// (`namespace/mod.rs:149-179`), and the physical barrier itself is over
    /// the whole retained action directory, so the ordinal selects no distinct
    /// physical target here.
    fn barrier(
        &mut self,
        parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        _ordinal: BarrierOrdinalV1,
    ) -> Result<DurableNamespace, CheckedFsError> {
        if parent.handle() != &self.handle
            || parent.identity() != self.retained.identity()
            || parent.path_profile() != self.retained.path_profile()
        {
            return Err(binding_error(
                "namespace barrier parent is not the retained action directory",
            ));
        }
        self.retained.barrier()?;
        Ok(self.issuer().durable())
    }
}
