use super::super::backend::{
    ActionDestination, NamespaceObjectKind, ProviderBinding, RawNamespaceBackend,
};
use super::super::managed::{
    ManagedInstallObservationV1, ManagedInstallRequestV1, ManagedMarkerRetirementObservationV1,
    ManagedMarkerRetirementRequestV1,
};
use super::super::{
    DurableNamespace, PublishedIdentity, RetainedDirectory, RetainedNamespaceObject,
    RetiredIdentity, binding_error,
};
use crate::checked_artifact::capability::{
    ActionNamespaceEdgeV1, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
};
use crate::checked_artifact::protocol::{BarrierOrdinalV1, RecordDigestV1, managed_marker_name};

use super::*;

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
