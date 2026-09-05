//! `gwz-refcopy`: the product tree copier (lane R).
//!
//! [`SystemTreeCopier`] implements `gwz_copy_contract::TreeCopier`: it copies
//! a whole tree with exclusions applied during traversal, following the
//! traversal, metadata, fallback and reporting rules of gwz-dev
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §3.
//!
//! State of this build: the **ordinary** copy path is implemented and passes
//! the contract's conformance suite (`gwz_copy_contract::contract_tests::
//! run_all`). The native copy-on-write path is **not** in this build: Apple
//! `clonefile`, Linux `FICLONE` and Windows block cloning each need a
//! platform dependency (`libc`/`rustix`/`windows-sys`) that the local-clone
//! boundary gate does not admit yet. Nothing here claims a native mechanism
//! ran: [`SystemTreeCopier::probe_native`] reports
//! [`NativeCapability::Unavailable`], [`SystemTreeCopier::mechanism`] reports
//! [`NativeMechanism::None`], every report counts its files as
//! `ordinary_files`, and `CopyMode::Auto` carries one copy-wide
//! `CopyWarningKind::NativeUnavailable` warning saying so (design §12,
//! "Native copy unavailable -> ordinary independent copy; actual method
//! reported"); `NativeUnsupportedFellBack` is reserved for a native attempt
//! that was made and rejected per entry, which this build never makes.

#![forbid(unsafe_code)]

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemTreeCopier {
    mechanism: Option<NativeMechanism>,
}

impl SystemTreeCopier {
    pub fn new() -> Self {
        Self::default()
    }

    /// The native mechanism this copier would attempt in `CopyMode::Auto`.
    pub fn mechanism(&self) -> NativeMechanism {
        self.mechanism.unwrap_or(NativeMechanism::None)
    }

    /// Hint whether `source` and `destination` may share a native
    /// copy-on-write path. Never authoritative.
    ///
    /// This build links no platform copy-on-write dependency, so the honest
    /// answer for every pair is [`NativeCapability::Unavailable`]: there is no
    /// mechanism to attempt, and a probe must not suggest one exists.
    /// [`NativeCapability::Unknown`] returns when a mechanism is linked but
    /// the pair has not been examined.
    pub fn probe_native(&self, _source: &Path, _destination: &Path) -> NativeCapability {
        match self.mechanism {
            None => NativeCapability::Unavailable,
            Some(NativeMechanism::None) => NativeCapability::Unavailable,
            Some(_) => NativeCapability::Unknown,
        }
    }
}

impl TreeCopier for SystemTreeCopier {
    /// Copy `request`'s source tree, ordinarily.
    ///
    /// `CopyMode::Auto` and `CopyMode::OrdinaryOnly` do the same work here,
    /// because no native mechanism is linked; `Auto` additionally warns that
    /// native copy-on-write was unavailable. See [`ordinary`] for the shape of
    /// a copy, its admission rules and its error classification.
    fn copy_tree(
        &self,
        request: &CopyRequest,
        cancellation: &dyn Cancellation,
    ) -> Result<CopyReport, CopyError> {
        ordinary::copy_tree(request, cancellation)
    }
}

#[cfg(test)]
mod tests;
