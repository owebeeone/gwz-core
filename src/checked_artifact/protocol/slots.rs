//! Exhaustive deterministic root and action-directory name grammar.

use super::bounds::{
    MAX_BARRIER_INVOCATIONS_PER_ACTION, MAX_BOOTSTRAP_INTENT_GENERATIONS,
    MAX_MANAGED_PARENT_COMPONENTS, MAX_NAME_BYTES,
};
use super::schedule::ActionDigestV1;

const PROTOCOL_VERSION_SUFFIX: &str = "-v1";
const ACTION_PREFIX: &str = "action-";
const ACTION_DIGEST_HEX_WIDTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogNameClassificationV1<T> {
    Valid(T),
    RecognizedInvalid(CatalogNameInvalidReasonV1),
    Foreign,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogNameInvalidReasonV1 {
    NonAscii,
    NameTooLong,
    UnsupportedVersion,
    InvalidActionDigestWidth,
    InvalidActionDigestEncoding,
    ActionDigestMismatch,
    InvalidOrdinalWidth,
    InvalidOrdinalEncoding,
    OrdinalOutOfRange,
    UnknownSlotRole,
    MalformedInfrastructureName,
}

#[cfg(test)]
impl<T: PartialEq> PartialEq<Option<T>> for CatalogNameClassificationV1<T> {
    fn eq(&self, other: &Option<T>) -> bool {
        matches!((self, other), (Self::Valid(value), Some(expected)) if value == expected)
            || matches!(
                (self, other),
                (Self::RecognizedInvalid(_) | Self::Foreign, None)
            )
    }
}

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

    fn recognizes(self, value: &str) -> bool {
        value.starts_with(
            self.name()
                .strip_suffix(PROTOCOL_VERSION_SUFFIX)
                .expect("infrastructure names have the protocol suffix"),
        )
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
            value => {
                let (grammar, ordinal) = value
                    .dynamic_parts()
                    .expect("every non-base slot has a dynamic grammar");
                format!("{}{ordinal:02}", grammar.prefix())
            }
        };
        format!(
            "{ACTION_PREFIX}{}-{suffix}{PROTOCOL_VERSION_SUFFIX}",
            action.hex()
        )
    }

    fn dynamic_parts(self) -> Option<(DynamicActionSlotGrammarV1, u8)> {
        match self {
            Self::Base(_) => None,
            Self::BarrierIntentActive(value) => {
                Some((DynamicActionSlotGrammarV1::BarrierIntentActive, value))
            }
            Self::BarrierIntentRetired(value) => {
                Some((DynamicActionSlotGrammarV1::BarrierIntentRetired, value))
            }
            Self::RetiredRoamingAnchorAlias(value) => {
                Some((DynamicActionSlotGrammarV1::RetiredRoamingAnchorAlias, value))
            }
            Self::BootstrapIntentActive(value) => {
                Some((DynamicActionSlotGrammarV1::BootstrapIntentActive, value))
            }
            Self::BootstrapIntentRetired(value) => {
                Some((DynamicActionSlotGrammarV1::BootstrapIntentRetired, value))
            }
            Self::RetiredBootstrapMarker(value) => {
                Some((DynamicActionSlotGrammarV1::RetiredBootstrapMarker, value))
            }
        }
    }

    pub(in crate::checked_artifact) fn parse(
        action: ActionDigestV1,
        value: &[u8],
    ) -> CatalogNameClassificationV1<Self> {
        use CatalogNameClassificationV1::{Foreign, RecognizedInvalid, Valid};
        use CatalogNameInvalidReasonV1::*;

        if !value.starts_with(ACTION_PREFIX.as_bytes()) {
            return Foreign;
        }
        if value.len() > MAX_NAME_BYTES {
            return RecognizedInvalid(NameTooLong);
        }
        if !value.is_ascii() {
            return RecognizedInvalid(NonAscii);
        }
        let value = std::str::from_utf8(value).expect("ASCII was checked");
        let Some(unversioned) = value.strip_suffix(PROTOCOL_VERSION_SUFFIX) else {
            return RecognizedInvalid(UnsupportedVersion);
        };
        let remainder = &unversioned[ACTION_PREFIX.len()..];
        let Some((digest, role)) = remainder.split_once('-') else {
            return RecognizedInvalid(match parse_action_digest(remainder) {
                Ok(_) => UnknownSlotRole,
                Err(reason) => reason,
            });
        };
        let parsed_action = match parse_action_digest(digest) {
            Ok(value) => value,
            Err(reason) => return RecognizedInvalid(reason),
        };
        if parsed_action != action {
            return RecognizedInvalid(ActionDigestMismatch);
        }
        if let Some(slot) = BaseActionSlotV1::ALL
            .iter()
            .copied()
            .find(|slot| slot.suffix() == role)
        {
            return Valid(Self::Base(slot));
        }
        for grammar in DynamicActionSlotGrammarV1::ALL {
            if let Some(ordinal) = role.strip_prefix(grammar.prefix()) {
                return match parse_ordinal(ordinal, grammar.limit()) {
                    Ok(value) => Valid(grammar.slot(value)),
                    Err(reason) => RecognizedInvalid(reason),
                };
            }
        }
        RecognizedInvalid(UnknownSlotRole)
    }
}

#[derive(Clone, Copy)]
enum DynamicActionSlotGrammarV1 {
    BarrierIntentActive,
    BarrierIntentRetired,
    RetiredRoamingAnchorAlias,
    BootstrapIntentActive,
    BootstrapIntentRetired,
    RetiredBootstrapMarker,
}

