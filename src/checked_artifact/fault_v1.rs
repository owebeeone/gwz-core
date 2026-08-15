//! Target-independent v1 fault vocabulary.
//!
//! Every variant denotes one restart-visible boundary. Repeated scheduled
//! operations use [`FaultInstanceV1`] for ordinals and diagnostic labels; they
//! never mint new stable keys at runtime.

macro_rules! define_fault_keys {
    ($($variant:ident => $stable_key:literal,)+) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(super) enum CheckedArtifactFaultKeyV1 {
            $($variant,)+
        }

        impl CheckedArtifactFaultKeyV1 {
            const ALL: &'static [Self] = &[$(Self::$variant,)+];

            pub(super) fn all() -> Vec<Self> {
                Self::ALL.to_vec()
            }

            const fn stable_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $stable_key,)+
                }
            }

            pub(super) fn stable_key(&self) -> String {
                self.stable_name().to_owned()
            }
        }
    };
}

define_fault_keys! {
    RuntimeGitDirRetain => "runtime.git_dir_retain",
    RuntimeWorkspaceRetain => "runtime.workspace_retain",
    RuntimeBootstrapGuardOpenOrCreate => "runtime.bootstrap_guard_open_or_create",
    RuntimeBootstrapGuardLockAcquire => "runtime.bootstrap_guard_lock_acquire",
    RuntimeBootstrapGuardReobserve => "runtime.bootstrap_guard_reobserve",
    RuntimeGwzDirectoryCreate => "runtime.gwz_directory_create",
    RuntimeGwzDirectoryReobserve => "runtime.gwz_directory_reobserve",
    RuntimeLocksDirectoryCreate => "runtime.locks_directory_create",
    RuntimeLocksDirectoryReobserve => "runtime.locks_directory_reobserve",
    RuntimeLeaseFileOpenOrCreate => "runtime.lease_file_open_or_create",
    RuntimeLeaseFileReobserveBeforeLock => "runtime.lease_file_reobserve_before_lock",
    RuntimeLeaseLockAcquire => "runtime.lease_lock_acquire",
    RuntimeLeaseReobserveAfterLock => "runtime.lease_reobserve_after_lock",
    RuntimeBootstrapGuardRelease => "runtime.bootstrap_guard_release",
    RuntimeLeaseRelease => "runtime.lease_release",
    RuntimePathWalk => "runtime.path_walk",
    RuntimeCollisionScan => "runtime.collision_scan",
    RuntimeCapabilityProof => "runtime.capability_proof",
    CatalogBootstrapGitParentCreate => "catalog_bootstrap.git_parent_create",
    CatalogBootstrapGitParentReobserve => "catalog_bootstrap.git_parent_reobserve",
    CatalogBootstrapReadyEdgeRootFlush => "catalog_bootstrap.ready_edge_root_flush",
    CatalogBootstrapScratchCreate => "catalog_bootstrap.scratch_create",
    CatalogBootstrapScratchWrite => "catalog_bootstrap.scratch_write",
    CatalogBootstrapScratchFlush => "catalog_bootstrap.scratch_flush",
    CatalogBootstrapScratchRootFlush => "catalog_bootstrap.scratch_root_flush",
    CatalogBootstrapActivePublish => "catalog_bootstrap.active_publish",
    CatalogBootstrapActiveReobserve => "catalog_bootstrap.active_reobserve",
    CatalogBootstrapStagingCreate => "catalog_bootstrap.staging_create",
    CatalogBootstrapInfrastructurePopulate => "catalog_bootstrap.infrastructure_populate",
    CatalogBootstrapInfrastructureFlush => "catalog_bootstrap.infrastructure_flush",
    CatalogBootstrapAnchorScratchCreate => "catalog_bootstrap.anchor_scratch_create",
    CatalogBootstrapAnchorScratchFlush => "catalog_bootstrap.anchor_scratch_flush",
    CatalogBootstrapAnchorPublish => "catalog_bootstrap.anchor_publish",
    CatalogBootstrapAnchorReobserve => "catalog_bootstrap.anchor_reobserve",
    CatalogBootstrapAnchorHomeAExercise => "catalog_bootstrap.anchor_home_a_exercise",
    CatalogBootstrapAnchorHomeBExercise => "catalog_bootstrap.anchor_home_b_exercise",
    CatalogBootstrapStagingFlush => "catalog_bootstrap.staging_flush",
    CatalogBootstrapFinalPublish => "catalog_bootstrap.final_publish",
    CatalogBootstrapFinalReopen => "catalog_bootstrap.final_reopen",
    CatalogBootstrapFinalReobserve => "catalog_bootstrap.final_reobserve",
    CatalogBootstrapActiveRetire => "catalog_bootstrap.active_retire",
    CatalogBootstrapRetiredReobserve => "catalog_bootstrap.retired_reobserve",
    CatalogBootstrapCatalogEnumerate => "catalog_bootstrap.catalog_enumerate",
    AdmissionOccupancyObserve => "admission.occupancy_observe",
    AdmissionCapacityCheck => "admission.capacity_check",
    AdmissionPreparingScratchCreate => "admission.preparing_scratch_create",
    AdmissionPreparingScratchWrite => "admission.preparing_scratch_write",
    AdmissionPreparingScratchFlush => "admission.preparing_scratch_flush",
    AdmissionPreparingPublish => "admission.preparing_publish",
    AdmissionPreparingReobserve => "admission.preparing_reobserve",
    AdmissionStagingCreate => "admission.staging_create",
    AdmissionReservationCreate => "admission.reservation_create",
    AdmissionReservationWrite => "admission.reservation_write",
    AdmissionReservationFlush => "admission.reservation_flush",
    AdmissionStagingFlush => "admission.staging_flush",
    AdmissionFinalPublish => "admission.final_publish",
    AdmissionFinalReobserve => "admission.final_reobserve",
    AdmissionIdleScratchCreate => "admission.idle_scratch_create",
    AdmissionIdleScratchWrite => "admission.idle_scratch_write",
    AdmissionIdleScratchFlush => "admission.idle_scratch_flush",
    AdmissionIdlePublish => "admission.idle_publish",
    AdmissionIdleReobserve => "admission.idle_reobserve",
    RecordBoundedRead => "record.bounded_read",
    RecordDecode => "record.decode",
    RecordCanonicalReencode => "record.canonical_reencode",
    RecordBindingValidate => "record.binding_validate",
    RecordScratchCreate => "record.scratch_create",
    RecordScratchWrite => "record.scratch_write",
    RecordScratchFlush => "record.scratch_flush",
    RecordActivePublish => "record.active_publish",
    RecordActiveReobserve => "record.active_reobserve",
    RecordRetirementReserve => "record.retirement_reserve",
    RecordRetireExact => "record.retire_exact",
    RecordRetiredReobserve => "record.retired_reobserve",
    RecordTerminalRelationValidate => "record.terminal_relation_validate",
    NamespaceSourceRetain => "namespace.source_retain",
    NamespaceDestinationReserve => "namespace.destination_reserve",
    NamespacePrePublishReobserve => "namespace.pre_publish_reobserve",
    NamespacePublishNoReplace => "namespace.publish_no_replace",
    NamespacePublishedReobserve => "namespace.published_reobserve",
    NamespaceRetirementReserve => "namespace.retirement_reserve",
    NamespacePreRetireReobserve => "namespace.pre_retire_reobserve",
    NamespaceRetireExact => "namespace.retire_exact",
    NamespaceRetiredReobserve => "namespace.retired_reobserve",
    NamespaceParentBarrier => "namespace.parent_barrier",
    NamespaceParentRevalidate => "namespace.parent_revalidate",
    DurableLeafFirstOpen => "durable_leaf.first_open",
    DurableLeafFirstIdentity => "durable_leaf.first_identity",
    DurableLeafFirstContent => "durable_leaf.first_content",
    DurableLeafFileFlush => "durable_leaf.file_flush",
    DurableLeafNamespaceBarrier => "durable_leaf.namespace_barrier",
    DurableLeafParentRevalidate => "durable_leaf.parent_revalidate",
    DurableLeafNameRevalidate => "durable_leaf.name_revalidate",
    DurableLeafHandleRevalidate => "durable_leaf.handle_revalidate",
    DurableLeafLengthRevalidate => "durable_leaf.length_revalidate",
    DurableLeafContentRevalidate => "durable_leaf.content_revalidate",
    DurableLeafMissingRevalidate => "durable_leaf.missing_revalidate",
    BarrierIntentScratchCreate => "barrier.intent_scratch_create",
    BarrierIntentScratchWrite => "barrier.intent_scratch_write",
    BarrierIntentScratchFlush => "barrier.intent_scratch_flush",
    BarrierIntentPublish => "barrier.intent_publish",
    BarrierIntentReobserve => "barrier.intent_reobserve",
    BarrierAnchorOutbound => "barrier.anchor_outbound",
    BarrierAnchorOutboundReobserve => "barrier.anchor_outbound_reobserve",
    BarrierTargetBarrier => "barrier.target_barrier",
    BarrierTargetReobserve => "barrier.target_reobserve",
    BarrierAnchorReturn => "barrier.anchor_return",
    BarrierAnchorReturnReobserve => "barrier.anchor_return_reobserve",
    BarrierTargetAliasRetire => "barrier.target_alias_retire",
    BarrierTargetAliasReobserve => "barrier.target_alias_reobserve",
    BarrierIntentRetire => "barrier.intent_retire",
    BarrierIntentRetiredReobserve => "barrier.intent_retired_reobserve",
    BarrierCompletionReobserve => "barrier.completion_reobserve",
    ManagedBootstrapPreflight => "managed_bootstrap.preflight",
    ManagedBootstrapInitialIntentScratchCreate => "managed_bootstrap.initial_intent_scratch_create",
    ManagedBootstrapInitialIntentScratchWrite => "managed_bootstrap.initial_intent_scratch_write",
    ManagedBootstrapInitialIntentScratchFlush => "managed_bootstrap.initial_intent_scratch_flush",
    ManagedBootstrapInitialIntentPublish => "managed_bootstrap.initial_intent_publish",
    ManagedBootstrapInitialIntentReobserve => "managed_bootstrap.initial_intent_reobserve",
    ManagedBootstrapComponentReobserve => "managed_bootstrap.component_reobserve",
    ManagedBootstrapStagingDirectoryCreate => "managed_bootstrap.staging_directory_create",
    ManagedBootstrapOwnershipMarkerCreate => "managed_bootstrap.ownership_marker_create",
    ManagedBootstrapOwnershipMarkerWrite => "managed_bootstrap.ownership_marker_write",
    ManagedBootstrapOwnershipMarkerFlush => "managed_bootstrap.ownership_marker_flush",
    ManagedBootstrapStagingDirectoryFlush => "managed_bootstrap.staging_directory_flush",
    ManagedBootstrapStagingDirectoryPublish => "managed_bootstrap.staging_directory_publish",
    ManagedBootstrapFinalDirectoryReopen => "managed_bootstrap.final_directory_reopen",
    ManagedBootstrapFinalDirectoryReobserve => "managed_bootstrap.final_directory_reobserve",
    ManagedBootstrapSuccessorScratchCreate => "managed_bootstrap.successor_scratch_create",
    ManagedBootstrapSuccessorScratchWrite => "managed_bootstrap.successor_scratch_write",
    ManagedBootstrapSuccessorScratchFlush => "managed_bootstrap.successor_scratch_flush",
    ManagedBootstrapSuccessorScratchReobserve => "managed_bootstrap.successor_scratch_reobserve",
    ManagedBootstrapPriorGenerationRetire => "managed_bootstrap.prior_generation_retire",
    ManagedBootstrapPriorGenerationReobserve => "managed_bootstrap.prior_generation_reobserve",
    ManagedBootstrapSuccessorPublish => "managed_bootstrap.successor_publish",
    ManagedBootstrapSuccessorReobserve => "managed_bootstrap.successor_reobserve",
    ManagedBootstrapMarkerRetire => "managed_bootstrap.marker_retire",
    ManagedBootstrapMarkerRetiredReobserve => "managed_bootstrap.marker_retired_reobserve",
    ManagedBootstrapFinalIdentityReobserve => "managed_bootstrap.final_identity_reobserve",
    ManagedBootstrapFinalIntentRetire => "managed_bootstrap.final_intent_retire",
    ManagedBootstrapFinalIntentRetiredReobserve => "managed_bootstrap.final_intent_retired_reobserve",
    ManagedBootstrapParentRevalidate => "managed_bootstrap.parent_revalidate",
    ManagedBootstrapPlanComplete => "managed_bootstrap.plan_complete",
    CleanupWorklistScratchCreate => "cleanup.worklist_scratch_create",
    CleanupWorklistScratchWrite => "cleanup.worklist_scratch_write",
    CleanupWorklistScratchFlush => "cleanup.worklist_scratch_flush",
    CleanupWorklistPublish => "cleanup.worklist_publish",
    CleanupWorklistReobserve => "cleanup.worklist_reobserve",
    CleanupSourceReobserve => "cleanup.source_reobserve",
    CleanupDestinationReobserve => "cleanup.destination_reobserve",
    CleanupAliasRetire => "cleanup.alias_retire",
    CleanupRetiredAliasReobserve => "cleanup.retired_alias_reobserve",
    CleanupRowComplete => "cleanup.row_complete",
    CleanupCompletionReobserve => "cleanup.completion_reobserve",
    TerminalAuthorityReobserve => "terminal.authority_reobserve",
    TerminalPayloadReobserve => "terminal.payload_reobserve",
    TerminalCleanupReobserve => "terminal.cleanup_reobserve",
    TerminalReservationReobserve => "terminal.reservation_reobserve",
    TerminalDirectoryFlush => "terminal.directory_flush",
    TerminalRetiredSlotReserve => "terminal.retired_slot_reserve",
    TerminalActionDirectoryRetire => "terminal.action_directory_retire",
    TerminalRetiredDirectoryReobserve => "terminal.retired_directory_reobserve",
    TerminalCatalogBarrier => "terminal.catalog_barrier",
    TerminalTerminalRevalidate => "terminal.terminal_revalidate",
    TerminalAuthorityRelease => "terminal.authority_release",
}

