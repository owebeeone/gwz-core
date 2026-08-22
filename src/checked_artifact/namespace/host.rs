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
    ActionNamespace, BootstrapIntentRowV1, DurableNamespace, PublishedIdentity, RetainedDirectory,
    RetainedNamespaceObject, RetiredIdentity, binding_error,
};
use crate::checked_artifact::capability::{
    ActionNamespaceEdgeV1, AsciiComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, ManagedInstalledFactsV1, ManagedIntentEdgeV1, ManagedRetiredFactsV1,
    ObservedManagedObjectV1, ObservedNamespaceObjectV1, RetainedActionNamespaceV1,
    RetainedManagedParentV1, observe_managed_intent_row, read_managed_intent_row,
    write_managed_intent_scratch,
};
use crate::checked_artifact::catalog::OpaqueRetainedCatalogV1;
use crate::checked_artifact::protocol::{
    AdmittedActionV1, BarrierOrdinalV1, OwnershipMarkerV1, ProtocolRecordKindV1, RecordDigestV1,
    managed_marker_name,
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

/// The one managed component this backend currently holds retained: its parent
/// capability, and the staged directory or ownership marker its next edge will
/// consume.
struct RetainedManagedV1 {
    parent: RetainedManagedParentV1,
    source: Option<ObservedManagedObjectV1>,
}

/// The production `RawNamespaceBackend` over one admitted action directory.
pub(in crate::checked_artifact) struct HostActionNamespaceV1 {
    retained: RetainedActionNamespaceV1,
    provider: ProviderBinding,
    handle: ActionNamespaceHandleV1,
    source: Option<RetainedSourceV1>,
    managed: Option<RetainedManagedV1>,
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
        managed: None,
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

    /// Retains one managed parent as this backend's managed target and returns
    /// the scheduled component slots over it.
    ///
    /// Both names in the returned slots are schedule- and intent-derived
    /// (`managed_staging_name`, the component's own `final_name`, and
    /// `ActionSlotV1::RetiredBootstrapMarker`); nothing is minted here. The
    /// `RetainedManagedParentV1` is the provider owner's opaque capability, so a
    /// consumer still never receives a path or an OS handle.
    pub(in crate::checked_artifact) fn retain_managed_component_slots(
        &mut self,
        parent: RetainedManagedParentV1,
        bootstrap_index: usize,
        component_ordinal: usize,
        final_leaf: AsciiComponent,
    ) -> Result<
        super::BootstrapComponentSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        CheckedFsError,
    > {
        let reservation = self.admitted_action().reservation();
        if parent.reservation() != reservation.record_digest() {
            return Err(binding_error(
                "managed parent is not bound to the admitted reservation",
            ));
        }
        let issuer = BackendIssuer::new(self.backend.provider);
        let retained_parent = issuer.retained_directory(
            self.backend.handle,
            parent.identity().clone(),
            parent.path_profile().clone(),
        );
        let target = issuer.bootstrap_target(
            retained_parent,
            reservation.action_digest(),
            reservation.record_digest(),
            component_ordinal,
            final_leaf,
        )?;
        let slots = self
            .bootstrap_slots(bootstrap_index)
            .map_err(|_| binding_error("bootstrap row is not scheduled"))?
            .component(component_ordinal, target)
            .map_err(|_| binding_error("bootstrap component is not scheduled"))?;
        self.backend.managed = Some(RetainedManagedV1 {
            parent,
            source: None,
        });
        Ok(slots)
    }

    /// Retains the staged component directory as this backend's managed source.
    /// The §4.4 Class 1 interior expectation is proved once here, so a directory
    /// this backend would refuse to publish cannot be retained in the first
    /// place.
    pub(in crate::checked_artifact) fn retain_managed_staging_source(
        &mut self,
        intent: &crate::checked_artifact::protocol::ManagedParentBootstrapIntentV1,
        slots: &super::BootstrapComponentSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<
        RetainedNamespaceObject<
            ActionNamespaceHandleV1,
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        CheckedFsError,
    > {
        let marker = OwnershipMarkerV1::for_current_component(intent)
            .map_err(|_| binding_error("managed install intent cannot issue its marker"))?;
        let handle = self.backend.handle;
        let issuer = BackendIssuer::new(self.backend.provider);
        let Some(managed) = self.backend.managed.as_mut() else {
            return Err(binding_error("no managed parent is retained"));
        };
        let observed = managed
            .parent
            .retain_staging_source(slots.staging_leaf(), &marker)?;
        let parent = issuer.retained_directory(
            handle,
            managed.parent.identity().clone(),
            managed.parent.path_profile().clone(),
        );
        let object = issuer.retained_object_from_parent(
            parent,
            slots.staging_leaf().clone(),
            handle,
            observed.identity().clone(),
            NamespaceObjectKind::Directory,
        );
        managed.source = Some(observed);
        Ok(object)
    }

    /// R2-D Phase 3 Step 3.1b — the managed intent record's scheduled scratch
    /// row, written through the provider owner.
    ///
    /// The leaf is taken from the schedule-derived generation slots rather than
    /// from the caller, and the slots are re-proved against this action's own
    /// binding first, so a consumer cannot name a row the schedule did not
    /// reserve. This owner still holds no injection site: the boundaries are
    /// announced from `managed_mutation.rs`, exactly as the E15/E16 ones are.
    pub(in crate::checked_artifact) fn write_bootstrap_intent_scratch(
        &self,
        slots: &super::BootstrapGenerationSlots,
        bytes: &[u8],
        edge: ManagedIntentEdgeV1,
    ) -> Result<(), CheckedFsError> {
        self.validate_generation_slots(slots)?;
        write_managed_intent_scratch(&self.backend.retained, slots.scratch_leaf(), bytes, edge)
    }

    /// R2-D Phase 3 Step 3.1b — the post-edge proof of one scheduled intent row,
    /// and the two boundaries around it.
    pub(in crate::checked_artifact) fn observe_bootstrap_intent_row(
        &self,
        slots: &super::BootstrapGenerationSlots,
        row: BootstrapIntentRowV1,
        edge: ManagedIntentEdgeV1,
    ) -> Result<Vec<u8>, CheckedFsError> {
        self.validate_generation_slots(slots)?;
        observe_managed_intent_row(&self.backend.retained, row.leaf(slots), edge)
    }

    /// R2-D Phase 3 Step 3.1b — a bounded read of one resident scheduled intent
    /// row, for the resume's own chain walk. No durable edge, no boundary.
    pub(in crate::checked_artifact) fn read_bootstrap_intent_row(
        &self,
        slots: &super::BootstrapGenerationSlots,
        row: BootstrapIntentRowV1,
    ) -> Result<Vec<u8>, CheckedFsError> {
        self.validate_generation_slots(slots)?;
        read_managed_intent_row(
            &self.backend.retained,
            row.leaf(slots),
            "read resident managed intent",
        )
    }

    /// Whether one scheduled intent row is durably resident.
    pub(in crate::checked_artifact) fn bootstrap_intent_row_is_resident(
        &self,
        slots: &super::BootstrapGenerationSlots,
        row: BootstrapIntentRowV1,
    ) -> bool {
        self.backend.row_is_resident(row.leaf(slots))
    }

    /// The generation slots must belong to this admitted action, for the same
    /// reason every other managed entry point re-proves its binding.
    fn validate_generation_slots(
        &self,
        slots: &super::BootstrapGenerationSlots,
    ) -> Result<(), CheckedFsError> {
        if slots.binding != self.binding() {
            return Err(binding_error(
                "bootstrap generation slots do not belong to the admitted action",
            ));
        }
        Ok(())
    }

    /// Retains the installed component's ownership marker as this backend's
    /// managed source. Its parent is the *installed component*, not the managed
    /// parent, which is exactly the role
    /// `namespace/operations.rs:340-367` validates.
    pub(in crate::checked_artifact) fn retain_managed_marker_source(
        &mut self,
        slots: &super::BootstrapComponentSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<
        RetainedNamespaceObject<
            ActionNamespaceHandleV1,
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        CheckedFsError,
    > {
        let handle = self.backend.handle;
        let issuer = BackendIssuer::new(self.backend.provider);
        let Some(managed) = self.backend.managed.as_mut() else {
            return Err(binding_error("no managed parent is retained"));
        };
        let (installed_identity, installed_path) =
            managed.parent.installed_facts(slots.final_leaf())?;
        let observed = managed.parent.retain_marker_source(slots.final_leaf())?;
        let parent = issuer.retained_directory(handle, installed_identity, installed_path);
        let object = issuer.retained_object_from_parent(
            parent,
            managed_marker_name(),
            handle,
            observed.identity().clone(),
            NamespaceObjectKind::RegularFile,
        );
        managed.source = Some(observed);
        Ok(object)
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

    /// The retained managed parent, revalidated against the admitted
    /// reservation before every managed operation — the managed analogue of
    /// `validate_operation`'s action-directory revalidation, and the boundary
    /// `managed_bootstrap.parent_revalidate` names.
    fn managed(
        &self,
        expected_reservation: RecordDigestV1,
    ) -> Result<&RetainedManagedParentV1, CheckedFsError> {
        let Some(managed) = self.managed.as_ref() else {
            return Err(binding_error(
                "managed operation has no retained managed parent",
            ));
        };
        if expected_reservation != self.retained.reservation() {
            return Err(binding_error(
                "managed request is not bound to the admitted reservation",
            ));
        }
        managed.parent.revalidate(expected_reservation)?;
        Ok(&managed.parent)
    }

    /// The managed twin of [`Self::execute`]'s source check: a managed edge
    /// consumes the object this backend itself retained, under this backend's
    /// own handle, beneath the managed parent it still holds. A capability that
    /// names a different leaf, identity, parent or kind is refused before any
    /// physical edge, and the retention is cleared by the edge that consumes it.
    fn take_managed_source(
        &mut self,
        source: &RetainedNamespaceObject<
            ActionNamespaceHandleV1,
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        expected_leaf: &AsciiComponent,
        expected_kind: NamespaceObjectKind,
    ) -> Result<ObservedManagedObjectV1, CheckedFsError> {
        let Some(managed) = self.managed.as_mut() else {
            return Err(binding_error(
                "managed operation has no retained managed parent",
            ));
        };
        let Some(retained) = managed.source.take() else {
            return Err(binding_error(
                "managed edge has no retained source capability",
            ));
        };
        if source.leaf() != expected_leaf
            || source.identity() != retained.identity()
            || source.handle() != &self.handle
            || source.kind() != expected_kind
        {
            return Err(binding_error(
                "managed source is not the capability this backend retained",
            ));
        }
        Ok(retained)
    }

    /// The install observation. The marker is the one the request already bound
    /// from the intent and the interior recheck proved byte-exact on disk; every
    /// other field is a fact this backend independently reobserved.
    fn installed(
        &self,
        request: &ManagedInstallRequestV1,
        facts: ManagedInstalledFactsV1,
    ) -> Result<ManagedInstallObservationV1, CheckedFsError> {
        request.complete(
            self.provider,
            request.expected_marker().clone(),
            facts.marker_object_identity,
            facts.installed_identity,
            facts.installed_mode,
            facts.installed_path,
        )
    }

    /// The retirement observation. The marker is re-derived from the *durable*
    /// bytes of the retired row and bound back into the intent, so a substituted
    /// or replayed marker is a typed refusal rather than accepted evidence.
    fn retired_marker(
        &self,
        request: &ManagedMarkerRetirementRequestV1,
        facts: ManagedRetiredFactsV1,
    ) -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError> {
        let marker = request.bind_marker_bytes(&facts.marker_bytes)?;
        request.complete(
            self.provider,
            marker,
            facts.retired_marker_identity,
            facts.installed_parent_identity,
            facts.installed_parent_mode,
            facts.installed_parent_path,
        )
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

    /// Edge E15 (`GwzM5-8R2DInterfaceFreeze.md` §4.3), primitives P1+P2+P3, with
    /// the §4.4 Class 1 managed source-interior arm. The observation returned is
    /// the durable reobservation of the published component, not the request's
    /// own expectation, so `ManagedInstallRequestV1::complete` compares two
    /// independently established facts.
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
        if destination.leaf() != request.final_leaf() {
            return Err(binding_error(
                "managed install destination is not the component's final name",
            ));
        }
        self.managed(destination.reservation())?;
        let retained = self.take_managed_source(
            source,
            request.staging_leaf(),
            NamespaceObjectKind::Directory,
        )?;
        let managed = self.managed(request.reservation())?;
        let facts = managed.install_component(
            request.staging_leaf(),
            request.final_leaf(),
            &retained,
            request.expected_marker(),
        )?;
        self.installed(request, facts)
    }

    /// The restart half of edge E15 (ConsumerCheckpoint §8 :228-231): no edge,
    /// the same reobservation, and therefore the same evidence.
    fn observe_installed_managed_component(
        &mut self,
        request: &ManagedInstallRequestV1,
    ) -> Result<ManagedInstallObservationV1, CheckedFsError> {
        let facts = self
            .managed(request.reservation())?
            .observe_installed_on_restart(request.final_leaf(), request.expected_marker())?;
        self.installed(request, facts)
    }

    /// Edge E16, primitive family P1. The marker is a regular file, so the
    /// primitive verifies it by identity and bytes and neither §4.4 arm is
    /// involved — §4.3's E16 annotation makes the destination arm conditional on
    /// the marker retiring as a directory, and it does not.
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
        if destination.leaf() != request.marker_retirement_leaf() {
            return Err(binding_error(
                "managed marker destination is not the scheduled retirement row",
            ));
        }
        self.managed(destination.reservation())?;
        let retained = self.take_managed_source(
            source,
            &managed_marker_name(),
            NamespaceObjectKind::RegularFile,
        )?;
        let facts = self.managed(request.reservation())?.retire_marker(
            &self.retained,
            request.final_leaf(),
            destination.leaf(),
            &retained,
        )?;
        self.retired_marker(request, facts)
    }

    /// The restart half of edge E16 (ConsumerCheckpoint §8 :228-231).
    fn observe_retired_managed_marker(
        &mut self,
        request: &ManagedMarkerRetirementRequestV1,
    ) -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError> {
        let facts = self
            .managed(request.reservation())?
            .observe_retired_marker(
                &self.retained,
                request.final_leaf(),
                request.marker_retirement_leaf(),
            )?;
        self.retired_marker(request, facts)
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
