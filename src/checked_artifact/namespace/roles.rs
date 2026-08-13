//! Opaque namespace roles derived from one admitted action schedule.

use std::ops::Range;

use super::backend::{ActionDestination, RetainedDirectory};
use super::{ActionBinding, action_destination};
use crate::checked_artifact::capability::AsciiComponent;
use crate::checked_artifact::protocol::{
    ActionSlotV1, BarrierOrdinalV1, BaseActionSlotV1, BootstrapOrdinalV1, CleanupAliasV1,
    RecordDigestV1, ScheduleErrorV1,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum PublishRoleV1 {
    Authority,
    SourcePayload,
    GoalPayload,
    AuthorityScratch,
    GoalScratch,
    RecordScratch,
    BarrierIntentScratch,
    BootstrapIntentScratch,
    CleanupWorklist,
}

impl PublishRoleV1 {
    pub(super) const fn slot(self) -> BaseActionSlotV1 {
        match self {
            Self::Authority => BaseActionSlotV1::Authority,
            Self::SourcePayload => BaseActionSlotV1::SourcePayload,
            Self::GoalPayload => BaseActionSlotV1::GoalPayload,
            Self::AuthorityScratch => BaseActionSlotV1::AuthorityScratch,
            Self::GoalScratch => BaseActionSlotV1::GoalScratch,
            Self::RecordScratch => BaseActionSlotV1::RecordScratch,
            Self::BarrierIntentScratch => BaseActionSlotV1::BarrierIntentScratch,
            Self::BootstrapIntentScratch => BaseActionSlotV1::BootstrapIntentScratch,
            Self::CleanupWorklist => BaseActionSlotV1::CleanupWorklist,
        }
    }
}

pub(in crate::checked_artifact) struct PublishDestination {
    pub(super) binding: ActionBinding,
    pub(super) destination: ActionDestination,
}

impl PublishDestination {
    pub(in crate::checked_artifact) fn leaf(&self) -> &AsciiComponent {
        self.destination.leaf()
    }

    pub(in crate::checked_artifact) const fn reservation_binding(&self) -> RecordDigestV1 {
        self.binding.reservation
    }
}

pub(in crate::checked_artifact) struct CleanupRetirementDestination {
    pub(super) binding: ActionBinding,
    pub(super) alias: CleanupAliasV1,
    pub(super) destination: ActionDestination,
}

impl CleanupRetirementDestination {
    pub(in crate::checked_artifact) const fn alias(&self) -> CleanupAliasV1 {
        self.alias
    }
}

/// Provider-issued target retained without exposing a raw leaf to consumers.
pub(in crate::checked_artifact) struct BarrierTarget<DirectoryHandle, Identity, PathProfile> {
    pub(super) parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
    pub(super) leaf: AsciiComponent,
    pub(super) action: crate::checked_artifact::protocol::ActionDigestV1,
    pub(super) reservation: RecordDigestV1,
    pub(super) ordinal: BarrierOrdinalV1,
}

/// Provider-issued managed-parent names from the already bound plan.
pub(in crate::checked_artifact) struct BootstrapTarget<DirectoryHandle, Identity, PathProfile> {
    pub(super) parent: RetainedDirectory<DirectoryHandle, Identity, PathProfile>,
    pub(super) staging_leaf: AsciiComponent,
    pub(super) final_leaf: AsciiComponent,
    pub(super) action: crate::checked_artifact::protocol::ActionDigestV1,
    pub(super) reservation: RecordDigestV1,
    pub(super) component_ordinal: usize,
}

pub(in crate::checked_artifact) struct ScheduledBarrierOrdinal {
    pub(super) binding: ActionBinding,
    pub(super) ordinal: BarrierOrdinalV1,
}

impl ScheduledBarrierOrdinal {
    pub(in crate::checked_artifact) const fn ordinal(&self) -> BarrierOrdinalV1 {
        self.ordinal
    }
}

pub(in crate::checked_artifact) struct BarrierSlots<DirectoryHandle, Identity, PathProfile> {
    pub(super) binding: ActionBinding,
    pub(super) ordinal: BarrierOrdinalV1,
    pub(super) active: ActionDestination,
    pub(super) retired: ActionDestination,
    pub(super) retired_anchor_alias: ActionDestination,
    pub(super) target: BarrierTarget<DirectoryHandle, Identity, PathProfile>,
}

impl<DirectoryHandle, Identity, PathProfile> BarrierSlots<DirectoryHandle, Identity, PathProfile> {
    pub(in crate::checked_artifact) const fn ordinal(&self) -> BarrierOrdinalV1 {
        self.ordinal
    }

    pub(in crate::checked_artifact) fn active_leaf(&self) -> &AsciiComponent {
        self.active.leaf()
    }

    pub(in crate::checked_artifact) fn retired_leaf(&self) -> &AsciiComponent {
        self.retired.leaf()
    }

    pub(in crate::checked_artifact) fn retired_anchor_alias_leaf(&self) -> &AsciiComponent {
        self.retired_anchor_alias.leaf()
    }

    pub(in crate::checked_artifact) fn target_leaf(&self) -> &AsciiComponent {
        &self.target.leaf
    }
}

pub(in crate::checked_artifact) struct BootstrapSlots {
    pub(super) binding: ActionBinding,
    pub(super) bootstrap_ordinal: BootstrapOrdinalV1,
    pub(super) generation_range: Range<usize>,
    pub(super) component_range: Range<usize>,
}

pub(in crate::checked_artifact) struct BootstrapGenerationSlots {
    binding: ActionBinding,
    active: ActionDestination,
    retired: ActionDestination,
    scratch: ActionDestination,
}

impl BootstrapGenerationSlots {
    pub(in crate::checked_artifact) fn active_leaf(&self) -> &AsciiComponent {
        self.active.leaf()
    }

    pub(in crate::checked_artifact) fn retired_leaf(&self) -> &AsciiComponent {
        self.retired.leaf()
    }

    pub(in crate::checked_artifact) fn scratch_leaf(&self) -> &AsciiComponent {
        self.scratch.leaf()
    }
}

pub(in crate::checked_artifact) struct BootstrapComponentSlots<
    DirectoryHandle,
    Identity,
    PathProfile,
> {
    pub(super) binding: ActionBinding,
    pub(super) bootstrap_ordinal: BootstrapOrdinalV1,
    pub(super) global_component_ordinal: usize,
    pub(super) target: BootstrapTarget<DirectoryHandle, Identity, PathProfile>,
    pub(super) marker_retired: ActionDestination,
}

impl<DirectoryHandle, Identity, PathProfile>
    BootstrapComponentSlots<DirectoryHandle, Identity, PathProfile>
{
    pub(in crate::checked_artifact) fn staging_leaf(&self) -> &AsciiComponent {
        &self.target.staging_leaf
    }

    pub(in crate::checked_artifact) fn final_leaf(&self) -> &AsciiComponent {
        &self.target.final_leaf
    }

    pub(in crate::checked_artifact) fn marker_retired_leaf(&self) -> &AsciiComponent {
        self.marker_retired.leaf()
    }
}

impl BootstrapSlots {
    #[cfg(test)]
    pub(in crate::checked_artifact) fn component_range_for_test(&self) -> Range<usize> {
        self.component_range.clone()
    }

    pub(in crate::checked_artifact) fn generation(
        &self,
        ordinal: usize,
    ) -> Result<BootstrapGenerationSlots, ScheduleErrorV1> {
        if !self.generation_range.contains(&ordinal) {
            return Err(ScheduleErrorV1::OutOfBounds);
        }
        Ok(BootstrapGenerationSlots {
            binding: self.binding,
            active: action_destination(
                self.binding,
                ActionSlotV1::BootstrapIntentActive(ordinal as u8),
            ),
            retired: action_destination(
                self.binding,
                ActionSlotV1::BootstrapIntentRetired(ordinal as u8),
            ),
            scratch: action_destination(
                self.binding,
                ActionSlotV1::Base(BaseActionSlotV1::BootstrapIntentScratch),
            ),
        })
    }

    pub(in crate::checked_artifact) fn component<DirectoryHandle, Identity, PathProfile>(
        &self,
        ordinal: usize,
        target: BootstrapTarget<DirectoryHandle, Identity, PathProfile>,
    ) -> Result<BootstrapComponentSlots<DirectoryHandle, Identity, PathProfile>, ScheduleErrorV1>
    {
        if !self.component_range.contains(&ordinal)
            || target.parent.provider() != self.binding.provider
            || target.action != self.binding.action
            || target.reservation != self.binding.reservation
            || target.component_ordinal != ordinal
        {
            return Err(ScheduleErrorV1::OutOfBounds);
        }
        Ok(BootstrapComponentSlots {
            binding: self.binding,
            bootstrap_ordinal: self.bootstrap_ordinal,
            global_component_ordinal: ordinal,
            target,
            marker_retired: action_destination(
                self.binding,
                ActionSlotV1::RetiredBootstrapMarker(ordinal as u8),
            ),
        })
    }
}