#[cfg(test)]
type FaultCallbackV1 = (CheckedArtifactFaultKeyV1, Box<dyn FnOnce()>);

#[cfg(test)]
thread_local! {
    static NEXT_FAULT: std::cell::RefCell<Option<FaultCallbackV1>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn run_next_at(key: CheckedArtifactFaultKeyV1, callback: impl FnOnce() + 'static) {
    NEXT_FAULT.with(|slot| {
        let previous = slot.replace(Some((key, Box::new(callback))));
        assert!(
            previous.is_none(),
            "checked-artifact fault already installed"
        );
    });
}

#[cfg(test)]
pub(super) fn hit(key: CheckedArtifactFaultKeyV1) {
    NEXT_FAULT.with(|slot| {
        let execute = {
            let mut slot = slot.borrow_mut();
            match slot.as_ref() {
                Some((expected, _)) if *expected == key => slot.take(),
                _ => None,
            }
        };
        if let Some((_, callback)) = execute {
            callback();
        }
    });
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FaultInstanceV1 {
    key: CheckedArtifactFaultKeyV1,
    ordinal: Option<u8>,
    label: Option<String>,
}

impl FaultInstanceV1 {
    pub(super) fn new(
        key: CheckedArtifactFaultKeyV1,
        ordinal: Option<u8>,
        label: Option<String>,
    ) -> Self {
        Self {
            key,
            ordinal,
            label,
        }
    }
}
