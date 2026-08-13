//! Role-typed namespace transitions for barrier and managed-bootstrap protocols.

use super::backend::{
    BackendIssuer, NamespaceObjectKind, RawNamespaceBackend, RetainedNamespaceObject,
};
use super::managed::{ManagedInstallRequestV1, ManagedMarkerRetirementRequestV1};
use super::{
    ActionBinding, ActionNamespace, BarrierSlots, BootstrapComponentSlots,
    BootstrapGenerationSlots, InstalledManagedComponentV1, PublishedIdentity, RetiredIdentity,
    RetiredManagedMarkerV1, binding_error,
};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
};
use crate::checked_artifact::protocol::ManagedParentBootstrapIntentV1;

impl<
    Implementation: RawNamespaceBackend<Identity = DurableObjectIdentityV1, PathProfile = CanonicalPathIdentityV1>,
> ActionNamespace<Implementation>
{
    pub(in crate::checked_artifact) fn publish_barrier_intent(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        slots: &BarrierSlots<
            Implementation::DirectoryHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<PublishedIdentity<DurableObjectIdentityV1>, CheckedFsError> {
        self.validate_action_source(
            source,
            slots.binding,
            slots.scratch.leaf(),
            NamespaceObjectKind::RegularFile,
        )?;
        self.backend.publish_no_replace(source, &slots.active)
    }

    pub(in crate::checked_artifact) fn retire_barrier_intent(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        slots: &BarrierSlots<
            Implementation::DirectoryHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<RetiredIdentity<DurableObjectIdentityV1>, CheckedFsError> {
        self.validate_action_source(
            source,
            slots.binding,
            slots.active.leaf(),
            NamespaceObjectKind::RegularFile,
        )?;
        self.backend.retire_exact(source, &slots.retired)
    }

    pub(in crate::checked_artifact) fn retire_barrier_target_alias(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        slots: &BarrierSlots<
            Implementation::DirectoryHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<RetiredIdentity<DurableObjectIdentityV1>, CheckedFsError> {
        self.validate_operation(source.provider(), slots.binding)?;
        validate_source_role(
            source,
            slots.target.parent.identity(),
            slots.target.parent.path_profile(),
            &slots.target.leaf,
            NamespaceObjectKind::RegularFile,
        )?;
        self.backend
            .retire_exact(source, &slots.retired_anchor_alias)
    }

    pub(in crate::checked_artifact) fn publish_bootstrap_generation(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        slots: &BootstrapGenerationSlots,
    ) -> Result<PublishedIdentity<DurableObjectIdentityV1>, CheckedFsError> {
        self.validate_action_source(
            source,
            slots.binding,
            slots.scratch.leaf(),
            NamespaceObjectKind::RegularFile,
        )?;
        self.backend.publish_no_replace(source, &slots.active)
    }

    pub(in crate::checked_artifact) fn retire_bootstrap_generation(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        slots: &BootstrapGenerationSlots,
    ) -> Result<RetiredIdentity<DurableObjectIdentityV1>, CheckedFsError> {
        self.validate_action_source(
            source,
            slots.binding,
            slots.active.leaf(),
            NamespaceObjectKind::RegularFile,
        )?;
        self.backend.retire_exact(source, &slots.retired)
    }

    pub(in crate::checked_artifact) fn install_bootstrap_component(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        intent: &ManagedParentBootstrapIntentV1,
        slots: &BootstrapComponentSlots<
            Implementation::DirectoryHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<InstalledManagedComponentV1, CheckedFsError> {
        let request = ManagedInstallRequestV1::bind(
            self.backend.provider_binding(),
            self.admitted_action.reservation(),
            intent,
            slots,
        )?;
        self.validate_operation(source.provider(), slots.binding)?;
        validate_source_role(
            source,
            slots.target.parent.identity(),
            slots.target.parent.path_profile(),
            &slots.target.staging_leaf,
            NamespaceObjectKind::Directory,
        )?;
        let observation =
            self.backend
                .install_managed_component(source, &slots.final_destination, &request)?;
        let evidence = self.installed_evidence(intent, &request, observation)?;
        if evidence.installed_identity() != source.identity() {
            return Err(binding_error(
                "managed install changed the retained directory identity",
            ));
        }
        Ok(evidence)
    }

    pub(in crate::checked_artifact) fn recover_installed_bootstrap_component(
        &mut self,
        intent: &ManagedParentBootstrapIntentV1,
        slots: &BootstrapComponentSlots<
            Implementation::DirectoryHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<InstalledManagedComponentV1, CheckedFsError> {
        let request = ManagedInstallRequestV1::bind(
            self.backend.provider_binding(),
            self.admitted_action.reservation(),
            intent,
            slots,
        )?;
        self.validate_operation(slots.target.parent.provider(), slots.binding)?;
        let observation = self.backend.observe_installed_managed_component(&request)?;
        self.installed_evidence(intent, &request, observation)
    }

    pub(in crate::checked_artifact) fn retire_bootstrap_marker(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        intent: &ManagedParentBootstrapIntentV1,
        slots: &BootstrapComponentSlots<
            Implementation::DirectoryHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<RetiredManagedMarkerV1, CheckedFsError> {
        let request = ManagedMarkerRetirementRequestV1::bind(
            self.backend.provider_binding(),
            self.admitted_action.reservation(),
            intent,
            slots,
        )?;
        self.validate_operation(source.provider(), slots.binding)?;
        validate_marker_source_role(source, slots)?;
        if !request.matches_retained_marker(
            source.identity(),
            source.parent().identity(),
            source.parent().path_profile(),
        ) {
            return Err(binding_error("managed marker retained facts changed"));
        }
        let observation =
            self.backend
                .retire_managed_marker(source, &slots.marker_retired, &request)?;
        self.retired_marker_evidence(intent, &request, observation)
    }

    pub(in crate::checked_artifact) fn recover_retired_bootstrap_marker(
        &mut self,
        intent: &ManagedParentBootstrapIntentV1,
        slots: &BootstrapComponentSlots<
            Implementation::DirectoryHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<RetiredManagedMarkerV1, CheckedFsError> {
        let request = ManagedMarkerRetirementRequestV1::bind(
            self.backend.provider_binding(),
            self.admitted_action.reservation(),
            intent,
            slots,
        )?;
        self.validate_operation(slots.target.parent.provider(), slots.binding)?;
        let observation = self.backend.observe_retired_managed_marker(&request)?;
        self.retired_marker_evidence(intent, &request, observation)
    }

    fn installed_evidence(
        &self,
        intent: &ManagedParentBootstrapIntentV1,
        request: &ManagedInstallRequestV1,
        observation: super::managed::ManagedInstallObservationV1,
    ) -> Result<InstalledManagedComponentV1, CheckedFsError> {
        let evidence = BackendIssuer::new(self.backend.provider_binding())
            .installed_managed_component(request, observation)?;
        intent
            .successor_after_component(&evidence)
            .map_err(|_| binding_error("managed install observation does not close intent"))?;
        Ok(evidence)
    }

    fn retired_marker_evidence(
        &self,
        intent: &ManagedParentBootstrapIntentV1,
        request: &ManagedMarkerRetirementRequestV1,
        observation: super::managed::ManagedMarkerRetirementObservationV1,
    ) -> Result<RetiredManagedMarkerV1, CheckedFsError> {
        let evidence = BackendIssuer::new(self.backend.provider_binding())
            .retired_managed_marker(request, observation)?;
        intent
            .successor_after_marker_retirement(&evidence)
            .map_err(|_| binding_error("managed retirement observation does not close intent"))?;
        Ok(evidence)
    }

    fn validate_action_source(
        &mut self,
        source: &RetainedNamespaceObject<
            Implementation::DirectoryHandle,
            Implementation::ObjectHandle,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        binding: ActionBinding,
        expected_leaf: &AsciiComponent,
        expected_kind: NamespaceObjectKind,
    ) -> Result<(), CheckedFsError> {
        self.validate_operation(source.provider(), binding)?;
        validate_action_source_role(
            source,
            self.admitted_action.directory_identity(),
            expected_leaf,
            expected_kind,
        )
    }
}

fn validate_source_role<DirectoryHandle, ObjectHandle>(
    source: &RetainedNamespaceObject<
        DirectoryHandle,
        ObjectHandle,
        DurableObjectIdentityV1,
        CanonicalPathIdentityV1,
    >,
    expected_parent_identity: &DurableObjectIdentityV1,
    expected_parent_path: &CanonicalPathIdentityV1,
    expected_leaf: &AsciiComponent,
    expected_kind: NamespaceObjectKind,
) -> Result<(), CheckedFsError> {
    if source.parent().identity() != expected_parent_identity
        || source.parent().path_profile() != expected_parent_path
        || source.leaf() != expected_leaf
        || source.kind() != expected_kind
    {
        return Err(binding_error("namespace source role mismatch"));
    }
    Ok(())
}

fn validate_action_source_role<DirectoryHandle, ObjectHandle>(
    source: &RetainedNamespaceObject<
        DirectoryHandle,
        ObjectHandle,
        DurableObjectIdentityV1,
        CanonicalPathIdentityV1,
    >,
    expected_parent_identity: &DurableObjectIdentityV1,
    expected_leaf: &AsciiComponent,
    expected_kind: NamespaceObjectKind,
) -> Result<(), CheckedFsError> {
    if source.parent().identity() != expected_parent_identity
        || source.leaf() != expected_leaf
        || source.kind() != expected_kind
    {
        return Err(binding_error("action namespace source role mismatch"));
    }
    Ok(())
}

fn validate_marker_source_role<DirectoryHandle, ObjectHandle>(
    source: &RetainedNamespaceObject<
        DirectoryHandle,
        ObjectHandle,
        DurableObjectIdentityV1,
        CanonicalPathIdentityV1,
    >,
    slots: &BootstrapComponentSlots<
        DirectoryHandle,
        DurableObjectIdentityV1,
        CanonicalPathIdentityV1,
    >,
) -> Result<(), CheckedFsError> {
    let source_path = source.parent().path_profile().components();
    let retained_path = slots.target.parent.path_profile().components();
    let Some(installed_component) = source_path.strip_prefix(retained_path) else {
        return Err(binding_error("bootstrap marker parent changed"));
    };
    if installed_component.len() != 1
        || installed_component[0].original() != &slots.target.final_leaf
        || installed_component[0].parent_durable_identity() != slots.target.parent.identity()
        || source.leaf() != &crate::checked_artifact::protocol::managed_marker_name()
        || source.kind() != NamespaceObjectKind::RegularFile
    {
        return Err(binding_error("bootstrap marker source role mismatch"));
    }
    Ok(())
}
