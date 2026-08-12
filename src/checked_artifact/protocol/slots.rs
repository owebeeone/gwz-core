//! Exhaustive deterministic root and action-directory name grammar.

use super::bounds::{
    MAX_BARRIER_INVOCATIONS_PER_ACTION, MAX_BOOTSTRAP_INTENT_GENERATIONS,
    MAX_MANAGED_PARENT_COMPONENTS,
};
use super::schedule::ActionDigestV1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum InfrastructureSlotV1 {
    CatalogFormat,
    CatalogAnchorA,
    CatalogAnchorB,
    RoamingAnchorHome,
    RetiredActions,
    RetiredActionsDescriptor,
    CatalogBootstrapRetired,
    ActionAdmissionActive,
    ActionAdmissionScratch,
    ActionAdmissionStaging,
}

impl InfrastructureSlotV1 {
    pub(in crate::checked_artifact) const ALL: &'static [Self] = &[
        Self::CatalogFormat,
        Self::CatalogAnchorA,
        Self::CatalogAnchorB,
        Self::RoamingAnchorHome,
        Self::RetiredActions,
        Self::RetiredActionsDescriptor,
        Self::CatalogBootstrapRetired,
        Self::ActionAdmissionActive,
        Self::ActionAdmissionScratch,
        Self::ActionAdmissionStaging,
    ];

    pub(in crate::checked_artifact) const fn name(self) -> &'static str {
        match self {
            Self::CatalogFormat => "catalog-format-v1",
            Self::CatalogAnchorA => "catalog-anchor-a-v1",
            Self::CatalogAnchorB => "catalog-anchor-b-v1",
            Self::RoamingAnchorHome => "roaming-anchor-home-v1",
            Self::RetiredActions => "retired-actions-v1",
            Self::RetiredActionsDescriptor => "retired-actions-descriptor-v1",
            Self::CatalogBootstrapRetired => "catalog-bootstrap-retired-v1",
            Self::ActionAdmissionActive => "action-admission-active-v1",
            Self::ActionAdmissionScratch => "action-admission-scratch-v1",
            Self::ActionAdmissionStaging => "action-admission-staging-v1",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum BaseActionSlotV1 {
    Reservation,
    Authority,
    SourcePayload,
    GoalPayload,
    AuthorityScratch,
    GoalScratch,
    RecordScratch,
    BarrierIntentScratch,
    BootstrapIntentScratch,
    RetiredSourceAlias,
    RetiredGoalAlias,
    RetiredAuthorityAlias,
    CleanupWorklist,
}

impl BaseActionSlotV1 {
    pub(in crate::checked_artifact) const ALL: &'static [Self] = &[
        Self::Reservation,
        Self::Authority,
        Self::SourcePayload,
        Self::GoalPayload,
        Self::AuthorityScratch,
        Self::GoalScratch,
        Self::RecordScratch,
        Self::BarrierIntentScratch,
        Self::BootstrapIntentScratch,
        Self::RetiredSourceAlias,
        Self::RetiredGoalAlias,
        Self::RetiredAuthorityAlias,
        Self::CleanupWorklist,
    ];

    pub(in crate::checked_artifact) const fn suffix(self) -> &'static str {
        match self {
            Self::Reservation => "reservation",
            Self::Authority => "authority",
            Self::SourcePayload => "source-payload",
            Self::GoalPayload => "goal-payload",
            Self::AuthorityScratch => "authority-scratch",
            Self::GoalScratch => "goal-scratch",
            Self::RecordScratch => "record-scratch",
            Self::BarrierIntentScratch => "barrier-intent-scratch",
            Self::BootstrapIntentScratch => "bootstrap-intent-scratch",
            Self::RetiredSourceAlias => "retired-source-alias",
            Self::RetiredGoalAlias => "retired-goal-alias",
            Self::RetiredAuthorityAlias => "retired-authority-alias",
            Self::CleanupWorklist => "cleanup-worklist",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ActionSlotV1 {
    Base(BaseActionSlotV1),
    BarrierIntentActive(u8),
    BarrierIntentRetired(u8),
    RetiredRoamingAnchorAlias(u8),
    BootstrapIntentActive(u8),
    BootstrapIntentRetired(u8),
    RetiredBootstrapMarker(u8),
}

impl ActionSlotV1 {
    pub(in crate::checked_artifact) fn all() -> Vec<Self> {
        let mut values = BaseActionSlotV1::ALL
            .iter()
            .copied()
            .map(Self::Base)
            .collect::<Vec<_>>();
        for ordinal in 0..MAX_BARRIER_INVOCATIONS_PER_ACTION as u8 {
            values.extend([
                Self::BarrierIntentActive(ordinal),
                Self::BarrierIntentRetired(ordinal),
                Self::RetiredRoamingAnchorAlias(ordinal),
            ]);
        }
        for generation in 0..MAX_BOOTSTRAP_INTENT_GENERATIONS as u8 {
            values.extend([
                Self::BootstrapIntentActive(generation),
                Self::BootstrapIntentRetired(generation),
            ]);
        }
        for component in 0..MAX_MANAGED_PARENT_COMPONENTS as u8 {
            values.push(Self::RetiredBootstrapMarker(component));
        }
        values
    }

    pub(in crate::checked_artifact) fn name(self, action: ActionDigestV1) -> String {
        let suffix = match self {
            Self::Base(slot) => slot.suffix().to_owned(),
            Self::BarrierIntentActive(value) => format!("barrier-intent-active-{value:02}"),
            Self::BarrierIntentRetired(value) => format!("barrier-intent-retired-{value:02}"),
            Self::RetiredRoamingAnchorAlias(value) => {
                format!("retired-roaming-anchor-alias-{value:02}")
            }
            Self::BootstrapIntentActive(value) => {
                format!("bootstrap-intent-active-{value:02}")
            }
            Self::BootstrapIntentRetired(value) => {
                format!("bootstrap-intent-retired-{value:02}")
            }
            Self::RetiredBootstrapMarker(value) => {
                format!("retired-bootstrap-marker-{value:02}")
            }
        };
        format!("action-{}-{suffix}-v1", action.hex())
    }

    pub(in crate::checked_artifact) fn parse(action: ActionDigestV1, value: &[u8]) -> Option<Self> {
        if !value.is_ascii() {
            return None;
        }
        let value = std::str::from_utf8(value).ok()?;
        Self::all()
            .into_iter()
            .find(|slot| slot.name(action) == value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum RootEntryNameV1 {
    Infrastructure(InfrastructureSlotV1),
    ActiveAction(ActionDigestV1),
}

impl RootEntryNameV1 {
    pub(in crate::checked_artifact) fn name(self) -> String {
        match self {
            Self::Infrastructure(slot) => slot.name().to_owned(),
            Self::ActiveAction(action) => format!("action-{}-v1", action.hex()),
        }
    }

    pub(in crate::checked_artifact) fn parse(value: &[u8]) -> Option<Self> {
        if !value.is_ascii() {
            return None;
        }
        let value = std::str::from_utf8(value).ok()?;
        if let Some(slot) = InfrastructureSlotV1::ALL
            .iter()
            .copied()
            .find(|slot| slot.name() == value)
        {
            return Some(Self::Infrastructure(slot));
        }
        let hex = value.strip_prefix("action-")?.strip_suffix("-v1")?;
        Some(Self::ActiveAction(ActionDigestV1::from_hex(hex)?))
    }
}
