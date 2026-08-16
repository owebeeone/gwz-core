use std::ffi::{OsStr, OsString};

use cap_std::fs::Dir;

use crate::model::{ErrorCode, ModelError, ModelResult};

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
    let roundtrip = anchor_roundtrip_name(&final_name);
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
        let expected = std::ffi::OsString::from(anchor_name(&identity.name_digest()));
        if name == expected {
            if final_entry.replace((name, identity)).is_some() {
                return Ok(AnchorState::Invalid);
            }
        } else if name == anchor_roundtrip_name(&expected) {
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
            final_name,
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
fn anchor_roundtrip_name(final_name: &OsStr) -> std::ffi::OsString {
    let mut roundtrip = final_name.to_os_string();
    roundtrip.push(".roundtrip");
    roundtrip
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::super::fault::{CheckedArtifactFault, run_next_checked_artifact_at};
    use super::{
        anchor_roundtrip_name, hex, open_dir_share_delete, open_rename_source, rename_open_source,
    };
    use cap_std::{ambient_authority, fs::Dir};
    use std::ffi::{OsStr, OsString};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::model::ErrorCode;

    #[test]
    fn anchor_roundtrip_name_remains_native() {
        assert_eq!(
            anchor_roundtrip_name(OsStr::new(".ca1-durability-anchor-0123")),
            OsString::from(".ca1-durability-anchor-0123.roundtrip")
        );
    }

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
