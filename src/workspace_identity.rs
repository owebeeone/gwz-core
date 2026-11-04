//! The one identity of a workspace root (step 2.3f of `dev-docs/GwzServerCore-Step2.3-Plan.md`, carried in from the
//! 2.3c round-2 reviews, State R2S-1 = Code P3-N1): what the filesystem says the root directory is, not how a request
//! spelled it. The host's member lock manager keys a workspace by it, and the server's `id-<key>.lock` (one server per
//! directory, whatever the spelling) hashes it, so the member lock and the server agree on every spelling of one root.
//! The server's `ws-<key>` names do not: they hash the canonical root path (operator decision 2026-11-03, 2.3f round 2
//! Code P2-1, option (a)), so that an address-keyed grant names one path.
//!
//! A leaf: it imports nothing from the crate, so the member lock (`operation`) and the local carrier's listener files
//! (`transport_host::carrier`, extracted into a crate at 2.6) both depend on it and not on each other (2.3f round 1
//! Code P3-2).
//!
//! | Platform | Identity | Spellings that are one identity |
//! | --- | --- | --- |
//! | Linux, macOS | `(st_dev, st_ino)` of the root, links followed (`stat`) | a symlink, `..`, a macOS firmlink (`/System/Volumes/Data/...`), a bind mount, a case variant on a case-folding volume or directory |
//! | Windows | the 64-bit volume serial number and the 128-bit file ID (`GetFileInformationByHandleEx(FileIdInfo)` on the opened directory, reparse points followed); the 64-bit file index is not unique on ReFS (Dev Drives), the 128-bit ID is | a junction or symlink, a case variant, a short (8.3) name, a `\\?\` form |
//!
//! A root whose identity cannot be read (it does not exist, it cannot be opened, or Windows reports an all-zero file ID)
//! has none, and the caller decides what that means: the host manager refuses the step, the per-call manager keys by
//! the spelling instead.

use std::io;
use std::path::Path;

/// The filesystem's identity of a directory: the volume and the file on it. Two spellings are one directory exactly
/// when their identities are equal, for as long as the directory exists.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WorkspaceIdentity {
    /// The device (Unix) or the volume serial number (Windows).
    pub(crate) volume: u64,
    /// The inode (Unix) or the 128-bit file ID (Windows).
    pub(crate) file: u128,
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// The identity of the directory at `root`, links followed.
        ///
        /// # Errors
        ///
        /// The `stat` error.
        pub(crate) fn workspace_identity(root: &Path) -> io::Result<WorkspaceIdentity> {
            use std::os::unix::fs::MetadataExt;
            let meta = std::fs::metadata(root)?;
            Ok(WorkspaceIdentity { volume: meta.dev(), file: u128::from(meta.ino()) })
        }
    } else if #[cfg(windows)] {
        use windows_sys::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx},
        };

        /// The identity of the open file or directory `handle`: its volume serial number and its 128-bit file ID. An
        /// all-zero ID (a redirector that reports none) is no identity.
        ///
        /// # Errors
        ///
        /// The query's error, or `Unsupported` for an all-zero ID.
        pub(crate) fn handle_identity(handle: HANDLE) -> io::Result<WorkspaceIdentity> {
            // SAFETY: plain data, written by the call below.
            let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
            // SAFETY: a live handle and a writable structure of the stated size.
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    handle,
                    FileIdInfo,
                    (&raw mut info).cast(),
                    std::mem::size_of::<FILE_ID_INFO>() as u32,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            let file = u128::from_le_bytes(info.FileId.Identifier);
            if file == 0 {
                return Err(io::Error::new(io::ErrorKind::Unsupported, "the volume reports no file ID"));
            }
            Ok(WorkspaceIdentity { volume: info.VolumeSerialNumber, file })
        }

        /// The identity of the directory at `root`, reparse points followed.
        ///
        /// # Errors
        ///
        /// The open or the query error.
        pub(crate) fn workspace_identity(root: &Path) -> io::Result<WorkspaceIdentity> {
            use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            };
            // Access 0 reads attributes only; backup semantics opens a directory.
            let file = std::fs::OpenOptions::new()
                .access_mode(0)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(root)?;
            handle_identity(file.as_raw_handle())
        }
    }
}