impl DynamicActionSlotGrammarV1 {
    const ALL: [Self; 6] = [
        Self::BarrierIntentActive,
        Self::BarrierIntentRetired,
        Self::RetiredRoamingAnchorAlias,
        Self::BootstrapIntentActive,
        Self::BootstrapIntentRetired,
        Self::RetiredBootstrapMarker,
    ];

    const fn prefix(self) -> &'static str {
        match self {
            Self::BarrierIntentActive => "barrier-intent-active-",
            Self::BarrierIntentRetired => "barrier-intent-retired-",
            Self::RetiredRoamingAnchorAlias => "retired-roaming-anchor-alias-",
            Self::BootstrapIntentActive => "bootstrap-intent-active-",
            Self::BootstrapIntentRetired => "bootstrap-intent-retired-",
            Self::RetiredBootstrapMarker => "retired-bootstrap-marker-",
        }
    }

    const fn limit(self) -> usize {
        match self {
            Self::BarrierIntentActive
            | Self::BarrierIntentRetired
            | Self::RetiredRoamingAnchorAlias => MAX_BARRIER_INVOCATIONS_PER_ACTION,
            Self::BootstrapIntentActive | Self::BootstrapIntentRetired => {
                MAX_BOOTSTRAP_INTENT_GENERATIONS
            }
            Self::RetiredBootstrapMarker => MAX_MANAGED_PARENT_COMPONENTS,
        }
    }

    const fn slot(self, value: u8) -> ActionSlotV1 {
        match self {
            Self::BarrierIntentActive => ActionSlotV1::BarrierIntentActive(value),
            Self::BarrierIntentRetired => ActionSlotV1::BarrierIntentRetired(value),
            Self::RetiredRoamingAnchorAlias => ActionSlotV1::RetiredRoamingAnchorAlias(value),
            Self::BootstrapIntentActive => ActionSlotV1::BootstrapIntentActive(value),
            Self::BootstrapIntentRetired => ActionSlotV1::BootstrapIntentRetired(value),
            Self::RetiredBootstrapMarker => ActionSlotV1::RetiredBootstrapMarker(value),
        }
    }
}

fn parse_action_digest(value: &str) -> Result<ActionDigestV1, CatalogNameInvalidReasonV1> {
    if value.len() != ACTION_DIGEST_HEX_WIDTH {
        return Err(CatalogNameInvalidReasonV1::InvalidActionDigestWidth);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(CatalogNameInvalidReasonV1::InvalidActionDigestEncoding);
    }
    ActionDigestV1::from_hex(value).ok_or(CatalogNameInvalidReasonV1::InvalidActionDigestEncoding)
}

fn parse_ordinal(value: &str, limit: usize) -> Result<u8, CatalogNameInvalidReasonV1> {
    if value.len() != 2 {
        return Err(CatalogNameInvalidReasonV1::InvalidOrdinalWidth);
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CatalogNameInvalidReasonV1::InvalidOrdinalEncoding);
    }
    let value = value
        .parse::<u8>()
        .map_err(|_| CatalogNameInvalidReasonV1::InvalidOrdinalEncoding)?;
    if usize::from(value) >= limit {
        return Err(CatalogNameInvalidReasonV1::OrdinalOutOfRange);
    }
    Ok(value)
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
            Self::ActiveAction(action) => {
                format!("{ACTION_PREFIX}{}{PROTOCOL_VERSION_SUFFIX}", action.hex())
            }
        }
    }

    pub(in crate::checked_artifact) fn parse(value: &[u8]) -> CatalogNameClassificationV1<Self> {
        use CatalogNameClassificationV1::{Foreign, RecognizedInvalid, Valid};
        use CatalogNameInvalidReasonV1::*;

        let recognized_non_ascii = value.starts_with(ACTION_PREFIX.as_bytes())
            || InfrastructureSlotV1::ALL.iter().any(|slot| {
                value.starts_with(
                    slot.name()
                        .strip_suffix(PROTOCOL_VERSION_SUFFIX)
                        .expect("infrastructure names have the protocol suffix")
                        .as_bytes(),
                )
            });
        if !value.is_ascii() {
            return if recognized_non_ascii {
                RecognizedInvalid(NonAscii)
            } else {
                Foreign
            };
        }
        if value.len() > MAX_NAME_BYTES {
            return if recognized_non_ascii {
                RecognizedInvalid(NameTooLong)
            } else {
                Foreign
            };
        }
        let value = std::str::from_utf8(value).expect("ASCII was checked");
        if let Some(slot) = InfrastructureSlotV1::ALL
            .iter()
            .copied()
            .find(|slot| slot.name() == value)
        {
            return Valid(Self::Infrastructure(slot));
        }
        if let Some(slot) = InfrastructureSlotV1::ALL
            .iter()
            .copied()
            .find(|slot| slot.recognizes(value))
        {
            return RecognizedInvalid(
                if value.starts_with(
                    slot.name()
                        .strip_suffix(PROTOCOL_VERSION_SUFFIX)
                        .expect("infrastructure names have the protocol suffix"),
                ) && value.contains("-v")
                {
                    UnsupportedVersion
                } else {
                    MalformedInfrastructureName
                },
            );
        }
        if !value.starts_with(ACTION_PREFIX) {
            return Foreign;
        }
        let Some(hex) = value
            .strip_prefix(ACTION_PREFIX)
            .and_then(|name| name.strip_suffix(PROTOCOL_VERSION_SUFFIX))
        else {
            return RecognizedInvalid(UnsupportedVersion);
        };
        match parse_action_digest(hex) {
            Ok(action) => Valid(Self::ActiveAction(action)),
            Err(reason) => RecognizedInvalid(reason),
        }
    }
}
