//! Pure filesystem-capability contracts for checked artifacts.
//!
//! This module deliberately contains no host implementation. Its types freeze
//! the values that platform providers and the pre-catalog collision scan must
//! prove before checked-artifact code may create private state.

use std::io;

mod collision;
mod durable_identity;
mod path;
mod pre_catalog;

#[allow(
    unused_imports,
    reason = "R2 retains collision fact vocabulary for the sealed platform provider"
)]
pub(super) use collision::*;
pub(super) use durable_identity::DurableObjectIdentityV1;
pub(super) use path::*;
pub(super) use pre_catalog::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SupportedFilesystemProfile {
    LinuxExt4FsIocGetFsUuidV1,
    MacPersistentObjectIdV1,
    WindowsNtfsFileId128V1,
}

impl SupportedFilesystemProfile {
    pub(super) const ALL: &'static [Self] = &[
        Self::LinuxExt4FsIocGetFsUuidV1,
        Self::MacPersistentObjectIdV1,
        Self::WindowsNtfsFileId128V1,
    ];
}

/// The one refusal a user can act on, spelled once.
///
/// R2-E E4.1 precondition 1 (E0.2 §5.3): the durable-identity gap must reach
/// the user as an actionable sentence rather than an `errno`. It names the
/// substrate, the admitted filesystems, and the escape — the escape being real,
/// because the blast radius of `PersistentFilesystemIdentity` is exactly the
/// checked `--no-ff` v1 path (`model/version.rs`'s `ACTIVE_WRITER_FLOOR` keeps
/// ordinary and `--ff-only` starts on v0, which never take a catalog lease).
pub(super) const PERSISTENT_FILESYSTEM_IDENTITY_REMEDY: &str = "this filesystem does not expose the persistent file handles and mount identity that checked \
     merge artifacts require; run the workspace on a filesystem that does (local ext4 on Linux, \
     APFS or HFS+ on macOS, NTFS on Windows), or start the merge without --no-ff";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum PlatformCapability {
    AsciiProtocolPath,
    PathEquivalence,
    /// The identity VALUE contract: an identity the substrate already returned
    /// is well formed for its profile (`durable_identity.rs`'s three
    /// constructors). Distinct from the substrate capability below, which is
    /// about whether the filesystem can answer at all.
    DurableObjectIdentity,
    /// R2-E E4.1: the SUBSTRATE the checked catalog needs — persistent file
    /// handles (Linux `name_to_handle_at`, macOS `ATTR_CMN_OBJPERMANENTID`,
    /// NTFS 128-bit file ids) and a mount identity. Its absence is the one
    /// platform gap a user meets on a supported OS, so it is the only value
    /// carrying a [`PERSISTENT_FILESYSTEM_IDENTITY_REMEDY`].
    PersistentFilesystemIdentity,
    AtomicRenameDomain,
    NamespaceDurability,
    PrivateNamespaceCollisionScan,
    RuntimeAdvisoryLock,
    ManagedParentBootstrap,
}

impl PlatformCapability {
    /// The actionable sentence a refusal of this capability shows the user, if
    /// the gap is one a user can do something about.
    pub(super) const fn remedy(self) -> Option<&'static str> {
        match self {
            Self::PersistentFilesystemIdentity => Some(PERSISTENT_FILESYSTEM_IDENTITY_REMEDY),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(super) enum CheckedFsError {
    Unsupported {
        capability: PlatformCapability,
        detail: String,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Ambiguous {
        fact: &'static str,
        detail: String,
    },
}

impl CheckedFsError {
    pub(super) fn unsupported(capability: PlatformCapability, detail: impl Into<String>) -> Self {
        Self::Unsupported {
            capability,
            detail: detail.into(),
        }
    }

    pub(super) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub(super) fn ambiguous(fact: &'static str, detail: impl Into<String>) -> Self {
        Self::Ambiguous {
            fact,
            detail: detail.into(),
        }
    }
}

pub(super) trait PathEquivalenceProvider<DirectoryHandle: ?Sized> {
    fn parent_mode(&self, parent: &DirectoryHandle) -> Result<PathComponentMode, CheckedFsError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ObjectIdentityFact<DurableIdentity, InvocationIdentity> {
    durable: DurableIdentity,
    invocation: InvocationIdentity,
}

impl<DurableIdentity, InvocationIdentity> ObjectIdentityFact<DurableIdentity, InvocationIdentity> {
    pub(super) fn new(durable: DurableIdentity, invocation: InvocationIdentity) -> Self {
        Self {
            durable,
            invocation,
        }
    }

    pub(super) fn durable(&self) -> &DurableIdentity {
        &self.durable
    }

    pub(super) fn invocation(&self) -> &InvocationIdentity {
        &self.invocation
    }
}

pub(super) trait DurableIdentityProvider<DirectoryHandle: ?Sized, FileHandle: ?Sized> {
    type InvocationIdentity: Clone + Eq;
    type RenameDomain: Clone + Eq;

    fn support_profile(&self) -> SupportedFilesystemProfile;

    fn dir_identity(
        &self,
        directory: &DirectoryHandle,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Self::InvocationIdentity>, CheckedFsError>;

    fn file_identity(
        &self,
        file: &FileHandle,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Self::InvocationIdentity>, CheckedFsError>;

    fn rename_domain(
        &self,
        directory: &DirectoryHandle,
    ) -> Result<Self::RenameDomain, CheckedFsError>;
}
