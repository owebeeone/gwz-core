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
    /// DR-1 W2, 2026-09-03 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.2): the
    /// NAME says `Ext4` and the admission it stands for no longer does. The
    /// Linux provider's gate is identity-based since this step — a nonzero
    /// `FS_IOC_GETFSUUID` UUID plus a persistent `name_to_handle_at` handle —
    /// so this variant is what xfs and f2fs are admitted AS, not just ext4.
    /// The name stays because this enum is a PERSISTED catalog value:
    /// renaming it is a catalog-format change, which the charter parks
    /// alongside the nonce and the dual-tuple migration (charter §0 "What
    /// this is not"). Rename it only with that migration.
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
/// substrate, the admitted filesystems, and TWO escapes, because one of them
/// is wrong for a merge already open.
///
/// **Scope, corrected by the E4.1 review's [P1-1]/[P2-1].** The earlier claim
/// here — "the blast radius is exactly the checked `--no-ff` v1 path" — was
/// true of STARTS and false of records already on disk:
/// `workspace_ops/merge/model/version.rs`'s `ACTIVE_WRITER_FLOOR` governs which
/// version a start writes, not which lifecycle an existing record routes to. A
/// `--no-ff` start and the resume of a v1 record refuse; an ordinary merge falls
/// back to the v0 lifecycle instead of refusing.
///
/// **The `--abort` clause, SCOPED BY PATH** (2026-09-02,
/// `GwzM5-8R2E-CapabilityFreeAmendment.md` §6): an abort that touches no checked
/// artifact needs no such filesystem; aborts that must re-verify checked
/// artifacts — preservation bundles, a selected root's manifest and lock, or the
/// merge's published evidence, re-verified through the checked boundary — need
/// persistent file handles and a mount identity. A dated residual, shipped with
/// A1's v1 reverse path, cured only by DR-1's (C). Those doors take the LEGACY
/// probe (`identity.rs:312-367`), which admits btrfs/xfs/zfs where the catalog's
/// identity gate refuses, so the string's "ext4 only" was the CATALOG's
/// admission list, not the abort's.
///
/// **DR-1 W2, 2026-09-03: the string's "ext4 only" is now STALE, and it stays
/// for exactly one more step.** `GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.2
/// removed the Linux provider's `require_ext4`, so the catalog now admits xfs
/// and f2fs as well and the two probes' admission sets differ only on btrfs
/// (the UUID ioctl still refuses it with `ENOTTY`) and on tmpfs/ramfs (the
/// provider refuses those as volatile). Rewriting the sentence belongs to W3,
/// with the strict refusal it is the remedy for (§3.6) and the contracts pin
/// that carries it; W2 deliberately does not move it out from under that step.
///
/// The capability-free half is pinned by E4.1(c)'s
/// `a_v1_resume_refuses_without_mutation_and_abort_still_clears_the_record`
/// (`src/workspace_ops/tests/g23/a1_activation.rs`) — [P3-C1]'s carrier, discharged
/// by name; and [P3-8] closes (nothing converts, no snapshot exclusion grows).
pub(super) const PERSISTENT_FILESYSTEM_IDENTITY_REMEDY: &str = "this filesystem does not expose the persistent file handles and mount identity that checked \
     merge artifacts require; run the workspace on a filesystem that does (on Linux the checked \
     catalog admits ext4 only; APFS or HFS+ on macOS; NTFS on Windows). An open merge can be \
     cleared with `gwz merge --abort`, which needs no such filesystem unless it must re-verify \
     checked artifacts — a preservation bundle, a selected root's manifest and lock, or the \
     merge's published evidence; a new merge can be started without --no-ff";

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
