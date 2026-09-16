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

use super::backend::{BackendIssuer, NamespaceObjectKind};
use super::{
    ActionNamespace, BootstrapIntentRowV1, RetainedDirectory, RetainedNamespaceObject,
    binding_error,
};
use crate::checked_artifact::capability::{
    AliasRetirementEntryV1, AsciiComponent, BarrierIntentRowV1, CanonicalPathIdentityV1,
    CheckedFsError, DurableObjectIdentityV1, ManagedIntentEdgeV1, RetainedManagedParentV1,
    RoamingAnchorHomeWitnessV1, TargetAnchorAliasStateV1, barrier_target_parent,
    converge_target_anchor_alias, create_target_anchor_alias, observe_barrier_completion,
    observe_barrier_intent_row, observe_cleanup_completion, observe_cleanup_retirement,
    observe_cleanup_row_facts, observe_cleanup_worklist_row, observe_managed_intent_row,
    read_managed_intent_row, retire_target_anchor_alias, write_barrier_intent_scratch,
    write_cleanup_worklist_scratch, write_managed_intent_scratch,
};
use crate::checked_artifact::protocol::{
    BoundBarrierIntentV1, BoundCleanupWorklistV1, CleanupAliasV1, CleanupPhysicalFactV1,
    DurableLeafFingerprintV1, OwnershipMarkerV1, ProtocolRecordKindV1, managed_marker_name,
};

mod backend;
mod owner;
mod retain;

