use crate::checked_artifact::capability::{AsciiComponent, CheckedFsError, PlatformCapability};
use crate::filesystem::FsDirectory as Dir;
use std::ffi::{OsStr, OsString};

pub(crate) fn managed_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("managed component", detail)
}

/// Managed names are frozen ASCII, so this conversion is total and needs no
/// platform-specific `OsStr` construction (`namespace_mutation.rs:387-393`).
pub(crate) fn os_name(leaf: &AsciiComponent) -> OsString {
    OsString::from(
        std::str::from_utf8(leaf.as_bytes()).expect("an ASCII component is always valid UTF-8"),
    )
}

pub(crate) fn clone_root(root: &super::super::RetainedPlatformRoot) -> Result<Dir, CheckedFsError> {
    root.root()
        .handle()
        .clone_handle()
        .map_err(|source| CheckedFsError::io("retain managed parent root", source))
}

pub(crate) fn prefix_allocation_failure() -> CheckedFsError {
    CheckedFsError::unsupported(
        PlatformCapability::ManagedParentBootstrap,
        "managed parent prefix allocation failed",
    )
}

pub(crate) fn require_bounded_prefix(components: &[AsciiComponent]) -> Result<(), CheckedFsError> {
    if components.is_empty()
        || components.len() > crate::checked_artifact::protocol::MAX_MANAGED_PARENT_COMPONENTS
    {
        return Err(managed_error(
            "managed parent path is outside the frozen component bound",
        ));
    }
    Ok(())
}

/// The deterministic destination row must be free before the edge, stated as a
/// pre-edge expectation so a resident row is a typed refusal rather than an
/// `EEXIST` (`namespace_mutation.rs:317-331`).
pub(crate) fn require_absent_in(
    directory: &Dir,
    name: &OsStr,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    match directory.entry_metadata(name) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(CheckedFsError::io("observe managed destination", source)),
        Ok(_) => Err(CheckedFsError::ambiguous(
            label,
            "managed destination row is already occupied",
        )),
    }
}
