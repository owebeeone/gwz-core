use std::ffi::{OsStr, OsString};

use cap_std::fs::Dir;

use crate::model::{ErrorCode, ModelError, ModelResult};

/// The closed durability-anchor protocol (R2-D Phase 4 Step 4.2, freeze §4.3 row
/// E22). Split out of this file at that step: `platform.rs` keeps the P1 pair,
/// the two sealed publication compositions, and the P2/P5 arms, and the anchor —
/// the whole of P5's `AnchoredPrivateArea` machinery — now owns its own module.
#[cfg(any(windows, test))]
mod anchor;

pub(super) struct OpenedRenameSource<'a> {
    file: cap_std::fs::File,
    source_dir: &'a Dir,
    source: OsString,
}

impl OpenedRenameSource<'_> {
    pub(super) const fn file(&self) -> &cap_std::fs::File {
        &self.file
    }

    pub(super) const fn file_mut(&mut self) -> &mut cap_std::fs::File {
        &mut self.file
    }
}

#[cfg(not(windows))]
pub(super) fn open_rename_source<'a>(
    source_dir: &'a Dir,
    source: &OsStr,
    code: ErrorCode,
    label: &str,
) -> ModelResult<OpenedRenameSource<'a>> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
    use cap_std::fs::OpenOptions;

    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(FollowSymlinks::No)
        .maybe_dir(true);
    let file = source_dir
        .open_with(source, &options)
        .map_err(|cause| io_error(code, label, cause))?;
    Ok(OpenedRenameSource {
        file,
        source_dir,
        source: source.to_os_string(),
    })
}

#[cfg(windows)]
pub(super) fn open_rename_source<'a>(
    source_dir: &'a Dir,
    source: &OsStr,
    code: ErrorCode,
    label: &str,
) -> ModelResult<OpenedRenameSource<'a>> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use windows_sys::Win32::Foundation::GENERIC_READ;
    use windows_sys::Win32::Storage::FileSystem::*;

    let mut options = OpenOptions::new();
    options
        .access_mode(GENERIC_READ | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH | FILE_FLAG_BACKUP_SEMANTICS,
        )
        .follow(FollowSymlinks::No);
    let file = source_dir
        .open_with(source, &options)
        .map_err(|cause| io_error(code, label, cause))?;
    Ok(OpenedRenameSource {
        file,
        source_dir,
        source: source.to_os_string(),
    })
}

#[cfg(not(windows))]
pub(super) fn rename_open_source(
    source: &OpenedRenameSource<'_>,
    destination_dir: &Dir,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    rename_relative(
        source.source_dir,
        &source.source,
        destination_dir,
        destination,
        replace,
        code,
        label,
    )
}

