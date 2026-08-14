use std::ffi::OsStr;

use crate::checked_artifact::capability::{CheckedFsError, PathComponentMode, PlatformCapability};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;

use super::CatalogScratchNameV1;

pub(in crate::checked_artifact) const MAX_CATALOG_PARENT_ENTRIES_V1: usize = 4_096;
pub(in crate::checked_artifact) const MAX_CATALOG_NATIVE_NAME_UNITS_V1: usize = 255;
pub(in crate::checked_artifact) const MAX_CATALOG_ENCODED_NAME_BYTES_V1: usize = 510;
pub(in crate::checked_artifact) const MAX_CATALOG_AGGREGATE_NAME_BYTES_V1: usize =
    MAX_CATALOG_PARENT_ENTRIES_V1 * MAX_CATALOG_ENCODED_NAME_BYTES_V1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogNativeNameV1 {
    ascii_bytes: Option<Vec<u8>>,
    native_units: usize,
    encoded_bytes: usize,
}

impl CatalogNativeNameV1 {
    pub(in crate::checked_artifact) fn unix(bytes: Vec<u8>) -> Result<Self, CheckedFsError> {
        let length = bytes.len();
        let ascii_bytes = bytes.is_ascii().then_some(bytes);
        Self::new(ascii_bytes, length, length)
    }

