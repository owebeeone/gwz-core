use std::ffi::OsStr;

use cap_std::fs::Dir;

use crate::model::{ErrorCode, ModelError, ModelResult};

#[cfg(not(windows))]
pub(super) fn prepare_private(
    _dir: &Dir,
    _create: bool,
    _code: ErrorCode,
    _label: &str,
) -> ModelResult<()> {
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn private_barrier(dir: &Dir, code: ErrorCode, label: &str) -> ModelResult<()> {
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
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;

    let mut options = OpenOptions::new();
    options
        .access_mode(DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH)
        .follow(FollowSymlinks::No)
        // The same no-replace primitive publishes both regular-file records
        // and the first-catalog staging directory.
        .maybe_dir(true);
    let source = source_dir
        .open_with(source, &options)
        .map_err(|cause| io_error(code, label, cause))?;
    let name = destination.encode_wide().collect::<Vec<_>>();
    let size = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * 2;
    let mut storage = vec![0_usize; size.div_ceil(std::mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = replace;
        (*info).RootDirectory = destination_dir.as_raw_handle();
        (*info).FileNameLength = u32::try_from(name.len() * 2)
            .map_err(|_| error(code, label, "destination name is too long"))?;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
        if SetFileInformationByHandle(
            source.as_raw_handle(),
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

#[cfg(not(windows))]
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
const ANCHOR_BYTES: &[u8] = b"GWZ-CHECKED-ARTIFACT-DURABILITY-ANCHOR-V1\n";

#[cfg(windows)]
const ANCHOR_PREFIX: &str = ".ca1-durability-anchor-";

#[cfg(windows)]
pub(super) fn prepare_private(
    dir: &Dir,
    create: bool,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_WRITE_THROUGH;

    let state = anchor_state(dir, code, label)?;
    match state {
        AnchorState::Ready { .. } => Ok(()),
        AnchorState::NeedsReturn {
            roundtrip,
            final_name,
        } => {
            rename_relative(dir, &roundtrip, dir, &final_name, false, code, label)?;
            verify_anchor(dir, &final_name, code, label).map(|_| ())
        }
        AnchorState::NeedsRetireAlias { alias, final_name } => {
            verify_anchor(dir, &final_name, code, label)?;
            dir.remove_file(&alias)
                .map_err(|cause| io_error(code, label, cause))?;
            private_barrier(dir, code, label)
        }
        AnchorState::Missing { family_state } if !family_state && !create => Ok(()),
        AnchorState::Missing {
            family_state: false,
        } => {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random).map_err(|cause| {
                error(code, label, format!("random anchor name failed: {cause}"))
            })?;
            let scratch = format!(".ca1-anchor-scratch-{}", hex(&random));
            let mut options = OpenOptions::new();
            options
                .write(true)
                .create_new(true)
                .custom_flags(FILE_FLAG_WRITE_THROUGH)
                .follow(FollowSymlinks::No);
            let mut file = dir
                .open_with(&scratch, &options)
                .map_err(|cause| io_error(code, label, cause))?;
            use std::io::Write;
            file.write_all(ANCHOR_BYTES)
                .map_err(|cause| io_error(code, label, cause))?;
            file.sync_all()
                .map_err(|cause| io_error(code, label, cause))?;
            let identity = super::identity::file_identity(&file)
                .map_err(|cause| io_error(code, label, cause))?;
            drop(file);
            let final_name = anchor_name(&identity.name_digest());
            rename_relative(
                dir,
                scratch.as_ref(),
                dir,
                final_name.as_ref(),
                false,
                code,
                label,
            )?;
            verify_anchor(dir, final_name.as_ref(), code, label).map(|_| ())
        }
        AnchorState::Missing { family_state: true } | AnchorState::Invalid => Err(error(
            code,
            label,
            "private durability anchor is missing or ambiguous while family state exists",
        )),
    }
}

#[cfg(windows)]
pub(super) fn private_barrier(dir: &Dir, code: ErrorCode, label: &str) -> ModelResult<()> {
    use super::fault::{CheckedArtifactFault, fault};

    prepare_private(dir, false, code, label)?;
    let AnchorState::Ready { final_name } = anchor_state(dir, code, label)? else {
        return Err(error(code, label, "private durability anchor is not ready"));
    };
    let roundtrip = format!("{final_name}.roundtrip");
    fault(CheckedArtifactFault::BeforeAnchorRoundTrip, code, label)?;
    rename_relative(
        dir,
        final_name.as_ref(),
        dir,
        roundtrip.as_ref(),
        false,
        code,
        label,
    )?;
    fault(CheckedArtifactFault::AfterAnchorOutboundRename, code, label)?;
    let moved = verify_anchor(dir, roundtrip.as_ref(), code, label)?;
    rename_relative(
        dir,
        roundtrip.as_ref(),
        dir,
        final_name.as_ref(),
        false,
        code,
        label,
    )?;
    fault(CheckedArtifactFault::AfterAnchorReturnRename, code, label)?;
    let returned = verify_anchor(dir, final_name.as_ref(), code, label)?;
    if moved != returned {
        return Err(error(code, label, "durability anchor identity changed"));
    }
    fault(CheckedArtifactFault::AfterAnchorReobservation, code, label)
}

#[cfg(windows)]
#[derive(Debug)]
enum AnchorState {
    Ready {
        final_name: std::ffi::OsString,
    },
    NeedsReturn {
        roundtrip: std::ffi::OsString,
        final_name: std::ffi::OsString,
    },
    NeedsRetireAlias {
        alias: std::ffi::OsString,
        final_name: std::ffi::OsString,
    },
    Missing {
        family_state: bool,
    },
    Invalid,
}

#[cfg(windows)]
fn anchor_state(dir: &Dir, code: ErrorCode, label: &str) -> ModelResult<AnchorState> {
    let mut anchors = Vec::new();
    let mut family_state = false;
    for entry in dir
        .entries()
        .map_err(|cause| io_error(code, label, cause))?
    {
        let entry = entry.map_err(|cause| io_error(code, label, cause))?;
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if text.starts_with(ANCHOR_PREFIX) {
            anchors.push(name);
        } else if text.starts_with("ca1-") {
            family_state = true;
        }
    }
    if anchors.is_empty() {
        return Ok(AnchorState::Missing { family_state });
    }
    if anchors.len() > 2 {
        return Ok(AnchorState::Invalid);
    }
    let mut final_entry = None;
    let mut roundtrip_entry = None;
    for name in anchors {
        let identity = verify_anchor(dir, &name, code, label)?;
        let expected = anchor_name(&identity.name_digest());
        if name == expected {
            if final_entry.replace((name, identity)).is_some() {
                return Ok(AnchorState::Invalid);
            }
        } else if name == format!("{expected}.roundtrip") {
            if roundtrip_entry
                .replace((name, expected, identity))
                .is_some()
            {
                return Ok(AnchorState::Invalid);
            }
        } else {
            return Ok(AnchorState::Invalid);
        }
    }
    match (final_entry, roundtrip_entry) {
        (Some((final_name, _)), None) => Ok(AnchorState::Ready { final_name }),
        (None, Some((roundtrip, final_name, _))) => Ok(AnchorState::NeedsReturn {
            roundtrip,
            final_name: final_name.into(),
        }),
        (Some((final_name, final_identity)), Some((alias, expected, alias_identity)))
            if final_name == expected && final_identity == alias_identity =>
        {
            Ok(AnchorState::NeedsRetireAlias { alias, final_name })
        }
        _ => Ok(AnchorState::Invalid),
    }
}

#[cfg(windows)]
fn verify_anchor(
    dir: &Dir,
    name: &OsStr,
    code: ErrorCode,
    label: &str,
) -> ModelResult<super::identity::ObjectIdentity> {
    let observed = super::observation::observe_leaf_exact(dir, name, code, label)?;
    if observed.fact != super::CheckedArtifactFact::Bytes(ANCHOR_BYTES.to_vec()) {
        return Err(error(
            code,
            label,
            "private durability anchor bytes are invalid",
        ));
    }
    observed
        .identity
        .ok_or_else(|| error(code, label, "private durability anchor lacks identity"))
}

#[cfg(windows)]
fn anchor_name(identity: &[u8; 16]) -> String {
    format!("{ANCHOR_PREFIX}{}", hex(identity))
}

#[cfg(windows)]
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn io_error(code: ErrorCode, label: &str, cause: std::io::Error) -> ModelError {
    error(code, label, cause)
}

fn error(code: ErrorCode, label: &str, detail: impl std::fmt::Display) -> ModelError {
    ModelError::new(code, format!("checked {label}: {detail}"))
}