#[cfg(windows)]
pub(super) fn rename_open_source(
    source: &OpenedRenameSource<'_>,
    destination_dir: &Dir,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;

    use super::fault::{CheckedArtifactFault, fault};

    let destination_path = windows_destination_path(destination_dir, destination)
        .map_err(|cause| io_error(code, label, cause))?;
    // Destination-window hook (R2-F, amendment §4.1 erratum): the residual
    // window opens once the absolute destination path is derived and closes
    // at the handle rename below. Observation-only: outside cfg(test) this
    // compiles to Ok(()), and no production behavior changes.
    fault(
        CheckedArtifactFault::AfterDestinationPathDerivation,
        code,
        label,
    )?;
    let name = destination_path.encode_wide().collect::<Vec<_>>();
    // Windows requires at least the fixed structure size plus the variable
    // name bytes, even though the fixed structure already contains its
    // one-element FileName placeholder.
    let size = std::mem::size_of::<FILE_RENAME_INFO>() + name.len() * 2;
    let mut storage = vec![0_usize; size.div_ceil(std::mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = replace;
        // SetFileInformationByHandle rejects a non-null RootDirectory on
        // supported Windows runners, so the destination is an absolute path
        // derived from the retained directory handle immediately before the
        // rename. The handle does NOT prevent a same-user process from
        // renaming the destination directory or a path ancestor inside this
        // window (directory opens share FILE_SHARE_DELETE); that residual is
        // assigned to the amendment's cooperating-same-user boundary, and
        // the mandatory post-publish verification through the retained
        // destination handle detects a redirect read-only (§4.1 erratum
        // 2026-08-15; native window test executes at R2-F).
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = u32::try_from(name.len() * 2)
            .map_err(|_| error(code, label, "destination name is too long"))?;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
        if SetFileInformationByHandle(
            source.file.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            u32::try_from(size).map_err(|_| error(code, label, "rename buffer is too large"))?,
        ) == 0
        {
            return Err(io_error(code, label, std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

/// The exact object a legacy leaf edge proved, restated for the sealed
/// publication below so the primitive can re-verify it through the very handle
/// it renames rather than trusting the caller's earlier open-by-name.
pub(super) struct LeafPublicationSourceV1<'a> {
    pub(super) identity: &'a super::identity::ObjectIdentity,
    pub(super) bytes: &'a [u8],
}

/// Sealed source-associated publication for the legacy leaf family — the P1
/// composition of `GwzM5-8R2DInterfaceFreeze.md` §4.1 ("`open_rename_source`
/// … then `rename_open_source` — retains the identity-checked handle across a
/// relative no-replace rename") applied to §4.3 rows E18-E21, which the frozen
/// table assigns to P1 "(replaces `platform::rename_relative`)".
///
/// It is the legacy twin of
/// `capability/pre_catalog/provider/publication.rs::publish_verified_no_replace`
/// and not a call into it, for one binding reason: that function's identity
/// compare is `HostPlatform`-bound, and `HostPlatform` admits only the closed
/// support table (`require_ext4` on Linux, `ATTR_CMN_OBJPERMANENTID` on macOS,
/// NTFS `FileId128` on Windows). The legacy leaf writer is live on every
/// filesystem that carries a persistent file handle, so routing these four
/// edges through that function would narrow production merge and stash flows to
/// that table — the one thing plan §4 Step 4.1 forbids ("with identical
/// external behavior"). This composition therefore takes P1's arms and the
/// legacy family's own durable identity vocabulary (`super::identity`), which is
/// the vocabulary the family's authority record already commits to.
///
/// The physical edge is unchanged. On Windows `rename_relative` already *is*
/// `open_rename_source` + `rename_open_source`; off Windows `rename_open_source`
/// delegates to the same `renameat_with(.., NOREPLACE)`. What the composition
/// adds is the acquisition window: identity and bytes are read back through the
/// retained handle, so a source substituted after the caller's proof is refused
/// before any namespace mutation instead of being moved and then rejected.
pub(super) fn publish_verified_leaf_no_replace(
    source_dir: &Dir,
    source: &OsStr,
    destination_dir: &Dir,
    destination: &OsStr,
    expected: &LeafPublicationSourceV1<'_>,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    use std::io::Read;

    let mut handle = open_rename_source(source_dir, source, code, label)?;
    let observed = super::identity::file_identity(handle.file()).map_err(|cause| {
        ModelError::new(
            ErrorCode::UnsupportedOperation,
            format!("checked {label}: durable filesystem identity is unsupported: {cause}"),
        )
    })?;
    if observed != *expected.identity {
        return Err(error(code, label, "publication source identity changed"));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(expected.bytes.len() + 1)
        .map_err(|_| {
            error(
                code,
                label,
                "publication source verification allocation failed",
            )
        })?;
    handle
        .file_mut()
        .by_ref()
        .take(expected.bytes.len() as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|cause| io_error(code, label, cause))?;
    if bytes != expected.bytes {
        return Err(error(code, label, "publication source bytes changed"));
    }
    rename_open_source(&handle, destination_dir, destination, false, code, label)
}

#[cfg(not(windows))]
pub(super) fn open_dir_share_delete(parent: &Dir, name: &OsStr) -> std::io::Result<Dir> {
    use cap_fs_ext::DirExt;
    parent.open_dir_nofollow(name)
}

#[cfg(windows)]
pub(super) fn open_dir_share_delete(parent: &Dir, name: &OsStr) -> std::io::Result<Dir> {
    // A plain directory open does not request DELETE sharing, so it
    // collides (os error 32) with the retained rename-source handle, which
    // holds DELETE access across the publication edge. Mirror the
    // open_rename_source sharing recipe and no-follow discipline (W4,
    // GwzWindowsMatrix-Classification.md).
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use windows_sys::Win32::Foundation::GENERIC_READ;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = OpenOptions::new();
    options
        .access_mode(GENERIC_READ)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .follow(FollowSymlinks::No);
    let file = parent.open_with(name, &options)?;
    if !file.metadata()?.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "publication source is not a directory",
        ));
    }
    Ok(Dir::from_std_file(file.into_std()))
}

#[cfg(windows)]
fn windows_destination_path(
    destination_dir: &Dir,
    destination: &OsStr,
) -> std::io::Result<OsString> {
    use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };

    const MAX_PATH_UNITS: usize = 32_768;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(512)
        .map_err(|_| std::io::Error::other("allocate Windows destination path"))?;
    buffer.resize(512, 0);
    loop {
        let capacity = u32::try_from(buffer.len()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Windows destination path buffer is too large",
            )
        })?;
        let length = unsafe {
            GetFinalPathNameByHandleW(
                destination_dir.as_raw_handle(),
                buffer.as_mut_ptr(),
                capacity,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        if length == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let length = usize::try_from(length).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Windows destination path length is invalid",
            )
        })?;
        if length < buffer.len() {
            buffer.truncate(length);
            let mut path = std::path::PathBuf::from(OsString::from_wide(&buffer));
            path.push(destination);
            return Ok(path.into_os_string());
        }
        let required = length.checked_add(1).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Windows destination path length overflowed",
            )
        })?;
        if required > MAX_PATH_UNITS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Windows destination path exceeds the platform bound",
            ));
        }
        buffer
            .try_reserve_exact(required - buffer.len())
            .map_err(|_| std::io::Error::other("grow Windows destination path"))?;
        buffer.resize(required, 0);
    }
}

