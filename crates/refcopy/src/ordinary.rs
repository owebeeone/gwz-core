//! The ordinary (std-only) tree copy: admission, traversal and per-entry copy.
//!
//! This module is the whole product copier in this build. Native
//! copy-on-write (Apple `clonefile`, Linux `FICLONE`, Windows block cloning)
//! needs a platform dependency the boundary gate does not admit yet, so
//! [`CopyMode::Auto`] copies ordinarily and says so in the report's warnings;
//! nothing here ever claims a native mechanism ran
//! (gwz-dev `dev-docs/GwzLocalCloneDesign.md` §12, "Native copy unavailable →
//! ordinary independent copy; actual method reported").
//!
//! Shape of one copy (`dev-docs/GwzLocalCloneImplementationArchitecture.md` §3):
//!
//! - Admission first: the source must be an existing directory, the
//!   destination a new path or an admitted empty directory, and the two must
//!   not overlap. A refusal writes nothing.
//! - Traversal is iterative and deterministic (entry names sorted per
//!   directory), so a deep tree cannot exhaust the stack and two runs of the
//!   same tree produce the same order.
//! - Exclusions are tested before an entry is copied; an excluded entry is
//!   never written and then removed.
//! - Every regular file is written to a sibling temporary file and renamed
//!   into place, so an interrupted entry never appears under its final name;
//!   the temporary is removed on failure, which keeps the partial report equal
//!   to what the destination actually holds.
//! - Failures retain the partial destination and carry the accurate partial
//!   report. The source is only ever read.

// Ordinary file I/O in a filesystem copier: this crate is outside gwz-core's
// merge-writer boundary (gwz-core/clippy.toml), whose disallowed writers exist
// to route *merge artifact* mutation through checked entries.
#![allow(clippy::disallowed_methods)]

use std::ffi::OsString;
use std::fs::{self, File, Permissions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::vec::IntoIter;

use gwz_copy_contract::{
    Cancellation, CopyError, CopyErrorCategory, CopyMode, CopyReport, CopyRequest, CopyWarning,
    CopyWarningKind,
};

/// Bytes of the single reused copy buffer. One buffer serves the whole copy:
/// the copier holds at most one open source file, one open destination file
/// and this buffer at any time.
const BUFFER_BYTES: usize = 64 * 1024;

/// A failure at one entry: its source-root-relative path, the contract
/// category and diagnostic detail.
type Failure = (PathBuf, CopyErrorCategory, String);

/// Copy `request`'s source tree into its destination.
pub(crate) fn copy_tree(
    request: &CopyRequest,
    cancellation: &dyn Cancellation,
) -> Result<CopyReport, CopyError> {
    let source_permissions = admit_source(request)?;
    let created_root = admit_destination(request)?;
    let mut run = CopyRun {
        request,
        cancellation,
        report: CopyReport {
            warnings: opening_warnings(request.mode),
            ..CopyReport::default()
        },
        buffer: vec![0u8; BUFFER_BYTES],
        temporaries: 0,
    };
    // The root frame carries the source root's permissions only when this call
    // created the destination root; an admitted pre-existing directory belongs
    // to the caller and its mode is left alone.
    let root_permissions = created_root.then_some(source_permissions);
    if let Some(permissions) = &root_permissions
        && let Err(error) = narrow_new_directory(permissions, &request.destination)
    {
        return Err(CopyError::refused(
            &request.destination,
            CopyErrorCategory::MetadataFailed,
            format!("destination permissions could not be applied: {error}"),
        ));
    }
    match run.walk(root_permissions) {
        Ok(()) => Ok(run.report),
        Err((failed_path, category, detail)) => Err(CopyError {
            failed_path,
            category,
            detail,
            partial: run.report,
        }),
    }
}

/// The two copy-wide observations every report carries.
fn opening_warnings(mode: CopyMode) -> Vec<CopyWarning> {
    // A copy-wide warning names the copy itself: the empty relative path, the
    // same way the traversal names the source root.
    let mut warnings = Vec::new();
    if mode == CopyMode::Auto {
        warnings.push(CopyWarning {
            path: PathBuf::new(),
            kind: CopyWarningKind::NativeUnsupportedFellBack,
            detail: "native copy-on-write is unavailable in this build (no platform \
                     copy-on-write dependency is linked); every regular file was copied by \
                     ordinary read/write"
                .to_owned(),
        });
    }
    warnings.push(CopyWarning {
        path: PathBuf::new(),
        kind: CopyWarningKind::AncillaryMetadataUnsupported,
        detail: "ancillary metadata (ACLs, extended attributes, alternate data streams, \
                 timestamps) was not copied; contents, entry type, symlink target and \
                 permission bits were"
            .to_owned(),
    });
    warnings
}

/// The source must be an existing directory, read without following it.
/// Returns its permissions for the destination root.
fn admit_source(request: &CopyRequest) -> Result<Permissions, CopyError> {
    let metadata = fs::symlink_metadata(&request.source).map_err(|error| {
        let category = if error.kind() == ErrorKind::NotFound {
            CopyErrorCategory::SourceMissing
        } else {
            CopyErrorCategory::SourceUnreadable
        };
        CopyError::refused(&request.source, category, error.to_string())
    })?;
    if metadata.file_type().is_symlink() {
        return Err(CopyError::refused(
            &request.source,
            CopyErrorCategory::SourceUnreadable,
            "source is a symbolic link; the copier does not follow it",
        ));
    }
    if !metadata.is_dir() {
        return Err(CopyError::refused(
            &request.source,
            CopyErrorCategory::SourceUnreadable,
            "source is not a directory",
        ));
    }
    Ok(metadata.permissions())
}

/// Admit the destination and report whether this call created it.
///
/// A new path is created; an existing empty directory is admitted as it is; an
/// existing non-empty directory, a non-directory and a symbolic link refuse
/// before anything is written.
fn admit_destination(request: &CopyRequest) -> Result<bool, CopyError> {
    refuse_overlap(request)?;
    match fs::symlink_metadata(&request.destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(CopyError::refused(
                &request.destination,
                CopyErrorCategory::DestinationNotEmpty,
                "destination is a symbolic link; the copier does not follow it",
            ));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(CopyError::refused(
                &request.destination,
                CopyErrorCategory::DestinationNotEmpty,
                "destination exists and is not a directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(&request.destination).map_err(|error| {
                CopyError::refused(
                    &request.destination,
                    CopyErrorCategory::DestinationUnwritable,
                    error.to_string(),
                )
            })?;
            return Ok(true);
        }
        Err(error) => {
            return Err(CopyError::refused(
                &request.destination,
                CopyErrorCategory::DestinationUnwritable,
                format!("destination could not be examined: {error}"),
            ));
        }
    }
    let mut entries = fs::read_dir(&request.destination).map_err(|error| {
        CopyError::refused(
            &request.destination,
            CopyErrorCategory::DestinationUnwritable,
            format!("destination directory could not be enumerated: {error}"),
        )
    })?;
    if entries.next().is_some() {
        return Err(CopyError::refused(
            &request.destination,
            CopyErrorCategory::DestinationNotEmpty,
            "destination exists and is not empty",
        ));
    }
    Ok(false)
}

