//! `gwz-family-store`: the sole writer of family metadata (lane S).
//!
//! [`YamlFamilyStore`] implements `gwz_family_store_contract::FamilyStore`
//! and its session over the frozen format-1 files (`gwz_family_model`
//! constants): the root index, clone pointers and allocation markers, using
//! same-directory temporary write and rename with checked flushes, the
//! 1 MiB encoded-index limit, and a small OS advisory try-lock on
//! `.gwz/local-family.lock` (`flock` on supported Unix hosts, `LockFileEx`
//! on Windows) released with its handle. It never reaches into gwz-core's
//! private checked-artifact locks or the single-caller pinned verified
//! writer; unsupported locking refuses family mutation.
//!
//! LCM1.0c checkpoint state: the store and session types exist with their
//! frozen signatures and refuse typed (`StoreError::Unimplemented`) without
//! reading or writing. Lane S's implementation starts from the contract's
//! conformance suite (`gwz_family_store_contract::contract_tests::run_all`)
//! failing through a real temporary-directory fixture.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use gwz_family_model::{AllocationId, FamilyChange, FamilyId, FamilyView, MemberName};
use gwz_family_store_contract::{
    AppliedChange, FamilyLocation, FamilyObservation, FamilySession, FamilyStore, StoreError,
    StoreOperation,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YamlFamilyStore;

impl YamlFamilyStore {
    pub fn new() -> Self {
        Self
    }
}

impl FamilyStore for YamlFamilyStore {
    type Session = LockedFamilySession;

    fn read_view(&self, _location: &FamilyLocation) -> Result<FamilyObservation, StoreError> {
        Err(StoreError::Unimplemented {
            operation: StoreOperation::ReadIndex,
        })
    }

    fn try_lock(&self, _location: &FamilyLocation) -> Result<Self::Session, StoreError> {
        Err(StoreError::Unimplemented {
            operation: StoreOperation::Lock,
        })
    }
}

/// A held root family lock. Constructed only by [`YamlFamilyStore::try_lock`].
#[derive(Debug)]
pub struct LockedFamilySession {
    root: PathBuf,
}

impl FamilySession for LockedFamilySession {
    fn root(&self) -> &Path {
        &self.root
    }

    fn reread(&mut self) -> Result<Option<FamilyView>, StoreError> {
        Err(StoreError::Unimplemented {
            operation: StoreOperation::ReadIndex,
        })
    }

    fn found(
        &mut self,
        _family_id: FamilyId,
        _root_allocation: AllocationId,
    ) -> Result<AppliedChange, StoreError> {
        Err(StoreError::Unimplemented {
            operation: StoreOperation::WriteIndex,
        })
    }

    fn apply(&mut self, _change: &FamilyChange) -> Result<AppliedChange, StoreError> {
        Err(StoreError::Unimplemented {
            operation: StoreOperation::WriteIndex,
        })
    }

    fn install_pointer(
        &mut self,
        _name: &MemberName,
        _destination: &Path,
    ) -> Result<AppliedChange, StoreError> {
        Err(StoreError::Unimplemented {
            operation: StoreOperation::WritePointer,
        })
    }

    fn remove_pointer(&mut self, _name: &MemberName) -> Result<AppliedChange, StoreError> {
        Err(StoreError::Unimplemented {
            operation: StoreOperation::RemovePointer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_store_refuses_typed_and_creates_nothing() {
        let temp = std::env::temp_dir().join(format!(
            "gwz-family-store-checkpoint-{}",
            std::process::id()
        ));
        let location = FamilyLocation::new(&temp);
        let store = YamlFamilyStore::new();
        assert!(matches!(
            store.read_view(&location),
            Err(StoreError::Unimplemented {
                operation: StoreOperation::ReadIndex
            })
        ));
        assert!(matches!(
            store.try_lock(&location),
            Err(StoreError::Unimplemented {
                operation: StoreOperation::Lock
            })
        ));
        assert!(
            !temp.exists(),
            "the checkpoint store creates no directory, index or lock file"
        );
    }
}