#[cfg(not(windows))]
pub(super) fn prepare_private(
    _dir: &Dir,
    _create: bool,
    _code: ErrorCode,
    _label: &str,
) -> ModelResult<()> {
    Ok(())
}

/// Which writer class a P5 dirent barrier is serving
/// (`GwzM5-8R2DInterfaceFreeze.md` §4.1 row P5, §4.3 rows E10/E14 and the E9
/// activation annotation).
///
/// The distinction exists on Windows alone — on every other platform every
/// class is the same directory `fsync` — and it is a *caller* fact rather
/// than one the directory's own contents may be trusted to reveal: only the
/// checked-artifact private area is allowed to retain the durability anchor the
/// Windows round-trip barrier renames, so only its callers may demand one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DirentBarrierClass<'alias> {
    /// The checked-artifact private area, which deliberately retains a
    /// permanent `.ca1-durability-anchor-<32hex>` file as product
    /// infrastructure (`finish()` never removes it), and whose barrier is
    /// therefore the anchor round trip on Windows.
    AnchoredPrivateArea,
    /// A directory whose children are exact evidence — an admitted action
    /// directory, a catalog interior — and which may therefore retain no
    /// anchor of its own. Its Windows arm is documented at the barrier.
    ExactInterior,
    /// R2-E Phase E2, DECISION B-3
    /// (`GwzM5-8R2E-SemanticsAmendment-DRAFT.md` §3.3): a barrier target parent
    /// that may retain no permanent anchor of its own and is instead **lent**
    /// one for the duration of a single scheduled barrier.
    ///
    /// Neither existing variant states that. `AnchoredPrivateArea` would
    /// *survey* for a resident anchor and, finding none, establish a permanent
    /// one here — the exact-evidence contamination class already diagnosed and
    /// fixed; `ExactInterior` documents the round trip as unavailable, which is
    /// exactly the property the roaming anchor exists to restore. So the
    /// Windows arm of this variant round-trips the **supplied** alias by the
    /// leaf the schedule reserved for it, and surveys for nothing.
    ///
    /// Minting a class variant moves no census — it is not a fault key — and
    /// the §4.3 E10/E14 activation annotation is unaffected: that annotation's
    /// claim is about which arm a *caller* takes, and the tree's **one**
    /// `ExactInterior` construction site (`namespace_mutation.rs`'s `barrier`)
    /// is untouched by this family, whose call site is a distinct one in a
    /// distinct file. *(E2 review [P3-2]: this doc said "both call sites" and
    /// named a second in `namespace/host.rs`. There is no second — `host.rs`'s
    /// `barrier` is an identity pin that delegates and names no class. The
    /// miscount was inherited from E0.2 §3.3 rather than opened and read; the
    /// substance is unchanged and independently verified.)*
    RoamingAnchoredTarget {
        /// The reserved leaf under which the target parent currently holds the
        /// alias. Supplied by the caller from the schedule-derived name the
        /// intent record bound — never surveyed for.
        alias: &'alias OsStr,
        /// The alias's frozen content, re-verified through the very handle each
        /// round-trip rename consumes.
        bytes: &'alias [u8],
    },
}

/// The one outbound-name suffix P5's two round trips share. Owned here rather
/// than by `anchor`, because the roaming arm's residue has to be classifiable on
/// every platform and `mod anchor` is `cfg(any(windows, test))`.
pub(super) const ROUNDTRIP_SUFFIX: &str = ".roundtrip";

pub(super) fn roundtrip_name(resident_name: &OsStr) -> OsString {
    let mut roundtrip = resident_name.to_os_string();
    roundtrip.push(ROUNDTRIP_SUFFIX);
    roundtrip
}

