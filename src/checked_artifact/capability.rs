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
/// M5d step (3)'s handle-probe half of the same seam
/// (`GwzM5-8M5d-Charter.md` §3, 2026-09-03): the legacy per-leaf probe the
/// record create and every reverse checked door take, which no CI host can be
/// made to refuse.
#[cfg(test)]
pub(super) use pre_catalog::handle_probe_is_unavailable;
#[cfg(test)]
pub(crate) use pre_catalog::with_handle_probe_unavailable;
pub(super) use pre_catalog::*;
/// DR-1 ship (1) W3's test-only seam (`GwzM5-8DR1-WarnOrRefuse-Charter.md`
/// §3.8, 2026-09-03). Named rather than glob-carried: the glob above
/// narrows every item to `pub(in crate::checked_artifact)`, and the merge
/// rows that arm this seam live in `crate::workspace_ops::tests`.
#[cfg(test)]
pub(crate) use pre_catalog::{InjectedVolumeDescription, with_identity_unavailable};

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
/// **Scope, corrected by the E4.1 review's [P1-1]/[P2-1], then NARROWED by
/// DR-1 W3.** The earlier claim here — "the blast radius is exactly the checked
/// `--no-ff` v1 path" — was true of STARTS and false of records already on disk:
/// `workspace_ops/merge/model/version.rs`'s `ACTIVE_WRITER_FLOOR` governs which
/// version a start writes, not which lifecycle an existing record routes to. As
/// of DR-1 ship (1) W3 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1/§3.6,
/// 2026-09-03) a below-bar `--no-ff` start and a below-bar continue no longer
/// refuse AT ALL by default: they warn once and run catalog-free
/// (`entry.rs::crash_recovery_decision`). This sentence is what
/// `--filesystem-strict` shows, plus what activation's OTHER refusals — an `Io`
/// after the probe, an `Ambiguous` — keep rendering through
/// `render_catalog_refusal`. Those are errors, not the bar.
///
/// **The `--abort` clause, SCOPED BY PATH** (2026-09-02,
/// `GwzM5-8R2E-CapabilityFreeAmendment.md` §6): an abort that touches no checked
/// artifact needs no such filesystem; aborts that must re-verify checked
/// artifacts — preservation bundles, a selected root's manifest and lock, or the
/// merge's published evidence, re-verified through the checked boundary — need
/// persistent file handles and a mount identity. A dated residual, shipped with
/// A1's v1 reverse path, cured only by DR-1's (C). Those doors take the LEGACY
/// probe (`identity.rs:312-367`), which admits btrfs/xfs/zfs where the catalog's
/// identity gate refuses — so the filesystems the sentence below names are the
/// CATALOG's admission contract, never the abort's, and an abort on a volume
/// this sentence refuses still clears the record.
///
/// **DR-1 W3, 2026-09-03: the "ext4 only" clause W2 dated STALE is GONE, and
/// the sentence is identity-based** (charter §3.6). The bar the string now
/// describes is the one `platform/linux.rs::identity` actually applies since W2
/// §3.2 removed `require_ext4`: a filesystem that answers `FS_IOC_GETFSUUID`
/// with a nonzero UUID and `name_to_handle_at` with a persistent handle. ext4,
/// xfs and f2fs clear it; btrfs (`ENOTTY`), kernels before 6.9, tmpfs/ramfs
/// (refused as volatile) and network mounts do not. The named filesystems are
/// EXAMPLES of that contract, not a name list — the gate tests the capability.
/// The escapes are the two the charter names: run without `--filesystem-strict`
/// to proceed without crash recovery, and `gwz merge --abort` for a merge
/// already open. `--no-ff` is deliberately no longer named: dropping it is no
/// longer an escape, because an ordinary start does not reach this door at all
/// until M5c and a `--no-ff` start below the bar now warns rather than refuses.
///
/// The capability-free half is pinned by E4.1(c)'s
/// `a_v1_resume_refuses_without_mutation_and_abort_still_clears_the_record`
/// (`src/workspace_ops/tests/g23/a1_activation.rs`) — [P3-C1]'s carrier, discharged
/// by name; and [P3-8] closes (nothing converts, no snapshot exclusion grows).
pub(super) const PERSISTENT_FILESYSTEM_IDENTITY_REMEDY: &str = "this filesystem does not expose the persistent file handles and durable filesystem identity \
     that crash recovery for checked merge artifacts requires; run the workspace on one that does \
     (on Linux any filesystem answering FS_IOC_GETFSUUID — ext4, xfs and f2fs do, btrfs, tmpfs and \
     network mounts do not; a local APFS or HFS+ volume on macOS; NTFS on Windows). Run without \
     --filesystem-strict to proceed without crash recovery, or clear a merge already open with \
     `gwz merge --abort`, which needs no such filesystem unless it must re-verify checked \
     artifacts — a preservation bundle, a selected root's manifest and lock, or the merge's \
     published evidence";

/// M5d step (3) — the REVERSE door's refusal on a handle-fail volume
/// (`GwzM5-8M5d-Charter.md` §3(b), 2026-09-03, operator-chartered product).
///
/// **Why it is not [`PERSISTENT_FILESYSTEM_IDENTITY_REMEDY`].** That sentence
/// offers two escapes, and one of them — "clear a merge already open with
/// `gwz merge --abort`" — is CIRCULAR at this door: a selected-root or
/// `--preserve` abort IS the thing refusing, because it must re-verify a
/// checked artifact and this volume exposes no persistent handles to bind one
/// with. The charter's answer is a new sentence naming exactly ONE escape,
/// and one that needs neither handles on this volume nor an old binary: move
/// the workspace to a volume that proves handles and abort there.
///
/// **What it deliberately does not say.** Not `git merge --abort` — it rolls
/// back none of the three things this door protects (a selected root's
/// manifest and lock, a preservation bundle, the merge's published evidence).
/// Not "delete the record" — that leaves exactly those three unrolled-back.
/// Not `gwz 0.13.0` — the charter is explicit (§3(b), last bullet) that an
/// old binary is not an accepted escape for an open v1 record.
///
/// Participant-only abort is unaffected and still clears the record: it
/// touches no checked artifact and so never reaches this door (charter
/// §3(a); `GwzM5-8R2E-CapabilityFreeAmendment.md` §6, capability-free by
/// path).
pub(super) const HANDLE_FAIL_REVERSE_DOOR_ESCAPE: &str = "this filesystem does not expose the persistent file handles that reversing a merge through the \
     checked boundary requires. One escape works from here: copy the whole workspace onto a volume \
     that proves them (a local APFS or HFS+ volume on macOS; ext4, xfs or f2fs on Linux; NTFS on \
     Windows) and run `gwz merge --abort` there, adding `--preserve` if that was the door that \
     refused";

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
