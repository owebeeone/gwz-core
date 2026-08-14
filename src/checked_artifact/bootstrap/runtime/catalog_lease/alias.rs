//! Bounded native-name proof for fixed catalog lease slots.

use std::ffi::OsStr;

use cap_std::fs::Dir;

use crate::checked_artifact::capability::{
    CheckedFsError, HostPlatform, PathComponentMode, PathEquivalenceProvider,
};

pub(super) const MAX_CATALOG_ALIAS_PARENT_ENTRIES_V1: usize = 4_096;
const MAX_CATALOG_ALIAS_NAME_UNITS_V1: usize = 255;
const MAX_CATALOG_ALIAS_NAME_BYTES_V1: usize = 510;
const MAX_CATALOG_ALIAS_AGGREGATE_BYTES_V1: usize = 2_088_960;

pub(super) fn reject_equivalent_alias(
    parent: &Dir,
    expected: &OsStr,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let mode = HostPlatform.parent_mode(parent)?;
    reject_equivalent_alias_with_mode(parent, expected, mode, label)
}

fn reject_equivalent_alias_with_mode(
    parent: &Dir,
    expected: &OsStr,
    mode: PathComponentMode,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    if mode == PathComponentMode::Sensitive {
        return Ok(());
    }
    let mut entries = 0usize;
    let mut aggregate_bytes = 0usize;
    for entry in parent
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate catalog lease parent", source))?
    {
        let entry =
            entry.map_err(|source| CheckedFsError::io("read catalog lease parent", source))?;
        entries = entries.checked_add(1).ok_or_else(alias_capacity_error)?;
        if entries > MAX_CATALOG_ALIAS_PARENT_ENTRIES_V1 {
            return Err(alias_capacity_error());
        }
        let observed = entry.file_name();
        let (_, encoded_bytes) = native_name_charge(&observed)?;
        aggregate_bytes = aggregate_bytes
            .checked_add(encoded_bytes)
            .ok_or_else(alias_capacity_error)?;
        if aggregate_bytes > MAX_CATALOG_ALIAS_AGGREGATE_BYTES_V1 {
            return Err(alias_capacity_error());
        }
        if ascii_case_equivalent(&observed, expected) && observed != expected {
            return Err(CheckedFsError::ambiguous(
                label,
                "platform-equivalent alias has noncanonical spelling",
            ));
        }
    }
    Ok(())
}

fn native_name_charge(name: &OsStr) -> Result<(usize, usize), CheckedFsError> {
    let (units, bytes) = native_name_lengths(name)?;
    if units > MAX_CATALOG_ALIAS_NAME_UNITS_V1 || bytes > MAX_CATALOG_ALIAS_NAME_BYTES_V1 {
        return Err(alias_capacity_error());
    }
    Ok((units, bytes))
}

#[cfg(unix)]
fn native_name_lengths(name: &OsStr) -> Result<(usize, usize), CheckedFsError> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = name.as_bytes().len();
    Ok((bytes, bytes))
}

#[cfg(windows)]
fn native_name_lengths(name: &OsStr) -> Result<(usize, usize), CheckedFsError> {
    use std::os::windows::ffi::OsStrExt;

    let units = name.encode_wide().count();
    let bytes = units.checked_mul(2).ok_or_else(alias_capacity_error)?;
    Ok((units, bytes))
}

#[cfg(not(any(unix, windows)))]
fn native_name_lengths(_name: &OsStr) -> Result<(usize, usize), CheckedFsError> {
    Err(CheckedFsError::ambiguous(
        "catalog lease alias capacity",
        "native name accounting is unsupported on this platform",
    ))
}

#[cfg(unix)]
fn ascii_case_equivalent(observed: &OsStr, expected: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let observed = observed.as_bytes();
    let expected = expected.as_bytes();
    observed.is_ascii() && expected.is_ascii() && observed.eq_ignore_ascii_case(expected)
}

#[cfg(windows)]
fn ascii_case_equivalent(observed: &OsStr, expected: &OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt;

    let mut observed = observed.encode_wide();
    let mut expected = expected.encode_wide();
    loop {
        match (observed.next(), expected.next()) {
            (None, None) => return true,
            (Some(left), Some(right)) if left <= 0x7f && right <= 0x7f => {
                if (left as u8).to_ascii_lowercase() != (right as u8).to_ascii_lowercase() {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn ascii_case_equivalent(_observed: &OsStr, _expected: &OsStr) -> bool {
    false
}

fn alias_capacity_error() -> CheckedFsError {
    CheckedFsError::ambiguous(
        "catalog lease alias capacity",
        "parent exceeds 4,096 entries, 255 native name units, 510 encoded name bytes per entry, or 2,088,960 encoded bytes in aggregate",
    )
}

#[cfg(test)]
pub(super) fn reject_equivalent_alias_with_mode_for_test(
    parent: &Dir,
    expected: &OsStr,
    mode: PathComponentMode,
) -> Result<(), CheckedFsError> {
    reject_equivalent_alias_with_mode(parent, expected, mode, "catalog lease alias test")
}

#[cfg(test)]
pub(super) fn native_name_charge_for_test(name: &OsStr) -> Result<(usize, usize), CheckedFsError> {
    native_name_charge(name)
}