/// What a barrier target parent holds under the two names the roaming arm can
/// produce, after [`prepare_roaming_target`] has converged whatever window a
/// previous drive left.
///
/// Two states, not four: the caller's only decision is create-or-resume, and
/// every window the arm can leave collapses into one of them. Whether a
/// tolerated legacy object was left under the outbound name is deliberately
/// *not* reported — nothing acts on it, and the matrix proves the toleration
/// behaviourally against the settled census rather than against a flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RoamingTargetStateV1 {
    /// Neither name is resident: the caller creates the alias.
    Absent,
    /// An alias is resident at its reserved leaf.
    Resident,
}

/// R2-E Phase E2 — P5's `RoamingAnchoredTarget` recovery entry, the roaming twin
/// of [`prepare_private`].
///
/// `prepare_private`'s job for the resident class is "establish the anchor, or
/// converge whatever window a previous drive left". This is the same job for the
/// lent class, minus the establish half — the caller owns creation, because only
/// the caller knows the schedule-derived leaf. **Its whole reason to exist is
/// that a drive branching on the reserved leaf alone cannot see the outbound
/// name**, whose derivation is this module's, so without this entry a
/// mid-round-trip crash would leave a name no later drive returns.
///
/// The state machine is total over the two names, and every arm is stated
/// because three of the four are crash windows:
///
/// | reserved leaf | outbound | action | result |
/// | --- | --- | --- | --- |
/// | absent | absent | none | `Absent` — the caller creates |
/// | resident | absent | none | `Resident` — the settled between-barriers state |
/// | absent | resident | **return it** | `Resident` — a crash between the round trip's two renames, converged |
/// | resident | resident | none | `Resident` — the outbound object is tolerated |
///
/// The third row is the one this entry exists for: the object is returned to its
/// reserved leaf through the sealed P1 publication — a rename, never a removal,
/// which is the discipline Step 4.2 installed — so nothing persists and the
/// caller resumes on the ordinary resident state.
///
/// The fourth row is **not** refused, and that is deliberate. Two objects under
/// the two names is unreachable on this tree: this entry runs before the caller
/// creates anything, so a drive can never create a second object over a resident
/// outbound name. It *is* reachable on a tree a pre-remediation binary wrote,
/// where the drive branched on the reserved leaf alone and created that second
/// object. Refusing there would be a permanent typed refusal on a reachable
/// state with no in-code exit — the wedge class E16's standard forbids — and the
/// only convergence that could clear it is a removal, which Step 4.2
/// deliberately replaced with durable retirement, and there is exactly one
/// retirement slot per ordinal. So the outbound object is left as a tolerated
/// legacy orphan, recorded rather than hidden: bounded by past crashes on
/// pre-remediation Windows trees, unable to grow because nothing here produces
/// it, and blocking nothing.
///
/// **One state is refused, and it is not a row of the table: foreign bytes.**
/// The third row re-proves the outbound object against the frozen bytes before
/// it returns it, so an object under this protocol's own outbound name that
/// carries something else is refused rather than adopted — and that refusal
/// does block the ordinal until it is cleared. That is the house rule for a
/// name in this family's grammar holding foreign content, pinned for the
/// resident protocol by `foreign_bytes_under_the_anchor_prefix_are_refused_not_adopted`
/// and for the reserved leaf by
/// `foreign_bytes_under_the_reserved_leaf_are_refused_before_the_edge`. Only
/// this protocol writes these two names, so the state means a foreign writer,
/// not a crash.
///
/// Portable, and deliberately not `cfg`-split. Off Windows nothing renames the
/// alias, so the survey is two `symlink_metadata` calls that always answer the
/// first two rows — but keeping it portable is what lets every platform execute
/// the restart rows for a window only Windows can open, exactly as
/// `platform/anchor.rs`'s header argues for the protocol it serves.
pub(super) fn prepare_roaming_target(
    dir: &Dir,
    alias: &OsStr,
    bytes: &[u8],
    code: ErrorCode,
    label: &str,
) -> ModelResult<RoamingTargetStateV1> {
    let outbound = roundtrip_name(alias);
    let alias_resident = leaf_is_resident(dir, alias, code, label)?;
    let outbound_resident = leaf_is_resident(dir, &outbound, code, label)?;
    match (alias_resident, outbound_resident) {
        (false, false) => Ok(RoamingTargetStateV1::Absent),
        (true, _) => Ok(RoamingTargetStateV1::Resident),
        (false, true) => {
            let identity = verify_leaf_bytes(dir, &outbound, bytes, code, label)?;
            publish_verified_leaf_no_replace(
                dir,
                &outbound,
                dir,
                alias,
                &LeafPublicationSourceV1 {
                    identity: &identity,
                    bytes,
                },
                code,
                label,
            )?;
            verify_leaf_bytes(dir, alias, bytes, code, label)?;
            Ok(RoamingTargetStateV1::Resident)
        }
    }
}

