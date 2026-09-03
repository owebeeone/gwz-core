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

pub(in crate::checked_artifact) struct HostPlatform;

/// The volume a checked directory lives on, as a WORDING AID
/// (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.3, 2026-09-03).
///
/// The warning a below-bar volume prints must NAME the filesystem and pick one
/// of three parentheticals, and no probe on the identity path produces either.
/// This struct is that answer and nothing more: it is never a decision input,
/// with one exception the charter states explicitly — `volatile`, and even
/// there the decision is taken by the provider's own admission gate on its own
/// `fstatfs` (`platform/linux.rs::identity`, §3.2), never by a caller reading
/// this value. `remote` in particular is a REASON for the warning's wording
/// and never a denylist (§0.1): a network volume that can still prove durable
/// identity is admitted. A `None` name is rendered `unknown`, not refused.
#[derive(Clone, Debug)]
pub(in crate::checked_artifact) struct VolumeDescription {
    /// The filesystem's own name — `ext4`, `btrfs`, `fuse.sshfs` on Linux,
    /// `apfs` on macOS, `NTFS` on Windows — or `None` where the platform
    /// cannot name it.
    pub(in crate::checked_artifact) name: Option<String>,
    /// The volume's bytes are reached over a network.
    pub(in crate::checked_artifact) remote: bool,
    /// The volume's contents do not survive power loss.
    pub(in crate::checked_artifact) volatile: bool,
}

impl HostPlatform {
    /// The host's volume description. An inherent method rather than a trait
    /// row because it answers no capability: it is the wording input W3's
    /// decision point reads after `dir_identity` has already decided, and
    /// W3's test-only seam (charter §3.8) wraps exactly this call.
    pub(in crate::checked_artifact) fn describe_volume(
        &self,
        directory: &Dir,
    ) -> Result<VolumeDescription, CheckedFsError> {
        imp::describe_volume(directory)
    }
}

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