/// Ordinary canonical-path check against the obvious mistake of copying a
/// tree into itself. The caller has already rejected registered paths and
/// overlap; this catches the case where that check was never made.
fn refuse_overlap(request: &CopyRequest) -> Result<(), CopyError> {
    let source = fs::canonicalize(&request.source).map_err(|error| {
        CopyError::refused(
            &request.source,
            CopyErrorCategory::SourceUnreadable,
            format!("source path could not be resolved: {error}"),
        )
    })?;
    let Some(destination) = resolve_new_path(&request.destination) else {
        return Ok(());
    };
    if destination.starts_with(&source) {
        return Err(CopyError::refused(
            &request.destination,
            CopyErrorCategory::DestinationUnwritable,
            "destination is inside the source tree",
        ));
    }
    // The reverse containment is only meaningful for a destination that
    // exists: an ancestor of a not-yet-created destination is an ordinary
    // parent directory, not an overlap.
    if request.destination.exists() && source.starts_with(&destination) {
        return Err(CopyError::refused(
            &request.destination,
            CopyErrorCategory::DestinationUnwritable,
            "source is inside the destination tree",
        ));
    }
    Ok(())
}

/// Resolve `path` through its nearest existing ancestor, so a destination that
/// does not exist yet still has a comparable absolute form. `None` when no
/// ancestor resolves.
fn resolve_new_path(path: &Path) -> Option<PathBuf> {
    let mut trailing: Vec<&std::ffi::OsStr> = Vec::new();
    let mut candidate = path;
    loop {
        if let Ok(resolved) = fs::canonicalize(candidate) {
            let mut resolved = resolved;
            for component in trailing.iter().rev() {
                resolved.push(component);
            }
            return Some(resolved);
        }
        let name = candidate.file_name()?;
        trailing.push(name);
        candidate = match candidate.parent()? {
            // A relative path runs out of ancestors at the empty path; the
            // current directory is the ancestor it actually names.
            parent if parent.as_os_str().is_empty() => Path::new("."),
            parent => parent,
        };
    }
}

