use cap_std::fs::{Dir, File};

use super::super::super::*;
use super::VolumeDescription;
use crate::checked_artifact::capability::{
    ObjectIdentityFact, PathComponentMode, PlatformCapability,
};

/// R2-E E4.1 precondition 5 — the swept Linux-profile claim (E0.1(b) row 3,
/// routed here from O12/E6.2 by E0.2 §5.3 item 5).
///
/// This stub used to CLAIM `LinuxExt4FsIocGetFsUuidV1` on a platform that is
/// neither Linux, macOS nor Windows. The trait's `support_profile` is
/// infallible, so some variant must be named; what the sweep removes is the
/// claim's standing, and it removes it structurally rather than by convention:
/// `CatalogLeaseTargetWitnessV1::facts` reads `support_profile()` only after
/// `dir_identity(...)?` has already answered, and every probe in this file
/// refuses. No caller on this platform can observe the value below — it is
/// unreachable, not merely shielded by fail-closed ordering, and it is named
/// for that rather than for a filesystem this platform does not have.
const UNREACHABLE_PROFILE: SupportedFilesystemProfile =
    SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1;

pub(super) const fn support_profile() -> SupportedFilesystemProfile {
    UNREACHABLE_PROFILE
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

/// DR-1 W2 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.3, 2026-09-03): the one
/// probe in this file that ANSWERS rather than refusing, and deliberately so.
/// It is a wording aid, not a capability: every identity probe above still
/// refuses, so this platform is always below the bar, and what this returns
/// only decides whether the warning names a filesystem (`None` → `unknown`)
/// and which parenthetical it takes (`no durable filesystem identity`).
/// Refusing here would give the warning nothing to say and would make a
/// wording input look like a gate.
pub(super) fn describe_volume(_directory: &Dir) -> Result<VolumeDescription, CheckedFsError> {
    Ok(VolumeDescription {
        name: None,
        remote: false,
        volatile: false,
    })
}

fn unsupported() -> CheckedFsError {
    CheckedFsError::unsupported(
        PlatformCapability::PersistentFilesystemIdentity,
        "this operating system has no checked filesystem provider at all",
    )
}
