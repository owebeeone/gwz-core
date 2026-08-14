use cap_std::fs::{Dir, File};

use super::super::*;
use crate::checked_artifact::capability::{
    DurableIdentityProvider, ObjectIdentityFact, PathComponentMode, PathEquivalenceProvider,
};

#[cfg(target_os = "linux")]
#[path = "platform/linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "platform/macos.rs"]
mod imp;
#[cfg(windows)]
#[path = "platform/windows.rs"]
mod imp;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
#[path = "platform/unsupported.rs"]
mod imp;

pub(super) struct HostPlatform;

impl PathEquivalenceProvider<Dir> for HostPlatform {
    fn parent_mode(&self, parent: &Dir) -> Result<PathComponentMode, CheckedFsError> {
        imp::parent_mode(parent)
    }
}

impl DurableIdentityProvider<Dir, File> for HostPlatform {
    type InvocationIdentity = Vec<u8>;
    type RenameDomain = Vec<u8>;

    fn support_profile(&self) -> SupportedFilesystemProfile {
        imp::support_profile()
    }

    fn dir_identity(
        &self,
        directory: &Dir,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
        imp::dir_identity(directory)
    }

    fn file_identity(
        &self,
        file: &File,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
        imp::file_identity(file)
    }

    fn rename_domain(&self, directory: &Dir) -> Result<Vec<u8>, CheckedFsError> {
        imp::rename_domain(directory)
    }
}