/// One open directory level of the traversal.
struct Frame {
    /// Source-root-relative path of this directory (empty for the root).
    relative: PathBuf,
    /// Remaining entry names, sorted.
    names: IntoIter<OsString>,
    /// Source permissions applied to the destination directory once every
    /// entry under it is written. Applying them earlier could make the
    /// directory unwritable for its own children.
    permissions: Option<Permissions>,
}

struct CopyRun<'a> {
    request: &'a CopyRequest,
    cancellation: &'a dyn Cancellation,
    report: CopyReport,
    buffer: Vec<u8>,
    temporaries: u64,
}

impl CopyRun<'_> {
    /// Depth-first traversal over an explicit stack.
    fn walk(&mut self, root_permissions: Option<Permissions>) -> Result<(), Failure> {
        let mut frames = vec![self.open_frame(PathBuf::new(), root_permissions)?];
        while let Some(frame) = frames.last_mut() {
            let Some(name) = frame.names.next() else {
                let finished = frames.pop().expect("the frame was just observed");
                self.finish_directory(&finished)?;
                continue;
            };
            let relative = frame.relative.join(name);
            // Cancellation is polled once per entry, before the entry is
            // examined. There is deliberately no poll before the traversal
            // starts: a copy that is admitted always makes progress on its
            // first entry or reports why it could not.
            if self.cancellation.is_cancelled() {
                return Err((
                    relative,
                    CopyErrorCategory::Cancelled,
                    "cancelled before entry".to_owned(),
                ));
            }
            if self.request.is_excluded(&relative) {
                continue;
            }
            if let Some(frame) = self.copy_entry(&relative)? {
                frames.push(frame);
            }
        }
        Ok(())
    }

    /// Copy one entry. Returns the directory frame to descend into, if the
    /// entry was a directory.
    fn copy_entry(&mut self, relative: &Path) -> Result<Option<Frame>, Failure> {
        let source = self.request.source.join(relative);
        let destination = self.request.destination.join(relative);
        let metadata = fs::symlink_metadata(&source).map_err(|error| {
            (
                relative.to_path_buf(),
                CopyErrorCategory::SourceUnreadable,
                error.to_string(),
            )
        })?;
        let file_type = metadata.file_type();
        // A symbolic link is recreated as a link and never followed, so a link
        // to a FIFO, a socket or a directory outside the tree is neither
        // opened nor walked.
        if file_type.is_symlink() {
            self.copy_symlink(relative, &source, &destination)?;
            return Ok(None);
        }
        if file_type.is_dir() {
            fs::create_dir(&destination).map_err(|error| {
                (
                    relative.to_path_buf(),
                    destination_category(&error),
                    error.to_string(),
                )
            })?;
            self.report.directories += 1;
            let permissions = metadata.permissions();
            narrow_new_directory(&permissions, &destination).map_err(|error| {
                (
                    relative.to_path_buf(),
                    CopyErrorCategory::MetadataFailed,
                    format!("directory permissions could not be applied: {error}"),
                )
            })?;
            let frame = self.open_frame(relative.to_path_buf(), Some(permissions))?;
            return Ok(Some(frame));
        }
        if file_type.is_file() {
            self.copy_file(relative, &source, &destination)?;
            return Ok(None);
        }
        // FIFOs, sockets, devices and anything else: refused by type, from the
        // link-level metadata alone. Nothing is opened, so nothing can block.
        Err((
            relative.to_path_buf(),
            CopyErrorCategory::UnsupportedEntry,
            format!("unsupported entry type ({})", describe_type(&metadata)),
        ))
    }

    fn copy_symlink(
        &mut self,
        relative: &Path,
        source: &Path,
        destination: &Path,
    ) -> Result<(), Failure> {
        let target = fs::read_link(source).map_err(|error| {
            (
                relative.to_path_buf(),
                CopyErrorCategory::SourceUnreadable,
                error.to_string(),
            )
        })?;
        create_symlink(&target, destination, source).map_err(|error| {
            // A destination that cannot be written is a destination failure;
            // anything else is the symlink target failing to apply.
            let category = if error.kind() == ErrorKind::PermissionDenied {
                CopyErrorCategory::DestinationUnwritable
            } else {
                CopyErrorCategory::MetadataFailed
            };
            (relative.to_path_buf(), category, error.to_string())
        })?;
        self.report.symlinks += 1;
        Ok(())
    }

    /// Copy one regular file through a sibling temporary file and a rename.
    fn copy_file(
        &mut self,
        relative: &Path,
        source: &Path,
        destination: &Path,
    ) -> Result<(), Failure> {
        let at = |category, detail: String| (relative.to_path_buf(), category, detail);
        let mut input = File::open(source)
            .map_err(|error| at(CopyErrorCategory::SourceUnreadable, error.to_string()))?;
        // Read the mode from the open handle, not the path, so the mode
        // applied is the mode of the bytes copied.
        let source_metadata = input
            .metadata()
            .map_err(|error| at(CopyErrorCategory::SourceUnreadable, error.to_string()))?;
        let temporary = self.temporary_path(destination);
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| at(destination_category(&error), error.to_string()))?;

        let outcome = stream_bytes(&mut input, &mut output, &mut self.buffer, self.cancellation)
            .and_then(|bytes| {
                preserve_permissions_on_handle(&source_metadata.permissions(), &output)
                    .map_err(|error| {
                        (
                            CopyErrorCategory::MetadataFailed,
                            format!("permissions could not be applied: {error}"),
                        )
                    })
                    .map(|()| bytes)
            });
        // Close before renaming: an open handle blocks a rename on some
        // platforms.
        drop(output);
        let bytes = match outcome {
            Ok(bytes) => bytes,
            Err((category, detail)) => {
                // The interrupted entry never appears in the destination, so
                // the partial report still equals what is there.
                let detail = match fs::remove_file(&temporary) {
                    Ok(()) => detail,
                    Err(error) => format!("{detail} (incomplete {temporary:?} remains: {error})"),
                };
                return Err(at(category, detail));
            }
        };
        if let Err(error) = fs::rename(&temporary, destination) {
            let detail = error.to_string();
            let _ = fs::remove_file(&temporary);
            return Err(at(destination_category(&error), detail));
        }
        // Counted as soon as the entry exists under its final name: a failure
        // to apply metadata after this point must not leave a file that the
        // report does not account for.
        self.report.ordinary_files += 1;
        self.report.logical_bytes += bytes;
        preserve_permissions_on_path(&source_metadata.permissions(), destination).map_err(
            |error| {
                at(
                    CopyErrorCategory::MetadataFailed,
                    format!("permissions could not be applied: {error}"),
                )
            },
        )?;
        Ok(())
    }

    /// Read one source directory into a sorted frame.
    fn open_frame(
        &mut self,
        relative: PathBuf,
        permissions: Option<Permissions>,
    ) -> Result<Frame, Failure> {
        let source = self.request.source.join(&relative);
        let unreadable = |error: io::Error| {
            (
                relative.clone(),
                CopyErrorCategory::SourceUnreadable,
                error.to_string(),
            )
        };
        let mut names = Vec::new();
        for entry in fs::read_dir(&source).map_err(unreadable)? {
            names.push(entry.map_err(unreadable)?.file_name());
        }
        // Deterministic order: the same tree copies its entries in the same
        // sequence on every run, so a cancellation or failure is reproducible.
        names.sort();
        Ok(Frame {
            relative,
            names: names.into_iter(),
            permissions,
        })
    }

    /// Apply the source directory's permissions once its subtree is written.
    fn finish_directory(&mut self, frame: &Frame) -> Result<(), Failure> {
        let Some(permissions) = frame.permissions.clone() else {
            return Ok(());
        };
        let destination = self.request.destination.join(&frame.relative);
        preserve_directory_permissions(&permissions, &destination).map_err(|error| {
            (
                frame.relative.clone(),
                CopyErrorCategory::MetadataFailed,
                format!("directory permissions could not be applied: {error}"),
            )
        })
    }

    /// A sibling temporary name in the destination directory, so the rename
    /// stays within one filesystem.
    fn temporary_path(&mut self, destination: &Path) -> PathBuf {
        self.temporaries += 1;
        let name = format!(
            ".gwz-refcopy.{}.{}.tmp",
            std::process::id(),
            self.temporaries
        );
        match destination.parent() {
            Some(parent) => parent.join(name),
            None => PathBuf::from(name),
        }
    }
}

