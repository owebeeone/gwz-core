//! The Windows ReFS block-cloning wrapper, gated to Windows targets.

#[cfg(windows)]
pub(crate) mod imp {
    //! `DeviceIoControl(temporary, FSCTL_DUPLICATE_EXTENTS_TO_FILE, ...)`.
    //!
    //! ReFS block cloning duplicates a *cluster-aligned extent range* between
    //! two open files on one volume. Unlike `clonefile` and `FICLONE`, which
    //! take a whole file, this is a range operation, so the wrapper has to
    //! build a range the filesystem will accept:
    //!
    //! - The destination is created here, made sparse first when the source is
    //!   sparse -- the call refuses a pair whose sparseness differs -- and
    //!   pre-sized to the source's length, because the call also refuses a
    //!   target region past end of file.
    //! - Offsets and byte counts are cluster-aligned, and the final range is
    //!   rounded up; see [`crate::native::block_clone`] for why that is the documented
    //!   pattern rather than a shortcut.
    //! - A file larger than
    //!   [`MAX_DUPLICATE_BYTES`](crate::native::block_clone::MAX_DUPLICATE_BYTES) is
    //!   duplicated by a short sequence of calls. It is still one work unit
    //!   for the engine: there is no cancellation point inside it, exactly as
    //!   for a `clonefile` of any size.
    //!
    //! Like the Linux wrapper, this one creates the temporary itself and
    //! removes it again unless every range was duplicated, so the engine finds
    //! either a complete clone or no file at all.
    //!
    //! Whether a *volume* can block-clone at all is asked before anything is
    //! created (`FILE_SUPPORTS_BLOCK_REFCOUNTING`), which is what keeps an
    //! NTFS copy from creating, pre-sizing and removing a temporary for every
    //! file on its way to the ordinary path. A volume that says yes is still
    //! not a promise: integrity streams, sparseness and reference limits are
    //! decided per file, by the operation, as everywhere else in this module.

