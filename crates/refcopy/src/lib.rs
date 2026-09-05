//! `gwz-refcopy`: the product tree copier (lane R).
//!
//! [`SystemTreeCopier`] implements `gwz_copy_contract::TreeCopier` with the
//! platform's native copy-on-write path (Apple `clonefile`, Linux
//! `FICLONE`, a Windows block-clone/system-copy path) and an ordinary
//! read/write fallback, following the fallback and metadata rules of
//! gwz-dev `dev-docs/GwzLocalCloneImplementationArchitecture.md` §3.
//!
//! LCM1.0c checkpoint state: the type and its capability probe exist with
//! their frozen signatures; `copy_tree` refuses typed
//! ([`CopyErrorCategory::Unimplemented`]) and writes nothing. Lane R's
//! implementation starts from the contract's conformance suite
//! (`gwz_copy_contract::contract_tests::run_all`) failing against this type.

#![forbid(unsafe_code)]

use std::path::Path;

use gwz_copy_contract::{
    Cancellation, CopyError, CopyErrorCategory, CopyReport, CopyRequest, TreeCopier,
};

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
    pub fn probe_native(&self, _source: &Path, _destination: &Path) -> NativeCapability {
        NativeCapability::Unknown
    }
}

impl TreeCopier for SystemTreeCopier {
    fn copy_tree(
        &self,
        request: &CopyRequest,
        _cancellation: &dyn Cancellation,
    ) -> Result<CopyReport, CopyError> {
        Err(CopyError::refused(
            &request.destination,
            CopyErrorCategory::Unimplemented,
            "gwz-refcopy: SystemTreeCopier::copy_tree is not implemented (LCM1.0c checkpoint)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_copy_contract::{CopyMode, NeverCancelled, contract_tests::TempTree};

    #[test]
    fn checkpoint_copier_refuses_typed_and_writes_nothing() {
        let source = TempTree::new("refcopy-source");
        source.file("a.txt", b"alpha");
        let parent = TempTree::new("refcopy-dest");
        let destination = parent.path().join("copy");
        let request = CopyRequest {
            source: source.path().to_path_buf(),
            destination: destination.clone(),
            exclusions: Vec::new(),
            mode: CopyMode::Auto,
        };
        let error = SystemTreeCopier::new()
            .copy_tree(&request, &NeverCancelled)
            .expect_err("the checkpoint copier refuses");
        assert_eq!(error.category, CopyErrorCategory::Unimplemented);
        assert_eq!(error.partial, CopyReport::default());
        assert!(!destination.exists(), "nothing is written");
        assert_eq!(
            SystemTreeCopier::new().probe_native(source.path(), &destination),
            NativeCapability::Unknown
        );
        assert_eq!(SystemTreeCopier::new().mechanism(), NativeMechanism::None);
    }
}