/// Copy every byte of `reader` into `writer` through `buffer`.
///
/// Short writes are retried until the chunk is written; a write that makes no
/// progress is a [`CopyErrorCategory::ShortWrite`]. Cancellation is polled
/// between buffered writes and never before the first one, so an entry that
/// starts is either finished or removed.
fn stream_bytes(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    buffer: &mut [u8],
    cancellation: &dyn Cancellation,
) -> Result<u64, (CopyErrorCategory, String)> {
    let mut total = 0u64;
    let mut chunks = 0u64;
    loop {
        let read = match read_chunk(reader, buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => {
                return Err((CopyErrorCategory::SourceUnreadable, error.to_string()));
            }
        };
        if chunks > 0 && cancellation.is_cancelled() {
            return Err((
                CopyErrorCategory::Cancelled,
                format!("cancelled after {total} bytes of this entry"),
            ));
        }
        write_chunk(writer, &buffer[..read])?;
        chunks += 1;
        total += read as u64;
    }
    // The available flush helper, with its error checked. This is a userspace
    // flush: a successful copy report does not claim crash durability.
    writer.flush().map_err(|error| {
        (
            destination_category(&error),
            format!("flush failed: {error}"),
        )
    })?;
    Ok(total)
}

fn read_chunk(reader: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match reader.read(buffer) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

fn write_chunk(writer: &mut dyn Write, chunk: &[u8]) -> Result<(), (CopyErrorCategory, String)> {
    let mut remaining = chunk;
    while !remaining.is_empty() {
        match writer.write(remaining) {
            Ok(0) => {
                return Err((
                    CopyErrorCategory::ShortWrite,
                    format!(
                        "write made no progress with {} of {} bytes remaining",
                        remaining.len(),
                        chunk.len()
                    ),
                ));
            }
            // `min` is defensive: a writer that claims more than it was given
            // must not turn into a slicing panic inside the copier.
            Ok(written) => remaining = &remaining[written.min(remaining.len())..],
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == ErrorKind::WriteZero => {
                return Err((CopyErrorCategory::ShortWrite, error.to_string()));
            }
            Err(error) => {
                return Err((destination_category(&error), error.to_string()));
            }
        }
    }
    Ok(())
}

/// Classify a failure of an operation that creates or writes a destination
/// entry. Permission, space and I/O failures are errors, never "unsupported".
fn destination_category(error: &io::Error) -> CopyErrorCategory {
    match error.kind() {
        ErrorKind::WriteZero => CopyErrorCategory::ShortWrite,
        _ => CopyErrorCategory::DestinationUnwritable,
    }
}

fn describe_type(metadata: &fs::Metadata) -> &'static str {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let file_type = metadata.file_type();
        if file_type.is_fifo() {
            return "fifo";
        }
        if file_type.is_socket() {
            return "socket";
        }
        if file_type.is_block_device() {
            return "block device";
        }
        if file_type.is_char_device() {
            return "character device";
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    "not a file, directory or symbolic link"
}

#[cfg(unix)]
fn create_symlink(target: &Path, destination: &Path, _source: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, destination)
}