    pub(in crate::checked_artifact) fn windows(units: Vec<u16>) -> Result<Self, CheckedFsError> {
        let native_units = units.len();
        let encoded_bytes = native_units.checked_mul(2).ok_or_else(capacity_error)?;
        let ascii = units.iter().all(|unit| *unit <= 0x7f);
        let mut ascii_bytes = Vec::new();
        ascii_bytes.try_reserve_exact(native_units).map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "catalog native-name allocation failed",
            )
        })?;
        if ascii {
            ascii_bytes.extend(units.into_iter().map(|unit| unit as u8));
        }
        Self::new(ascii.then_some(ascii_bytes), native_units, encoded_bytes)
    }

    fn new(
        ascii_bytes: Option<Vec<u8>>,
        native_units: usize,
        encoded_bytes: usize,
    ) -> Result<Self, CheckedFsError> {
        if native_units == 0
            || native_units > MAX_CATALOG_NATIVE_NAME_UNITS_V1
            || encoded_bytes > MAX_CATALOG_ENCODED_NAME_BYTES_V1
        {
            return Err(capacity_error());
        }
        Ok(Self {
            ascii_bytes,
            native_units,
            encoded_bytes,
        })
    }

    fn ascii_bytes(&self) -> Option<&[u8]> {
        self.ascii_bytes.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogRecognizedNameV1 {
    Scratch(Box<CatalogScratchNameV1>),
    Active,
    Staging,
    Final,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogParentObservationV1 {
    entry_count: usize,
    encoded_name_bytes: usize,
    recognized_count: usize,
    scratch_candidates: usize,
}

impl CatalogParentObservationV1 {
    pub(in crate::checked_artifact) const fn empty() -> Self {
        Self {
            entry_count: 0,
            encoded_name_bytes: 0,
            recognized_count: 0,
            scratch_candidates: 0,
        }
    }

    pub(in crate::checked_artifact) const fn entry_count(&self) -> usize {
        self.entry_count
    }

    pub(in crate::checked_artifact) const fn encoded_name_bytes(&self) -> usize {
        self.encoded_name_bytes
    }

    pub(in crate::checked_artifact) const fn recognized_count(&self) -> usize {
        self.recognized_count
    }

    pub(in crate::checked_artifact) const fn scratch_candidates(&self) -> usize {
        self.scratch_candidates
    }
}

pub(in crate::checked_artifact) struct CatalogParentGrammarV1 {
    mode: PathComponentMode,
}

impl CatalogParentGrammarV1 {
    pub(in crate::checked_artifact) const fn new(mode: PathComponentMode) -> Self {
        Self { mode }
    }

    pub(in crate::checked_artifact) fn classify(
        &self,
        names: impl IntoIterator<Item = CatalogNativeNameV1>,
    ) -> Result<CatalogParentObservationV1, CheckedFsError> {
        let mut scanner = CatalogParentScannerV1::new(self.mode);
        for name in names {
            scanner.observe_owned(&name)?;
        }
        Ok(scanner.finish())
    }

    pub(in crate::checked_artifact) const fn scanner(&self) -> CatalogParentScannerV1 {
        CatalogParentScannerV1::new(self.mode)
    }
}

pub(in crate::checked_artifact) struct CatalogParentScannerV1 {
    mode: PathComponentMode,
    budget: CatalogNameBudgetV1,
    scratch: usize,
    active: usize,
    staging: usize,
    final_directory: usize,
}

impl CatalogParentScannerV1 {
    pub(in crate::checked_artifact) const fn new(mode: PathComponentMode) -> Self {
        Self {
            mode,
            budget: CatalogNameBudgetV1::new(),
            scratch: 0,
            active: 0,
            staging: 0,
            final_directory: 0,
        }
    }

    fn observe_owned(
        &mut self,
        name: &CatalogNativeNameV1,
    ) -> Result<Option<CatalogRecognizedNameV1>, CheckedFsError> {
        self.budget.charge(name.native_units, name.encoded_bytes)?;
        reject_non_ascii_case_fold_name(self.mode, name.ascii_bytes().is_some())?;
        self.observe_ascii(name.ascii_bytes())
    }

    pub(in crate::checked_artifact) fn observe_os_str(
        &mut self,
        name: &OsStr,
    ) -> Result<Option<CatalogRecognizedNameV1>, CheckedFsError> {
        let (native_units, encoded_bytes) = native_charge(name)?;
        self.budget.charge(native_units, encoded_bytes)?;
        let ascii = NativeAsciiNameV1::from_os_str(name);
        reject_non_ascii_case_fold_name(self.mode, ascii.is_some())?;
        self.observe_ascii(ascii.as_ref().map(NativeAsciiNameV1::as_bytes))
    }

    fn observe_ascii(
        &mut self,
        bytes: Option<&[u8]>,
    ) -> Result<Option<CatalogRecognizedNameV1>, CheckedFsError> {
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let role = classify_ascii(bytes, self.mode)?;
        if let Some(role) = &role {
            let count = match role {
                CatalogRecognizedNameV1::Scratch(_) => &mut self.scratch,
                CatalogRecognizedNameV1::Active => &mut self.active,
                CatalogRecognizedNameV1::Staging => &mut self.staging,
                CatalogRecognizedNameV1::Final => &mut self.final_directory,
            };
            *count += 1;
            if *count > 1 {
                return Err(CheckedFsError::ambiguous(
                    "catalog parent grammar",
                    "duplicate reserved catalog role",
                ));
            }
        }
        Ok(role)
    }

    pub(in crate::checked_artifact) fn finish(self) -> CatalogParentObservationV1 {
        CatalogParentObservationV1 {
            entry_count: self.budget.entry_count,
            encoded_name_bytes: self.budget.encoded_name_bytes,
            recognized_count: self.scratch + self.active + self.staging + self.final_directory,
            scratch_candidates: self.scratch,
        }
    }
}

pub(in crate::checked_artifact) struct CatalogNameBudgetV1 {
    entry_count: usize,
    encoded_name_bytes: usize,
}

impl CatalogNameBudgetV1 {
    pub(in crate::checked_artifact) const fn new() -> Self {
        Self {
            entry_count: 0,
            encoded_name_bytes: 0,
        }
    }

    pub(in crate::checked_artifact) fn charge_os_str(
        &mut self,
        name: &OsStr,
    ) -> Result<(), CheckedFsError> {
        let (native_units, encoded_bytes) = native_charge(name)?;
        self.charge(native_units, encoded_bytes)
    }

    pub(in crate::checked_artifact) const fn entry_count(&self) -> usize {
        self.entry_count
    }

    pub(in crate::checked_artifact) const fn encoded_name_bytes(&self) -> usize {
        self.encoded_name_bytes
    }

    fn charge(&mut self, native_units: usize, encoded_bytes: usize) -> Result<(), CheckedFsError> {
        self.entry_count = self.entry_count.checked_add(1).ok_or_else(capacity_error)?;
        self.encoded_name_bytes = self
            .encoded_name_bytes
            .checked_add(encoded_bytes)
            .ok_or_else(capacity_error)?;
        if self.entry_count > MAX_CATALOG_PARENT_ENTRIES_V1
            || native_units == 0
            || native_units > MAX_CATALOG_NATIVE_NAME_UNITS_V1
            || encoded_bytes > MAX_CATALOG_ENCODED_NAME_BYTES_V1
            || self.encoded_name_bytes > MAX_CATALOG_AGGREGATE_NAME_BYTES_V1
        {
            return Err(capacity_error());
        }
        Ok(())
    }
}

pub(in crate::checked_artifact) fn native_name_matches_ascii(
    observed: &OsStr,
    expected: &[u8],
    mode: PathComponentMode,
) -> Result<bool, CheckedFsError> {
    let observed = NativeAsciiNameV1::from_os_str(observed);
    reject_non_ascii_case_fold_name(mode, observed.is_some())?;
    Ok(observed.is_some_and(|observed| match mode {
        PathComponentMode::Sensitive => observed.as_bytes() == expected,
        PathComponentMode::AsciiCaseFold => observed.as_bytes().eq_ignore_ascii_case(expected),
    }))
}

fn reject_non_ascii_case_fold_name(
    mode: PathComponentMode,
    is_ascii: bool,
) -> Result<(), CheckedFsError> {
    if mode == PathComponentMode::AsciiCaseFold && !is_ascii {
        return Err(alias_error());
    }
    Ok(())
}

fn classify_ascii(
    bytes: &[u8],
    mode: PathComponentMode,
) -> Result<Option<CatalogRecognizedNameV1>, CheckedFsError> {
    for (expected, role) in fixed_roles() {
        if bytes == expected {
            return Ok(Some(role));
        }
        if mode == PathComponentMode::AsciiCaseFold && bytes.eq_ignore_ascii_case(expected) {
            return Err(alias_error());
        }
    }

    let prefix = CatalogScratchNameV1::prefix();
    if bytes.starts_with(prefix) {
        return CatalogScratchNameV1::parse(bytes)
            .map(Box::new)
            .map(CatalogRecognizedNameV1::Scratch)
            .map(Some);
    }
    let family = &prefix[..prefix.len() - 1];
    if bytes == family || (bytes.starts_with(family) && bytes.get(family.len()) == Some(&b'.')) {
        return Err(malformed_error());
    }
    if mode == PathComponentMode::AsciiCaseFold
        && bytes.len() >= family.len()
        && bytes[..family.len()].eq_ignore_ascii_case(family)
        && (bytes.len() == family.len() || bytes.get(family.len()) == Some(&b'.'))
    {
        return Err(alias_error());
    }
    Ok(None)
}

struct NativeAsciiNameV1 {
    bytes: [u8; MAX_CATALOG_NATIVE_NAME_UNITS_V1],
    len: usize,
}

impl NativeAsciiNameV1 {
    fn from_os_str(value: &OsStr) -> Option<Self> {
        let mut result = Self {
            bytes: [0; MAX_CATALOG_NATIVE_NAME_UNITS_V1],
            len: 0,
        };
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bytes = value.as_bytes();
            if bytes.len() > result.bytes.len() || !bytes.is_ascii() {
                return None;
            }
            result.bytes[..bytes.len()].copy_from_slice(bytes);
            result.len = bytes.len();
            Some(result)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            for unit in value.encode_wide() {
                if unit > 0x7f || result.len == result.bytes.len() {
                    return None;
                }
                result.bytes[result.len] = unit as u8;
                result.len += 1;
            }
            Some(result)
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

fn native_charge(name: &OsStr) -> Result<(usize, usize), CheckedFsError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let units = name.as_bytes().len();
        Ok((units, units))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let units = name.encode_wide().count();
        Ok((units, units.checked_mul(2).ok_or_else(capacity_error)?))
    }
}

fn fixed_roles() -> [(&'static [u8], CatalogRecognizedNameV1); 3] {
    [
        (
            CatalogPrivateNameV1::BootstrapActive.leaf_bytes(),
            CatalogRecognizedNameV1::Active,
        ),
        (
            CatalogPrivateNameV1::BootstrapStaging.leaf_bytes(),
            CatalogRecognizedNameV1::Staging,
        ),
        (
            CatalogPrivateNameV1::Final.leaf_bytes(),
            CatalogRecognizedNameV1::Final,
        ),
    ]
}

fn capacity_error() -> CheckedFsError {
    CheckedFsError::ambiguous(
        "catalog parent capacity",
        "parent entry count or native-name budget exceeded",
    )
}

fn malformed_error() -> CheckedFsError {
    CheckedFsError::ambiguous(
        "catalog scratch grammar",
        "recognized scratch-family name is malformed",
    )
}

fn alias_error() -> CheckedFsError {
    CheckedFsError::ambiguous(
        "catalog parent grammar",
        "platform-equivalent reserved name is noncanonical",
    )
}