/// Whether a leaf is resident at all. `observe_leaf_exact` reports a missing
/// leaf as `Missing` rather than as an error, so this needs no error-kind
/// inspection of its own.
fn leaf_is_resident(dir: &Dir, name: &OsStr, code: ErrorCode, label: &str) -> ModelResult<bool> {
    Ok(
        super::observation::observe_leaf_exact(dir, name, code, label)?.fact
            != super::CheckedArtifactFact::Missing,
    )
}

/// One lent object re-proved by its frozen bytes, returning the durable identity
/// the sealed publication re-verifies through the handle it renames.
fn verify_leaf_bytes(
    dir: &Dir,
    name: &OsStr,
    bytes: &[u8],
    code: ErrorCode,
    label: &str,
) -> ModelResult<super::identity::ObjectIdentity> {
    let observed = super::observation::observe_leaf_exact(dir, name, code, label)?;
    if observed.fact != super::CheckedArtifactFact::Bytes(bytes.to_vec()) {
        return Err(error(code, label, "roaming anchor alias bytes are invalid"));
    }
    observed
        .identity
        .ok_or_else(|| error(code, label, "roaming anchor alias lacks identity"))
}

#[cfg(not(windows))]
pub(super) fn private_barrier(
    dir: &Dir,
    _class: DirentBarrierClass<'_>,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    sync_parent(dir).map_err(|cause| io_error(code, label, cause))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn rename_relative(
    source_dir: &Dir,
    source: &OsStr,
    destination_dir: &Dir,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    let flags = if replace {
        rustix::fs::RenameFlags::empty()
    } else {
        rustix::fs::RenameFlags::NOREPLACE
    };
    rustix::fs::renameat_with(source_dir, source, destination_dir, destination, flags).map_err(
        |cause| {
            io_error(
                code,
                label,
                std::io::Error::from_raw_os_error(cause.raw_os_error()),
            )
        },
    )
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(super) fn rename_relative(
    source_dir: &Dir,
    source: &OsStr,
    destination_dir: &Dir,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    if !replace {
        return Err(error(
            code,
            label,
            "atomic no-replace publication is unsupported on this Unix target",
        ));
    }
    source_dir
        .rename(source, destination_dir, destination)
        .map_err(|cause| io_error(code, label, cause))
}

#[cfg(windows)]
pub(super) fn rename_relative(
    source_dir: &Dir,
    source: &OsStr,
    destination_dir: &Dir,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    let source_handle = open_rename_source(source_dir, source, code, label)?;
    rename_open_source(
        &source_handle,
        destination_dir,
        destination,
        replace,
        code,
        label,
    )
}

#[cfg(all(not(unix), not(windows)))]
pub(super) fn rename_relative(
    source_dir: &Dir,
    source: &OsStr,
    destination_dir: &Dir,
    destination: &OsStr,
    replace: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    if !replace {
        return Err(error(
            code,
            label,
            "atomic no-replace publication is unsupported on this platform",
        ));
    }
    source_dir
        .rename(source, destination_dir, destination)
        .map_err(|cause| io_error(code, label, cause))
}

#[cfg(target_os = "linux")]
pub(super) fn sync_parent(dir: &Dir) -> std::io::Result<()> {
    // cap-std directory capabilities are `O_PATH` descriptors on Linux, and
    // the kernel resolves `fsync` through the descriptor lookup that refuses
    // `O_PATH` files with `EBADF` before any filesystem code runs, so a dup
    // of the capability cannot carry the barrier. Reopening `.` through the
    // capability performs no path re-resolution — the descriptor itself
    // anchors the lookup, so the result names the same directory object —
    // and yields a descriptor `fsync` accepts. Failures stay closed: a dead
    // capability reports the raw OS error from the reopen itself.
    let flushable = rustix::fs::openat(
        dir,
        c".",
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    rustix::fs::fsync(&flushable)?;
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn sync_parent(dir: &Dir) -> std::io::Result<()> {
    dir.try_clone()?.into_std_file().sync_all()
}

#[cfg(windows)]
pub(super) fn sync_parent(_dir: &Dir) -> std::io::Result<()> {
    // Relative replacement uses FILE_FLAG_WRITE_THROUGH. A normal directory
    // handle cannot be flushed portably on Windows.
    Ok(())
}

#[cfg(windows)]
pub(super) fn prepare_private(
    dir: &Dir,
    create: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    anchor::prepare(dir, create, code, label)
}

#[cfg(windows)]
pub(super) fn private_barrier(
    dir: &Dir,
    class: DirentBarrierClass<'_>,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    if let DirentBarrierClass::RoamingAnchoredTarget { alias, bytes } = class {
        // R2-E Phase E2, DECISION B-3. The roaming arm: this directory may
        // retain no permanent anchor, so it is lent one for the duration of one
        // scheduled barrier and the metadata transaction that orders its
        // dirents is that *supplied* object's round trip. Nothing is surveyed
        // for and nothing permanent is established here, which is the whole
        // difference from `AnchoredPrivateArea`.
        return anchor::round_trip_supplied(dir, alias, bytes, code, label);
    }
    if matches!(class, DirentBarrierClass::ExactInterior) {
        // The writer-class-conditional Windows arm of P5 — the twin of E9's
        // `flush_observed_leaf` no-op, and recorded in the same form
        // (`GwzM5-8R2DInterfaceFreeze.md` §4.3, the E9 activation annotation).
        //
        // The round trip below is *unavailable* to this class, not merely
        // skipped: it renames a resident `.ca1-durability-anchor-<32hex>` file,
        // and this class of directory may retain none. Its children are exact
        // evidence — admission refuses an action directory whose
        // `extra_children` is nonzero (`protocol/admission/owner.rs:29-38`) —
        // and the anchor is permanent by design, so planting one per catalog
        // directory would reproduce the exact-evidence contamination class
        // already diagnosed and fixed for the private area
        // (`GwzWindowsMatrix-ExactEvidenceDiagnosis.md` §3 Class B). Preparing
        // an anchor here would trade a durability claim for the very exactness
        // the barrier exists to protect.
        //
        // The property that substitutes is the P2 family's own, stated once for
        // `sync_parent` above and again for `sync_directory_edge`
        // (`capability/pre_catalog/provider/directory_mutation.rs`), and it is
        // writer-class-conditional exactly as E9's is: every row of an exact
        // interior is gwz-written through `durable_write_options`
        // (`FILE_FLAG_WRITE_THROUGH`) and moved by the sealed exact-handle
        // rename, and a normal directory handle cannot be flushed portably on
        // Windows. So this barrier adds no ordering of its own, deliberately
        // and by argument, instead of refusing an ordering the platform cannot
        // give it. Negative space, stated here rather than by reference,
        // because this arm is what changes it: for a FOREIGN-written row the
        // residual is *empty*, not merely weaker — this arm removes the
        // namespace ordering that would otherwise have been the fallback, and
        // a read-only observation handle can supply no byte flush either. An
        // exact interior admits no foreign row by construction (admission
        // refuses a nonzero `extra_children`), and the one consumer class that
        // could otherwise have leaned on that residual is refused outright by
        // `authority_record_binding::require_authority_strength`.
        //
        // R2-D Phase 4 Step 4.2 moved the `AnchoredPrivateArea` machinery this
        // arm declines into `platform/anchor.rs` and rebuilt it as a closed
        // protocol. Nothing about this arm's argument moves with it: the class
        // still selects nothing off Windows, and this class still takes the
        // documented no-op.
        return Ok(());
    }

    anchor::round_trip(dir, code, label)
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::super::fault::{CheckedArtifactFault, run_next_checked_artifact_at};
    use super::{hex, open_dir_share_delete, open_rename_source, rename_open_source};
    use cap_std::{ambient_authority, fs::Dir};
    use std::ffi::{OsStr, OsString};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::model::ErrorCode;

    #[test]
    fn rename_open_source_moves_the_checked_object_after_path_substitution() {
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).unwrap();
        let temporary = std::env::temp_dir().join(format!(
            "gwz-platform-{}-{}",
            std::process::id(),
            hex(&random)
        ));
        std::fs::create_dir(&temporary).unwrap();
        let _cleanup = Cleanup(temporary.clone());
        let source_path = temporary.join("source");
        let displaced_path = temporary.join("displaced");
        let destination_path = temporary.join("destination");
        std::fs::write(&source_path, b"checked\n").unwrap();
        let directory = Dir::open_ambient_dir(&temporary, ambient_authority()).unwrap();
        let source = open_rename_source(
            &directory,
            OsStr::new("source"),
            ErrorCode::IoError,
            "Windows publication test",
        )
        .unwrap();

        std::fs::rename(&source_path, &displaced_path).unwrap();
        std::fs::write(&source_path, b"foreign\n").unwrap();
        rename_open_source(
            &source,
            &directory,
            OsStr::new("destination"),
            false,
            ErrorCode::IoError,
            "Windows publication test",
        )
        .unwrap();

        assert_eq!(std::fs::read(destination_path).unwrap(), b"checked\n");
        assert_eq!(std::fs::read(source_path).unwrap(), b"foreign\n");
        assert!(!displaced_path.exists());
    }

    fn window_fixture(label: &str) -> (std::path::PathBuf, impl Drop) {
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).unwrap();
        let temporary = std::env::temp_dir().join(format!(
            "gwz-platform-{label}-{}-{}",
            std::process::id(),
            hex(&random)
        ));
        std::fs::create_dir(&temporary).unwrap();
        let cleanup = Cleanup(temporary.clone());
        (temporary, cleanup)
    }

    // Native destination-window test (R2-F; amendment §4.1 erratum,
    // GwzM5-8R2CCatalogBootstrapAmendment.md:315-316). A same-user actor
    // renames the destination directory away and plants a replacement at
    // the same absolute path inside the window between destination-path
    // derivation and the handle rename. Per the recorded residual the
    // primitive itself cannot prevent the re-binding; the mandatory
    // post-publish verification through the retained destination handle
    // must detect the redirect read-only (or the primitive fails with the
    // source untouched — either outcome is the specified rejection).
    #[test]
    fn destination_window_substitution_is_detected_through_the_retained_handle() {
        let (temporary, _cleanup) = window_fixture("destination-window");
        let source_path = temporary.join("source");
        let original = temporary.join("destination-dir");
        let displaced = temporary.join("displaced-destination");
        std::fs::write(&source_path, b"checked\n").unwrap();
        std::fs::create_dir(&original).unwrap();
        let root = Dir::open_ambient_dir(&temporary, ambient_authority()).unwrap();
        // Production retains destination directories with DELETE sharing
        // (open_dir_share_delete), which is exactly what leaves the window
        // open to a same-user rename of the destination directory.
        let destination_dir = open_dir_share_delete(&root, OsStr::new("destination-dir")).unwrap();
        let source = open_rename_source(
            &root,
            OsStr::new("source"),
            ErrorCode::IoError,
            "Windows destination-window test",
        )
        .unwrap();

        let substituted = Arc::new(AtomicBool::new(false));
        run_next_checked_artifact_at(CheckedArtifactFault::AfterDestinationPathDerivation, {
            let substituted = Arc::clone(&substituted);
            let original = original.clone();
            let displaced = displaced.clone();
            move || {
                std::fs::rename(&original, &displaced).unwrap();
                std::fs::create_dir(&original).unwrap();
                substituted.store(true, Ordering::SeqCst);
            }
        });
        let result = rename_open_source(
            &source,
            &destination_dir,
            OsStr::new("delivered"),
            false,
            ErrorCode::IoError,
            "Windows destination-window test",
        );
        assert!(
            substituted.load(Ordering::SeqCst),
            "destination-window hook was not reached"
        );

        // The retained destination handle follows the displaced original
        // directory, so the read-only verification through it must reject
        // the published name regardless of where the stale absolute path
        // delivered the object.
        assert!(
            destination_dir.metadata("delivered").is_err(),
            "retained-handle verification must reject the redirect"
        );
        assert!(!displaced.join("delivered").exists());
        match result {
            Ok(()) => {
                // The recorded §4.1 residual: the rename resolved the stale
                // absolute path into the replacement directory. Detection is
                // the retained-handle rejection asserted above.
                assert_eq!(
                    std::fs::read(original.join("delivered")).unwrap(),
                    b"checked\n"
                );
                assert!(!source_path.exists());
            }
            Err(_) => {
                // Also within the specified rejection: the primitive failed
                // with the source untouched and never delivered into the
                // replacement directory.
                assert_eq!(std::fs::read(&source_path).unwrap(), b"checked\n");
                assert!(!original.join("delivered").exists());
            }
        }
    }

    // Ancestor variant of the destination-window test
    // (GwzM5-8R2C2OwnerInterface-ReviewState-2.md:291-296): renaming a path
    // ancestor inside the window must fail the publication with the source
    // untouched and never deliver into the replacement ancestor.
    //
    // On real Windows the OS itself supplies that guarantee one level
    // earlier, so the DENIAL is what this test asserts: the retained
    // destination handle lives under `ancestor`, and Windows refuses to
    // rename a directory that still has an open handle anywhere beneath it
    // (os error 5, or 32 when the collision is spelled as a sharing
    // violation on the renamed directory itself — the same OS-level pin
    // asserted positively by `retained_directory_blocks_substitution_rename_windows`
    // in src/checked_artifact/tests.rs). Both outcomes are covered below:
    // when the OS denies the substitution there is no replacement ancestor
    // to deliver into at all, and when it permits one the original
    // source-untouched assertions apply unchanged.
    #[test]
    fn destination_window_ancestor_substitution_fails_with_the_source_untouched() {
        let (temporary, _cleanup) = window_fixture("destination-ancestor");
        let source_path = temporary.join("source");
        let ancestor = temporary.join("ancestor");
        let displaced = temporary.join("displaced-ancestor");
        std::fs::write(&source_path, b"checked\n").unwrap();
        std::fs::create_dir(&ancestor).unwrap();
        std::fs::create_dir(ancestor.join("destination-dir")).unwrap();
        let root = Dir::open_ambient_dir(&temporary, ambient_authority()).unwrap();
        let ancestor_dir = Dir::open_ambient_dir(&ancestor, ambient_authority()).unwrap();
        let destination_dir =
            open_dir_share_delete(&ancestor_dir, OsStr::new("destination-dir")).unwrap();
        // Release the plain (non-share-delete) ancestor handle before the
        // window; production never retains such a handle across the edge.
        drop(ancestor_dir);
        let source = open_rename_source(
            &root,
            OsStr::new("source"),
            ErrorCode::IoError,
            "Windows destination-window test",
        )
        .unwrap();

        let substituted = Arc::new(AtomicBool::new(false));
        let denial: Arc<Mutex<Option<std::io::Error>>> = Arc::new(Mutex::new(None));
        run_next_checked_artifact_at(CheckedArtifactFault::AfterDestinationPathDerivation, {
            let substituted = Arc::clone(&substituted);
            let denial = Arc::clone(&denial);
            let ancestor = ancestor.clone();
            let displaced = displaced.clone();
            move || {
                match std::fs::rename(&ancestor, &displaced) {
                    // Empty replacement ancestor: the stale absolute path can
                    // no longer resolve, and nothing may be delivered into it.
                    Ok(()) => std::fs::create_dir(&ancestor).unwrap(),
                    // The retained destination handle beneath `ancestor` makes
                    // the OS refuse the substitution outright.
                    Err(error) => *denial.lock().unwrap() = Some(error),
                }
                substituted.store(true, Ordering::SeqCst);
            }
        });
        let result = rename_open_source(
            &source,
            &destination_dir,
            OsStr::new("delivered"),
            false,
            ErrorCode::IoError,
            "Windows destination-window test",
        );
        assert!(
            substituted.load(Ordering::SeqCst),
            "destination-window hook was not reached"
        );

        let denial = denial.lock().unwrap().take();
        match denial {
            Some(error) => {
                // The asserted guarantee: the OS denied the ancestor rename,
                // so the substitution never happened.
                assert!(
                    matches!(error.raw_os_error(), Some(5 | 32)),
                    "ancestor rename over a retained descendant handle must be \
                     denied as a Windows sharing collision: {error:?}"
                );
                // No replacement ancestor exists, so nothing can have been
                // delivered into one, and the intact path published through
                // the retained destination handle exactly once.
                assert!(!displaced.exists());
                assert!(
                    result.is_ok(),
                    "a denied substitution must leave the publication intact: {result:?}"
                );
                assert!(!source_path.exists());
                assert_eq!(
                    std::fs::read(ancestor.join("destination-dir").join("delivered")).unwrap(),
                    b"checked\n"
                );
                assert!(destination_dir.metadata("delivered").is_ok());
            }
            None => {
                // The substitution landed: the recorded property applies —
                // the publication fails with the source untouched and nothing
                // reaches either the replacement or the displaced ancestor.
                assert!(
                    result.is_err(),
                    "ancestor substitution inside the window must fail the publication"
                );
                assert_eq!(std::fs::read(&source_path).unwrap(), b"checked\n");
                assert_eq!(std::fs::read_dir(&ancestor).unwrap().count(), 0);
                assert!(!displaced.join("destination-dir").join("delivered").exists());
                assert!(destination_dir.metadata("delivered").is_err());
            }
        }
    }
}

#[cfg(any(windows, test))]
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn io_error(code: ErrorCode, label: &str, cause: std::io::Error) -> ModelError {
    error(code, label, cause)
}

fn error(code: ErrorCode, label: &str, detail: impl std::fmt::Display) -> ModelError {
    ModelError::new(code, format!("checked {label}: {detail}"))
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use cap_std::fs::Dir;

    #[test]
    fn sync_parent_flushes_a_live_linux_directory_capability() {
        // Red before the reopen seam: cap-std directory capabilities are
        // `O_PATH` on Linux, and syncing a dup of the capability reported
        // `EBADF` on every Linux filesystem — the ARM64 matrix substrate
        // class (observation.rs / transition.rs sync sites).
        let root = std::env::temp_dir().join(format!(
            "gwz-sync-parent-linux-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir(&root).unwrap();
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        let synced = super::sync_parent(&dir);
        let _ = std::fs::remove_dir_all(&root);
        synced.expect("sync_parent must flush a live cap-std directory capability on Linux");
    }
}
