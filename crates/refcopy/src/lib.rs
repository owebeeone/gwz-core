//! `gwz-refcopy`: the product tree copier (lane R).
//!
//! [`SystemTreeCopier`] implements `gwz_copy_contract::TreeCopier`: it copies
//! a whole tree with exclusions applied during traversal, following the
//! traversal, metadata, fallback and reporting rules of gwz-dev
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §3.
//!
//! Two paths, one result. Every regular file is copied either by the
//! platform's native copy-on-write mechanism ([`native`]: Apple `clonefile`
//! on Apple targets, `FICLONE` on Linux, `FSCTL_DUPLICATE_EXTENTS_TO_FILE` on
//! Windows) or by ordinary buffered read/write
//! ([`ordinary`], which is also the whole engine: admission, traversal,
//! metadata and error classification). Which one ran changes nothing an
//! inspection of the destination can see -- contents, entry type, symlink
//! target and permission bits are the same either way, and both sides stay
//! independently writable, because a clone shares physical blocks and is
//! never a hardlink. It changes only [`CopyReport::native_files`] versus
//! `ordinary_files`, which is how a report says how the bytes actually
//! arrived (design §12, "Native copy unavailable -> ordinary independent
//! copy; actual method reported").
//!
//! Selection is per file and the operation decides:
//!
//! - `CopyMode::OrdinaryOnly` never attempts a native call, and is never
//!   promised one, so it carries no native warning.
//! - `CopyMode::Auto` attempts one for each regular file, unless no
//!   mechanism is compiled in for this target (see [`native`] for which
//!   targets those are) or [`SystemTreeCopier::probe_native`] has already
//!   ruled the pair out. Then the copy is ordinary and carries one copy-wide
//!   `CopyWarningKind::NativeUnavailable` warning saying why.
//! - An attempt that is made and classified unsupported (an unsupported
//!   filesystem, a cross-device pair) falls back to the ordinary copy for
//!   that file and is reported once as
//!   `CopyWarningKind::NativeUnsupportedFellBack`. A permission, space or
//!   I/O failure is an error, and stops the copy.

// Everywhere but Windows this crate is `forbid(unsafe_code)`: `rustix` gives
// `clonefile` and `FICLONE` safe wrappers, so nothing needs relaxing. Windows
// has no such wrapper -- `windows-sys` is a raw binding and
// `FSCTL_DUPLICATE_EXTENTS_TO_FILE` is reached through `DeviceIoControl` --
// so there the lint is `deny` instead, which an inner `allow` can lift for
// the handful of calls in `native::windows` that make it and nothing else.
// `forbid` is kept for every other target so the relaxation cannot travel.
#![cfg_attr(not(windows), forbid(unsafe_code))]
#![cfg_attr(windows, deny(unsafe_code))]
// `CopyError` is 128 bytes on `x86_64-pc-windows-msvc` (a `PathBuf` is 32
// bytes there), exactly clippy's `result_large_err` threshold; the contract
// allows the lint at `TreeCopier::copy_tree` for the same reason and boxing
// would change a public field every consumer reads. This crate's own
// `Result<_, CopyError>` helpers inherit that decision (LCM1.0c follow-up 3,
// the foreign-target clippy step; lane C, reported to lane R).
#![allow(clippy::result_large_err)]

mod native;
mod ordinary;

use std::path::Path;

use gwz_copy_contract::{Cancellation, CopyError, CopyReport, CopyRequest, TreeCopier};

/// Whether a native copy-on-write path is believed available for a
/// source/destination pair. A probe is a hint; the copy result decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeCapability {
    Unknown,
    Available,
    Unavailable,
}

/// Which native mechanism the copier selected, reported in warnings and
/// smoke tests so native acceptance can assert the native path ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMechanism {
    None,
    AppleClonefile,
    LinuxFiclone,
    WindowsBlockClone,
}

/// The product copier. `new()` selects the platform mechanism at
/// construction; `mechanism()` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemTreeCopier {
    mechanism: NativeMechanism,
}

impl SystemTreeCopier {
    pub fn new() -> Self {
        Self {
            mechanism: native::MECHANISM,
        }
    }

    /// The native mechanism this copier would attempt in `CopyMode::Auto`.
    /// [`NativeMechanism::None`] means this build has no native call for
    /// this target and every file is copied ordinarily.
    pub fn mechanism(&self) -> NativeMechanism {
        self.mechanism
    }

    /// Hint whether `source` and `destination` may share a native
    /// copy-on-write path. Never authoritative.
    ///
    /// [`NativeCapability::Unavailable`] is returned only for what can be
    /// known without copying: no mechanism is compiled in for this target,
    /// or the two paths are on different devices, which no copy-on-write
    /// mechanism can clone across. Otherwise the answer is
    /// [`NativeCapability::Unknown`] -- the necessary conditions hold, and
    /// whether the filesystem actually supports cloning is decided by the
    /// operation, per file.
    ///
    /// [`NativeCapability::Available`] is never returned. Establishing it
    /// would mean either performing a copy, which a probe must not do, or
    /// trusting a filesystem-type table, which the architecture explicitly
    /// declines to make authoritative; the copier itself never relies on the
    /// probe for anything but the cross-device shortcut above.
    pub fn probe_native(&self, source: &Path, destination: &Path) -> NativeCapability {
        match self.mechanism {
            NativeMechanism::None => NativeCapability::Unavailable,
            _ => native::probe(source, destination),
        }
    }
}

impl Default for SystemTreeCopier {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeCopier for SystemTreeCopier {
    /// Copy `request`'s source tree.
    ///
    /// The native plan is decided once, before anything is written; see
    /// [`ordinary`] for the shape of a copy, its admission rules and its
    /// error classification, and [`native`] for what a failed native attempt
    /// is read to mean.
    fn copy_tree(
        &self,
        request: &CopyRequest,
        cancellation: &dyn Cancellation,
    ) -> Result<CopyReport, CopyError> {
        ordinary::copy_tree(request, cancellation, native::Plan::for_request(request))
    }
}

#[cfg(test)]
mod tests;
