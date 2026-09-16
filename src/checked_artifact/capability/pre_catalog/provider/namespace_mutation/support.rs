use crate::checked_artifact::capability::{AsciiComponent, CheckedFsError};
use crate::checked_artifact::protocol::{ActionDigestV1, ActionSlotV1, BaseActionSlotV1};
use std::ffi::OsString;

/// One scheduled base slot of an admitted action, as this owner's leaf type.
///
/// Every name is derived from the admitted action's own digest through the
/// frozen `ActionSlotV1` grammar; this file mints no name, exactly as its
/// header says.
pub(crate) fn slot_leaf(
    action: ActionDigestV1,
    slot: BaseActionSlotV1,
) -> Result<AsciiComponent, CheckedFsError> {
    AsciiComponent::parse(ActionSlotV1::Base(slot).name(action).as_bytes())
}

/// Action slot names are frozen ASCII, so this conversion is total and needs no
/// platform-specific `OsStr` construction.
pub(crate) fn os_name(leaf: &AsciiComponent) -> OsString {
    OsString::from(
        std::str::from_utf8(leaf.as_bytes()).expect("an ASCII component is always valid UTF-8"),
    )
}

pub(crate) fn cleanup_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("action cleanup worklist", detail)
}