pub(crate) use retain::*;

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
    ///
    /// R2-E Phase E2.1 makes the second half of that sentence true rather than
    /// advisory: `target_leaf` is now *checked* against the frozen action-slot
    /// grammar (OPEN-B3, answered in `barrier_mutation.rs`'s header). This is
    /// the one gate both the first drive and every restart pass through, since
    /// a restart re-binds its slots from the leaf the intent record durably
    /// carries.
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
        require_reserved_target_leaf(&target_leaf, reservation.action_digest())?;
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

    /// R2-E Phase E2.1 — the barrier intent record's scheduled scratch row,
    /// written through the provider owner (keys #1-#3).
    ///
    /// Same rule as the managed intent lifecycle below: the leaf is taken from
    /// the schedule-derived barrier slots rather than from the caller, the slots
    /// are re-proved against this action's own binding first, and this owner
    /// still holds no injection site — every `barrier.*` boundary is announced
    /// from `barrier_mutation.rs`.
    pub(in crate::checked_artifact) fn write_barrier_intent_scratch(
        &self,
        slots: &super::BarrierSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        bytes: &[u8],
    ) -> Result<(), CheckedFsError> {
        self.validate_barrier_slots(slots)?;
        write_barrier_intent_scratch(&self.backend.retained, slots.scratch.leaf(), bytes)
    }

    /// R2-E Phase E2.1 — the post-edge proof of one scheduled barrier intent
    /// row, and the two boundaries around it (keys #4/#5 and #14/#15), with O6's
    /// read-side identity refusal folded in.
    pub(in crate::checked_artifact) fn observe_barrier_intent_row(
        &self,
        slots: &super::BarrierSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        row: BarrierIntentRowV1,
        home: &RoamingAnchorHomeWitnessV1,
    ) -> Result<BoundBarrierIntentV1, CheckedFsError> {
        self.validate_barrier_slots(slots)?;
        let leaf = match row {
            BarrierIntentRowV1::Active => slots.active.leaf(),
            BarrierIntentRowV1::Retired => slots.retired.leaf(),
        };
        observe_barrier_intent_row(
            &self.backend.retained,
            leaf,
            row,
            self.admitted_action().reservation(),
            slots.ordinal,
            home,
        )
    }

    /// R2-E Phase E2.1 — the alias phase's entry decision, and the only
    /// supported way to make it.
    ///
    /// A consumer must not branch on the reserved leaf's residency itself: the
    /// roaming barrier's Windows arm can leave the alias under an outbound name
    /// no owner outside `platform` can derive, and asking the wrong question
    /// there is exactly the defect the E2 review's [P2-1] found. This forwards
    /// to the provider, which converges that window before it answers.
    pub(in crate::checked_artifact) fn converge_target_anchor_alias(
        &self,
        slots: &super::BarrierSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<TargetAnchorAliasStateV1, CheckedFsError> {
        self.validate_barrier_slots(slots)?;
        converge_target_anchor_alias(&self.backend.retained, &slots.target.leaf)
    }

    /// R2-E Phase E2.1 — the target parent's roaming anchor alias, freshly
    /// created under the reserved leaf (keys #6-#7, DECISION B-5).
    ///
    /// Only legal after [`Self::converge_target_anchor_alias`] answered
    /// `Absent`. `create_new` is still the collision guard, so calling it out of
    /// order is a typed refusal rather than a second object.
    pub(in crate::checked_artifact) fn create_target_anchor_alias(
        &self,
        slots: &super::BarrierSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<(), CheckedFsError> {
        self.validate_barrier_slots(slots)?;
        create_target_anchor_alias(&self.backend.retained, &slots.target.leaf)
    }

    /// R2-E Phase E2.1 — the dirent barrier over the target parent, with the
    /// third `DirentBarrierClass` (keys #8-#9, DECISION B-3).
    ///
    /// The identity the post-barrier reobservation proves comes from the
    /// resident intent record, never from this owner's own expectation.
    pub(in crate::checked_artifact) fn barrier_target_parent(
        &self,
        slots: &super::BarrierSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        intent: &BoundBarrierIntentV1,
    ) -> Result<(), CheckedFsError> {
        self.validate_barrier_slots(slots)?;
        barrier_target_parent(
            &self.backend.retained,
            &slots.target.leaf,
            intent.value().target_parent_identity(),
        )
    }

    /// R2-E Phase E2.1 — the post-edge proof that the alias has left the
    /// reserved leaf for its scheduled retirement row (keys #10/#11 and
    /// #12/#13, DECISION B-4).
    pub(in crate::checked_artifact) fn observe_retired_target_anchor_alias(
        &self,
        slots: &super::BarrierSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
        entry: AliasRetirementEntryV1,
    ) -> Result<(), CheckedFsError> {
        self.validate_barrier_slots(slots)?;
        retire_target_anchor_alias(
            &self.backend.retained,
            &slots.target.leaf,
            slots.retired_anchor_alias.leaf(),
            entry,
        )
    }

    /// R2-E Phase E2.1 — the settled-ordinal restart observation (key #16).
    pub(in crate::checked_artifact) fn observe_barrier_completion(
        &self,
        slots: &super::BarrierSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<(), CheckedFsError> {
        self.validate_barrier_slots(slots)?;
        observe_barrier_completion(
            &self.backend.retained,
            &slots.target.leaf,
            slots.retired_anchor_alias.leaf(),
            slots.retired.leaf(),
        )
    }

    /// The barrier slots must belong to this admitted action, and the target
    /// parent they name must still be the retained action directory — OPEN-B2's
    /// answer at E2.1, restated where it is enforced rather than only where it
    /// is recorded.
    fn validate_barrier_slots(
        &self,
        slots: &super::BarrierSlots<
            ActionNamespaceHandleV1,
            DurableObjectIdentityV1,
            CanonicalPathIdentityV1,
        >,
    ) -> Result<(), CheckedFsError> {
        if slots.binding != self.binding() {
            return Err(binding_error(
                "barrier slots do not belong to the admitted action",
            ));
        }
        if slots.target.parent.handle() != &self.backend.handle
            || slots.target.parent.identity() != self.backend.retained.identity()
            || slots.target.parent.path_profile() != self.backend.retained.path_profile()
        {
            return Err(binding_error(
                "barrier target parent is not the retained action directory",
            ));
        }
        Ok(())
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
        write_managed_intent_scratch(
            &self.backend.retained,
            BootstrapIntentRowV1::Scratch.leaf(slots),
            bytes,
            edge,
        )
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

    /// R2-E Phase E1 Step E1.1 — the cleanup worklist's scheduled scratch row,
    /// written through the provider owner.
    ///
    /// The leaf is `BaseActionSlotV1::RecordScratch`, taken from this action's
    /// own publish role rather than from the caller (DECISION C-3 as simplified
    /// at E0.2b §4), so a consumer cannot name a row the schedule did not
    /// reserve. **This owner still holds no injection site:** the three scratch
    /// boundaries are announced from `namespace_mutation.rs`, exactly as the
    /// E12/E13 and E15/E16 ones are.
    pub(in crate::checked_artifact) fn write_cleanup_worklist_scratch(
        &self,
        bytes: &[u8],
    ) -> Result<(), CheckedFsError> {
        let scratch = self.publish_destination(super::PublishRoleV1::RecordScratch);
        write_cleanup_worklist_scratch(&self.backend.retained, scratch.leaf(), bytes)
    }

    /// R2-E E1.1 — the published worklist row, bound to this action's resident
    /// reservation, and the two boundaries around it.
    pub(in crate::checked_artifact) fn observe_cleanup_worklist_row(
        &self,
    ) -> Result<BoundCleanupWorklistV1, CheckedFsError> {
        let worklist = self.publish_destination(super::PublishRoleV1::CleanupWorklist);
        observe_cleanup_worklist_row(
            &self.backend.retained,
            worklist.leaf(),
            self.admitted_action().reservation(),
        )
    }

    /// R2-E E1.1 — one worklist row's `(source, destination)` physical fact
    /// pair, over the two schedule-derived leaves the alias resolves between.
    pub(in crate::checked_artifact) fn observe_cleanup_row_facts(
        &self,
        retirement: &super::CleanupRetirementDestination,
    ) -> Result<(CleanupPhysicalFactV1, CleanupPhysicalFactV1), CheckedFsError> {
        self.validate_cleanup_retirement(retirement)?;
        observe_cleanup_row_facts(
            &self.backend.retained,
            retirement.source_leaf(),
            retirement.leaf(),
        )
    }

    /// R2-E E1.1 — the post-edge proof of one alias retirement, and the three
    /// boundaries around it.
    pub(in crate::checked_artifact) fn observe_cleanup_retirement(
        &self,
        retirement: &super::CleanupRetirementDestination,
        expected: &DurableLeafFingerprintV1,
    ) -> Result<(), CheckedFsError> {
        self.validate_cleanup_retirement(retirement)?;
        observe_cleanup_retirement(&self.backend.retained, retirement.leaf(), expected)
    }

    /// R2-E E1.1 — the whole-worklist completion proof.
    ///
    /// The reserved alias set comes from this action's own schedule, so an alias
    /// the schedule did not reserve is skipped rather than fabricated
    /// (`namespace/mod.rs`, `cleanup_retirement` refuses it).
    pub(in crate::checked_artifact) fn observe_cleanup_completion(
        &self,
    ) -> Result<(), CheckedFsError> {
        let worklist = self.publish_destination(super::PublishRoleV1::CleanupWorklist);
        let mut rows = Vec::new();
        for alias in CleanupAliasV1::ALL {
            if let Ok(retirement) = self.cleanup_retirement(alias) {
                rows.push((
                    alias,
                    retirement.source_leaf().clone(),
                    retirement.leaf().clone(),
                ));
            }
        }
        observe_cleanup_completion(
            &self.backend.retained,
            worklist.leaf(),
            self.admitted_action().reservation(),
            &rows,
        )
    }

    /// A cleanup destination must belong to this admitted action, for the same
    /// reason every other scheduled entry point re-proves its binding.
    fn validate_cleanup_retirement(
        &self,
        retirement: &super::CleanupRetirementDestination,
    ) -> Result<(), CheckedFsError> {
        if retirement.binding != self.binding() {
            return Err(binding_error(
                "cleanup retirement does not belong to the admitted action",
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
