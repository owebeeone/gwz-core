use cap_std::fs::{Dir, File};

use super::super::super::*;
use crate::checked_artifact::capability::{
    ObjectIdentityFact, PathComponentMode, PlatformCapability,
};

pub(super) const fn support_profile() -> SupportedFilesystemProfile {
    SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1
}

pub(super) fn dir_identity(
    _directory: &Dir,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    Err(unsupported())
}

pub(super) fn file_identity(
    _file: &File,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    Err(unsupported())
}

pub(super) fn parent_mode(_parent: &Dir) -> Result<PathComponentMode, CheckedFsError> {
    Err(unsupported())
}

pub(super) fn rename_domain(_directory: &Dir) -> Result<Vec<u8>, CheckedFsError> {
    Err(unsupported())
}

fn unsupported() -> CheckedFsError {
    CheckedFsError::unsupported(
        PlatformCapability::DurableObjectIdentity,
        "checked filesystem provider is unsupported on this platform",
    )
}
