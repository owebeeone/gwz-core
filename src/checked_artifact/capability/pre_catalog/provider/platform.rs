use cap_std::fs::{Dir, File};

use super::super::*;
#[cfg(test)]
use crate::checked_artifact::capability::PlatformCapability;
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

/// DR-1 ship (1) W3's test-only seam (`GwzM5-8DR1-WarnOrRefuse-Charter.md`
/// §3.8, 2026-09-03).
///
/// The merge-level rows of §5 W3 reach `HostPlatform` through `entry.rs`, not
/// through `filesystem.rs`'s injected providers, so a below-bar volume cannot
/// be presented to them any other way: the CI hosts are ext4 and APFS, both
/// ABOVE the bar. Armed, `dir_identity` answers the same
/// `Unsupported(PersistentFilesystemIdentity, …)` a btrfs or tmpfs volume
/// answers, and `describe_volume` answers the injected description, so the
/// decision point (§2) sees exactly the shape it sees on such a volume.
///
/// **Scoped, not armed-once.** The probe calls `dir_identity` more than once
/// (`catalog_lease/target.rs::finish` asks the workspace target AND its related
/// Git directory), so a `fail_next`-style single-shot arm would answer the
/// first and admit the second. It stays armed for the whole closure and
/// disarms on the way out, panic included.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct InjectedVolumeDescription {
    pub(crate) name: Option<String>,
    pub(crate) remote: bool,
    pub(crate) volatile: bool,
}

#[cfg(test)]
thread_local! {
    static IDENTITY_UNAVAILABLE: std::cell::RefCell<Option<InjectedVolumeDescription>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `body` with the host's durable-identity probe answering `Unsupported`
/// and its volume description answering `injected`.
#[cfg(test)]
pub(crate) fn with_identity_unavailable<T>(
    injected: InjectedVolumeDescription,
    body: impl FnOnce() -> T,
) -> T {
    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            IDENTITY_UNAVAILABLE.with_borrow_mut(|slot| *slot = None);
        }
    }
    IDENTITY_UNAVAILABLE.with_borrow_mut(|slot| *slot = Some(injected));
    let _disarm = Disarm;
    body()
}

// M5d step (3)'s SECOND half of the same seam
// (`GwzM5-8M5d-Charter.md` §3, 2026-09-03).
//
// The seam above presents a volume below the CATALOG's identity bar. It says
// nothing about the LEGACY per-leaf handle probe (`identity.rs`'s
// `name_to_handle_at` / `ATTR_CMN_OBJPERMANENTID` / NTFS file-id), and that
// probe is the one the record create and every reverse checked door take. It
// cannot be made to fail naturally on either CI host — APFS and ext4 both
// answer it — and there is no other injection point, because those doors
// reach the host through `identity.rs`, not through `HostPlatform`.
//
// Armed, `identity::object_identity` answers the same
// `ErrorKind::Unsupported` carrying `PERSISTENT_FILESYSTEM_IDENTITY_REMEDY`
// that a real overlay without `nfs_export` answers (measured on a real mount
// at ship (1) W5: `name_to_handle_at` fails `EOPNOTSUPP`, which
// `persistent_identity_error` downgrades to exactly that). It is INDEPENDENT
// of the arm above: a volume can be below the identity bar with handles
// intact (NFS, tmpfs) or below both (overlay), and the charter's whole point
// is that those two answers differ.
//
// Scoped for the whole closure and disarmed on the way out, panic included,
// for the same reason its sibling is: a single door takes the probe more than
// once (root, then parent).
#[cfg(test)]
thread_local! {
    static HANDLE_PROBE_UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `body` with the host's LEGACY persistent-handle probe refusing.
#[cfg(test)]
pub(crate) fn with_handle_probe_unavailable<T>(body: impl FnOnce() -> T) -> T {
    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            HANDLE_PROBE_UNAVAILABLE.with(|slot| slot.set(false));
        }
    }
    HANDLE_PROBE_UNAVAILABLE.with(|slot| slot.set(true));
    let _disarm = Disarm;
    body()
}

/// Whether the seam above is armed, read by `identity.rs`'s legacy probe.
#[cfg(test)]
pub(in crate::checked_artifact) fn handle_probe_is_unavailable() -> bool {
    HANDLE_PROBE_UNAVAILABLE.with(std::cell::Cell::get)
}

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
        #[cfg(test)]
        if let Some(injected) = IDENTITY_UNAVAILABLE.with_borrow(Clone::clone) {
            return Ok(VolumeDescription {
                name: injected.name,
                remote: injected.remote,
                volatile: injected.volatile,
            });
        }
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
        #[cfg(test)]
        if IDENTITY_UNAVAILABLE.with_borrow(Option::is_some) {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PersistentFilesystemIdentity,
                "injected: identity unavailable",
            ));
        }
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