/// Windows needs the link kind at creation time. The source link is resolved
/// only to learn that kind; a link whose target does not resolve is recreated
/// as a file link, which is what a dangling link on Windows already is.
#[cfg(windows)]
fn create_symlink(target: &Path, destination: &Path, source: &Path) -> io::Result<()> {
    let names_directory = fs::metadata(source)
        .map(|meta| meta.is_dir())
        .unwrap_or(false);
    if names_directory {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    }
}

/// Apply the source file's permissions to the still-temporary handle, so the
/// entry never appears under its final name more permissive than its source.
#[cfg(unix)]
fn preserve_permissions_on_handle(permissions: &Permissions, handle: &File) -> io::Result<()> {
    handle.set_permissions(permissions.clone())
}

#[cfg(not(unix))]
fn preserve_permissions_on_handle(_permissions: &Permissions, _handle: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn preserve_permissions_on_path(_permissions: &Permissions, _path: &Path) -> io::Result<()> {
    Ok(())
}

/// Off unix, permission preservation is the read-only attribute, applied after
/// the rename: a read-only file is harder to rename than to create.
#[cfg(not(unix))]
fn preserve_permissions_on_path(permissions: &Permissions, path: &Path) -> io::Result<()> {
    if permissions.readonly() {
        fs::set_permissions(path, permissions.clone())
    } else {
        Ok(())
    }
}

/// Give a directory the source's mode as soon as it is created, with owner
/// access forced on so its own children can still be written. Without this a
/// private source directory would be readable at the process umask's mode for
/// as long as its subtree takes to copy; the exact mode lands in
/// [`preserve_directory_permissions`] once the subtree is complete.
#[cfg(unix)]
fn narrow_new_directory(permissions: &Permissions, path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = (permissions.mode() & 0o7777) | 0o700;
    fs::set_permissions(path, Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn narrow_new_directory(_permissions: &Permissions, _path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn preserve_directory_permissions(permissions: &Permissions, path: &Path) -> io::Result<()> {
    fs::set_permissions(path, permissions.clone())
}

/// Off unix a directory has no mode to preserve; the read-only attribute on a
/// directory does not mean "not writable" there.
#[cfg(not(unix))]
fn preserve_directory_permissions(_permissions: &Permissions, _path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
