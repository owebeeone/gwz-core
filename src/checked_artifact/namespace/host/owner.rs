use super::super::backend::{ActionDestination, BackendIssuer, NamespaceObjectKind};
use super::super::managed::{
    ManagedInstallObservationV1, ManagedInstallRequestV1, ManagedMarkerRetirementObservationV1,
    ManagedMarkerRetirementRequestV1,
};
use super::super::{RetainedDirectory, RetainedNamespaceObject, binding_error};
use crate::checked_artifact::capability::{
    ActionNamespaceEdgeV1, AsciiComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, ManagedInstalledFactsV1, ManagedRetiredFactsV1,
    ObservedManagedObjectV1, RetainedManagedParentV1,
};
use crate::checked_artifact::protocol::{ProtocolRecordKindV1, RecordDigestV1};

use super::*;

impl HostActionNamespaceV1 {
    pub(crate) fn issuer(&self) -> BackendIssuer {
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

    pub(crate) fn execute(
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
    pub(crate) fn managed(
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
    pub(crate) fn take_managed_source(
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
    pub(crate) fn installed(
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
    pub(crate) fn retired_marker(
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
