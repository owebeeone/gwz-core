//! Target-independent v1 fault vocabulary.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum FaultAxisV1 {
    Runtime,
    CatalogBootstrap,
    Admission,
    Record,
    Namespace,
    DurableLeaf,
    Barrier,
    ManagedBootstrap,
    Cleanup,
    Terminal,
}

impl FaultAxisV1 {
    const ALL: [Self; 10] = [
        Self::Runtime,
        Self::CatalogBootstrap,
        Self::Admission,
        Self::Record,
        Self::Namespace,
        Self::DurableLeaf,
        Self::Barrier,
        Self::ManagedBootstrap,
        Self::Cleanup,
        Self::Terminal,
    ];

    const fn stable_name(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::CatalogBootstrap => "catalog_bootstrap",
            Self::Admission => "admission",
            Self::Record => "record",
            Self::Namespace => "namespace",
            Self::DurableLeaf => "durable_leaf",
            Self::Barrier => "barrier",
            Self::ManagedBootstrap => "managed_bootstrap",
            Self::Cleanup => "cleanup",
            Self::Terminal => "terminal",
        }
    }

    const fn boundaries(self) -> &'static [&'static str] {
        match self {
            Self::Runtime => &[
                "guard_create",
                "locks_parent_create",
                "lease_create",
                "lease_lock",
                "lease_reobserve",
                "lease_release",
                "git_dir_retain",
                "workspace_retain",
                "path_walk",
                "collision_scan",
                "capability_proof",
            ],
            Self::CatalogBootstrap => &[
                "scratch_create",
                "scratch_write",
                "scratch_flush",
                "staging_create",
                "anchor_create",
                "anchor_flush",
                "staging_flush",
                "staging_publish",
                "active_reobserve",
                "retired_publish",
                "catalog_enumerate",
            ],
            Self::Admission => &[
                "capacity_check",
                "preparing_scratch",
                "preparing_replace",
                "staging_create",
                "reservation_create",
                "reservation_write",
                "reservation_flush",
                "staging_flush",
                "staging_publish",
                "final_reobserve",
                "idle_replace",
            ],
            Self::Record => &[
                "bounded_read",
                "decode",
                "canonical_reencode",
                "scratch_create",
                "scratch_write",
                "scratch_flush",
                "active_replace",
                "active_reobserve",
                "intent_publish",
                "intent_reobserve",
                "record_retire",
            ],
            Self::Namespace => &[
                "source_retain",
                "destination_reserve",
                "pre_publish_reobserve",
                "publish_no_replace",
                "published_reobserve",
                "retirement_reserve",
                "pre_retire_reobserve",
                "retire_exact",
                "retired_reobserve",
                "parent_barrier",
                "parent_revalidate",
            ],
            Self::DurableLeaf => &[
                "first_open",
                "first_identity",
                "first_content",
                "file_flush",
                "namespace_barrier",
                "parent_revalidate",
                "name_revalidate",
                "handle_revalidate",
                "length_revalidate",
                "content_revalidate",
                "missing_revalidate",
            ],
            Self::Barrier => &[
                "intent_publish",
                "anchor_outbound",
                "anchor_outbound_reobserve",
                "target_barrier",
                "target_reobserve",
                "anchor_return",
                "anchor_return_reobserve",
                "successor_publish",
                "intent_retire",
                "alias_retire",
                "completion_reobserve",
            ],
            Self::ManagedBootstrap => &[
                "preflight",
                "intent_publish",
                "component_reobserve",
                "component_create",
                "marker_flush",
                "parent_barrier",
                "successor_publish",
                "intent_retire",
                "marker_retire",
                "parent_revalidate",
                "plan_complete",
            ],
            Self::Cleanup => &[
                "worklist_scratch",
                "worklist_write",
                "worklist_flush",
                "worklist_publish",
                "source_reobserve",
                "destination_reobserve",
                "alias_retire",
                "retired_reobserve",
                "row_complete",
                "worklist_reobserve",
                "cleanup_complete",
            ],
            Self::Terminal => &[
                "authority_reobserve",
                "payload_reobserve",
                "cleanup_reobserve",
                "reservation_reobserve",
                "directory_flush",
                "retired_slot_reserve",
                "action_directory_retire",
                "retired_directory_reobserve",
                "catalog_barrier",
                "terminal_revalidate",
                "authority_release",
            ],
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct CheckedArtifactFaultKeyV1 {
    axis: FaultAxisV1,
    boundary: &'static str,
}

impl CheckedArtifactFaultKeyV1 {
    pub(super) fn all() -> Vec<Self> {
        FaultAxisV1::ALL
            .into_iter()
            .flat_map(|axis| {
                axis.boundaries()
                    .iter()
                    .copied()
                    .map(move |boundary| Self { axis, boundary })
            })
            .collect()
    }

    pub(super) fn stable_key(&self) -> String {
        format!("{}.{}", self.axis.stable_name(), self.boundary)
    }
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