    // The whole of the crate's `unsafe`, and only ever a call into the four
    // documented Win32 entry points below; every buffer they are given is
    // owned by the frame that makes the call. See `crate`'s lint header.
    #![allow(unsafe_code)]

    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use gwz_copy_contract::CopyErrorCategory;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_SPARSE_FILE, GetDiskFreeSpaceW, GetVolumeInformationW, GetVolumePathNameW,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        DUPLICATE_EXTENTS_DATA, FSCTL_DUPLICATE_EXTENTS_TO_FILE, FSCTL_SET_SPARSE,
    };
    use windows_sys::Win32::System::SystemServices::FILE_SUPPORTS_BLOCK_REFCOUNTING;

    use crate::NativeMechanism;
    use crate::native::block_clone::next_range;
    use crate::native::{Outcome, classify};

    pub(crate) const MECHANISM: NativeMechanism = NativeMechanism::WindowsBlockClone;

    /// UTF-16 units reserved for a `GetVolumePathNameW` answer. A mount point
    /// is usually `X:\`, but a volume can be mounted on a directory path, so
    /// this is generous; a path that still does not fit answers `None` and the
    /// copy falls back rather than guessing a geometry.
    const VOLUME_PATH_UNITS: usize = 1024;

    /// What the destination volume says about itself: enough to build a legal
    /// duplicate-extents request, and whether it could serve one at all.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct VolumeFacts {
        /// The volume serial number: this platform's device identity, and what
        /// the call means by "the same volume".
        pub(crate) serial: u32,
        /// Allocation unit in bytes. Every offset and byte count in a request
        /// is a multiple of it.
        pub(crate) cluster_bytes: u64,
        /// Whether the filesystem advertises sharing logical clusters between
        /// files. `false` rules the call out; `true` promises nothing.
        pub(crate) block_cloning: bool,
    }

    pub(crate) fn clone_regular_file(source: &File, temporary: &Path) -> Outcome {
        let Some(parent) = temporary.parent() else {
            return Outcome::Failed(
                CopyErrorCategory::DestinationUnwritable,
                format!("{} has no directory to be created in", temporary.display()),
            );
        };
        // A missing or non-directory parent is a destination error, even on
        // a volume without block cloning. Validate it before capability probing.
        match std::fs::metadata(parent) {
            Ok(metadata) if metadata.is_dir() => {}
            result => {
                return Outcome::Failed(
                    CopyErrorCategory::DestinationUnwritable,
                    format!(
                        "{} is not an accessible destination directory: {result:?}",
                        parent.display()
                    ),
                );
            }
        }
        let Some(volume) = facts(parent) else {
            return Outcome::Unsupported(format!(
                "the volume holding {} could not describe itself, so no cluster-aligned \
                 duplicate-extents request could be built for it",
                parent.display()
            ));
        };
        if !volume.block_cloning {
            return Outcome::Unsupported(
                "the destination filesystem does not report FILE_SUPPORTS_BLOCK_REFCOUNTING, so \
                 it cannot share blocks between files (ReFS does; NTFS, FAT and exFAT do not)"
                    .to_owned(),
            );
        }
        let metadata = match source.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                return Outcome::Failed(
                    CopyErrorCategory::SourceUnreadable,
                    format!("the source could not be measured: {error}"),
                );
            }
        };
        let destination = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary)
        {
            Ok(destination) => destination,
            Err(error) => {
                return Outcome::Failed(
                    CopyErrorCategory::DestinationUnwritable,
                    format!("the temporary could not be created: {error}"),
                );
            }
        };
        let outcome = duplicate_whole_file(
            source,
            &destination,
            metadata.len(),
            metadata.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE != 0,
            volume.cluster_bytes,
        );
        // Close before the engine renames or recreates the name.
        drop(destination);
        if outcome != Outcome::Cloned {
            // Reset the file this call created, so the fallback starts from
            // nothing rather than appending to a partial attempt.
            let _ = fs::remove_file(temporary);
        }
        outcome
    }

    /// Give `destination`, an empty file this call created, every extent of
    /// `source`.
    fn duplicate_whole_file(
        source: &File,
        destination: &File,
        length: u64,
        sparse: bool,
        cluster_bytes: u64,
    ) -> Outcome {
        if sparse && let Err(error) = set_sparse(destination) {
            // The call refuses a pair whose sparseness differs, so a temporary
            // that will not become sparse simply cannot be cloned into. That is
            // a property of this pair, not a failure of the copy.
            return Outcome::Unsupported(format!(
                "the temporary could not be made sparse to match the source: {error}"
            ));
        }
        // Pre-size to the source's *logical* length before any duplication:
        // the call refuses a target region past end of file, and this is what
        // allocates the trailing partial cluster the rounded-up final range
        // lands in. The destination's end of file stays the source's length.
        if let Err(error) = destination.set_len(length) {
            return Outcome::Failed(
                CopyErrorCategory::DestinationUnwritable,
                format!("the temporary could not be pre-sized to {length} bytes: {error}"),
            );
        }
        let mut offset = 0u64;
        // An empty source yields no range at all: it has no extents to share.
        // The pre-sized temporary is already the whole file, and no byte of it
        // was streamed, so it is the native path's -- the same answer
        // `clonefile` gives for an empty file.
        while let Some(range) = next_range(offset, length, cluster_bytes) {
            if let Err(outcome) = duplicate_range(source, destination, range.offset, range.count) {
                return outcome;
            }
            offset += range.advance;
        }
        Outcome::Cloned
    }

    /// One `FSCTL_DUPLICATE_EXTENTS_TO_FILE`. `Err` carries the classified
    /// outcome, so the caller stops at the first range the volume refuses.
    fn duplicate_range(
        source: &File,
        destination: &File,
        offset: u64,
        count: u64,
    ) -> Result<(), Outcome> {
        let request = DUPLICATE_EXTENTS_DATA {
            FileHandle: source.as_raw_handle() as HANDLE,
            SourceFileOffset: offset as i64,
            TargetFileOffset: offset as i64,
            ByteCount: count as i64,
        };
        let mut returned = 0u32;
        // SAFETY: `destination` and `source` are open files this call holds
        // borrows of, so both handles are live for its duration; `request` is
        // a live `DUPLICATE_EXTENTS_DATA` described by its own size; the
        // output buffer is declined with a null pointer and a zero length,
        // which this control code documents as taking none; `returned` is a
        // live `u32`; and the null `OVERLAPPED` requests the synchronous form,
        // which is what a handle opened without `FILE_FLAG_OVERLAPPED` needs.
        let ok = unsafe {
            DeviceIoControl(
                destination.as_raw_handle() as HANDLE,
                FSCTL_DUPLICATE_EXTENTS_TO_FILE,
                (&raw const request).cast(),
                size_of::<DUPLICATE_EXTENTS_DATA>() as u32,
                std::ptr::null_mut(),
                0,
                &raw mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        let code = error.raw_os_error().unwrap_or_default() as u32;
        Err(classify::describe(
            classify::duplicate_extents(code),
            error,
            "FSCTL_DUPLICATE_EXTENTS_TO_FILE",
        ))
    }

    /// Mark a file sparse, so its sparseness matches a sparse source's.
    fn set_sparse(destination: &File) -> io::Result<()> {
        let mut returned = 0u32;
        // SAFETY: as in `duplicate_range`; this control code takes neither an
        // input nor an output buffer, both of which are declined with a null
        // pointer and a zero length.
        let ok = unsafe {
            DeviceIoControl(
                destination.as_raw_handle() as HANDLE,
                FSCTL_SET_SPARSE,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &raw mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Describe the volume `path` is on. `None` when it cannot be described,
    /// which rules nothing out: the caller falls back for that file.
    pub(crate) fn facts(path: &Path) -> Option<VolumeFacts> {
        let volume = volume_path(path)?;
        let mut sectors_per_cluster = 0u32;
        let mut bytes_per_sector = 0u32;
        let mut free_clusters = 0u32;
        let mut total_clusters = 0u32;
        // SAFETY: `volume` is a NUL-terminated UTF-16 buffer owned by this
        // frame, and each out parameter is a live `u32` it also owns.
        let ok = unsafe {
            GetDiskFreeSpaceW(
                volume.as_ptr(),
                &raw mut sectors_per_cluster,
                &raw mut bytes_per_sector,
                &raw mut free_clusters,
                &raw mut total_clusters,
            )
        };
        let cluster_bytes = u64::from(sectors_per_cluster) * u64::from(bytes_per_sector);
        if ok == 0 || cluster_bytes == 0 {
            return None;
        }

        let mut serial = 0u32;
        let mut component = 0u32;
        let mut flags = 0u32;
        // SAFETY: as above; the volume-name and filesystem-name buffers are
        // declined with a null pointer and a zero length, which this call
        // documents as "not wanted".
        let ok = unsafe {
            GetVolumeInformationW(
                volume.as_ptr(),
                std::ptr::null_mut(),
                0,
                &raw mut serial,
                &raw mut component,
                &raw mut flags,
                std::ptr::null_mut(),
                0,
            )
        };
        if ok == 0 {
            return None;
        }
        Some(VolumeFacts {
            serial,
            cluster_bytes,
            block_cloning: flags & FILE_SUPPORTS_BLOCK_REFCOUNTING != 0,
        })
    }

    /// The mount point of the volume `path` is on, NUL-terminated, ready to
    /// pass to the volume queries above.
    fn volume_path(path: &Path) -> Option<Vec<u16>> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        let mut volume = vec![0u16; VOLUME_PATH_UNITS];
        // SAFETY: both buffers are owned by this frame and outlive the call;
        // `wide` is NUL-terminated and `volume` is described by its own length.
        let ok =
            unsafe { GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) };
        (ok != 0).then_some(volume)
    }
}
