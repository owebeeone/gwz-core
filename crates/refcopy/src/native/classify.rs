//! The failure tables: what each call's own manual page or reference topic
//! says a failure meant.
//!
//! Every table is compiled on every host that could have any of them -- the
//! two errno tables on every unix target, the Win32 table everywhere -- and
//! not only on the platform whose call this build can make, so that any
//! platform's classification can be read and unit-tested from one host; only
//! the syscall bindings below are platform-only. That is what the `dead_code`
//! allowance covers -- on Linux nothing calls the `clonefile` table, on Apple
//! targets nothing calls the `FICLONE` one, and off Windows nothing calls the
//! duplicate-extents one.
#![allow(dead_code)]

use std::fmt::Display;

use gwz_copy_contract::CopyErrorCategory;
#[cfg(unix)]
use rustix::io::Errno;

use super::Outcome;

/// How a failed native attempt is read: the classification the fallback
/// rule turns on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// The pair cannot be cloned by this mechanism. Fall back.
    Unsupported(&'static str),
    /// A real failure. Stop the copy with this category.
    Failed(CopyErrorCategory),
}

/// Apple `clonefile(2)` / `fclonefileat(2)`.
///
/// Unsupported -- fall back to ordinary copying:
///
/// - `ENOTSUP` / `EOPNOTSUPP`: "the underlying filesystem does not
///   support this call" (HFS+, exFAT, an SMB or NFS mount).
/// - `EXDEV`: source and destination are on different filesystems.
///
/// Everything else is a real failure. In particular `EINVAL`, which this
/// call documents as an invalid `flags` argument -- a bug in this
/// wrapper, not a missing filesystem capability. `EEXIST`, `EACCES`,
/// `EPERM`, `EROFS`, `ENOSPC` and `EDQUOT` are destination failures;
/// `EIO` and anything unrecognised are I/O failures.
#[cfg(unix)]
pub(crate) fn clonefile(errno: Errno) -> Class {
    // Compared with `==` rather than matched: `ENOTSUP` and `EOPNOTSUPP`
    // are the same value on some targets, and a `match` over two equal
    // constants is an unreachable pattern there.
    if errno == Errno::NOTSUP || errno == Errno::OPNOTSUPP {
        return Class::Unsupported("the filesystem does not support clonefile (ENOTSUP)");
    }
    if errno == Errno::XDEV {
        return Class::Unsupported("source and destination are on different filesystems (EXDEV)");
    }
    if destination_errno(errno) {
        return Class::Failed(CopyErrorCategory::DestinationUnwritable);
    }
    Class::Failed(CopyErrorCategory::Io)
}

/// Linux `ioctl(destination, FICLONE, source)`.
///
/// Unsupported -- fall back to ordinary copying:
///
/// - `EOPNOTSUPP` / `ENOTSUP`: the filesystem cannot reflink one of the
///   descriptors.
/// - `EXDEV`: the files are not on the same mounted filesystem.
/// - `EINVAL`: `ioctl_ficlone(2)` documents this as "the filesystem does
///   not support reflinking the ranges of the given files". Unlike
///   `clonefile`'s `EINVAL`, which means invalid flags, this one *is* a
///   missing capability -- which is why each call is classified from its
///   own manual page instead of from one shared errno table
///   (architecture §3: do not read every `EINVAL` as a missing
///   capability).
/// - `ENOTTY`: what a filesystem that does not implement the ioctl at
///   all returns.
///
/// Everything else is a real failure: `EBADF` and `EISDIR` (a bug in
/// this wrapper, which only ever clones open regular files), `ETXTBSY`
/// (a swap file), and the permission, space and I/O errno below.
#[cfg(unix)]
pub(crate) fn ficlone(errno: Errno) -> Class {
    if errno == Errno::NOTSUP || errno == Errno::OPNOTSUPP {
        return Class::Unsupported("the filesystem does not support reflinking (EOPNOTSUPP)");
    }
    if errno == Errno::XDEV {
        return Class::Unsupported("the files are not on the same mounted filesystem (EXDEV)");
    }
    if errno == Errno::INVAL {
        return Class::Unsupported(
            "the filesystem does not support reflinking these files (EINVAL)",
        );
    }
    if errno == Errno::NOTTY {
        return Class::Unsupported("the filesystem does not implement the FICLONE ioctl (ENOTTY)");
    }
    if destination_errno(errno) {
        return Class::Failed(CopyErrorCategory::DestinationUnwritable);
    }
    Class::Failed(CopyErrorCategory::Io)
}

/// The errno both calls read as "the destination could not be created or
/// written": permission, a read-only or full filesystem, an occupied
/// name, a missing or non-directory parent. Never "unsupported" (design
/// §4: "Permission, space, I/O and metadata failures are errors").
#[cfg(unix)]
fn destination_errno(errno: Errno) -> bool {
    errno == Errno::ACCESS
        || errno == Errno::PERM
        || errno == Errno::ROFS
        || errno == Errno::EXIST
        || errno == Errno::NOENT
        || errno == Errno::NOTDIR
        || errno == Errno::NOSPC
        || errno == Errno::DQUOT
}

/// Win32 error codes from `winerror.h`, written out here rather than
/// imported from `windows-sys` so that the table below compiles, and is
/// unit-tested, on any host -- the same reason both errno tables are
/// compiled on every unix target. They are ABI constants and cannot
/// change; `native::tests` checks them against the real bindings when the
/// build is for Windows.
pub(crate) mod win32 {
    pub(crate) const ERROR_INVALID_FUNCTION: u32 = 1;
    pub(crate) const ERROR_FILE_NOT_FOUND: u32 = 2;
    pub(crate) const ERROR_PATH_NOT_FOUND: u32 = 3;
    pub(crate) const ERROR_ACCESS_DENIED: u32 = 5;
    pub(crate) const ERROR_INVALID_HANDLE: u32 = 6;
    pub(crate) const ERROR_NOT_SAME_DEVICE: u32 = 17;
    pub(crate) const ERROR_WRITE_PROTECT: u32 = 19;
    pub(crate) const ERROR_SHARING_VIOLATION: u32 = 32;
    pub(crate) const ERROR_HANDLE_DISK_FULL: u32 = 39;
    pub(crate) const ERROR_NOT_SUPPORTED: u32 = 50;
    pub(crate) const ERROR_FILE_EXISTS: u32 = 80;
    pub(crate) const ERROR_INVALID_PARAMETER: u32 = 87;
    pub(crate) const ERROR_DISK_FULL: u32 = 112;
    pub(crate) const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;
    pub(crate) const ERROR_ALREADY_EXISTS: u32 = 183;
    pub(crate) const ERROR_OFFSET_ALIGNMENT_VIOLATION: u32 = 327;
    pub(crate) const ERROR_BLOCK_TOO_MANY_REFERENCES: u32 = 347;
}

/// Windows `DeviceIoControl(destination, FSCTL_DUPLICATE_EXTENTS_TO_FILE,
/// &DUPLICATE_EXTENTS_DATA)`.
///
/// Unsupported -- fall back to ordinary copying:
///
/// - `ERROR_NOT_SUPPORTED`, `ERROR_INVALID_FUNCTION` and
///   `ERROR_CALL_NOT_IMPLEMENTED`: how a filesystem that does not
///   implement this control code answers it, the Windows counterpart of
///   `ENOTTY`. NTFS and FAT answer here.
/// - `ERROR_NOT_SAME_DEVICE`: the two files are not on one volume.
/// - `ERROR_INVALID_PARAMETER` and `ERROR_OFFSET_ALIGNMENT_VIOLATION`:
///   ReFS's answer to a request it will not serve for *these two files* --
///   a cluster geometry the wrapper modelled wrongly, a sparseness or
///   integrity-stream setting that differs between them, a source range
///   outside the source's allocation. Like `FICLONE`'s `EINVAL` and unlike
///   `clonefile`'s, this call's own reference topic makes it a property of
///   the pair, so it falls back; the reason is carried in the warning, so
///   a wrapper bug is still visible rather than silent.
/// - `ERROR_BLOCK_TOO_MANY_REFERENCES`: the source region is already
///   shared as widely as the filesystem allows and cannot be shared again.
///
/// Everything else is a real failure: `ERROR_ACCESS_DENIED`,
/// `ERROR_WRITE_PROTECT`, `ERROR_SHARING_VIOLATION`, the two disk-full
/// codes and the name and path codes are destination failures;
/// `ERROR_INVALID_HANDLE` (a bug in this wrapper) and anything
/// unrecognised are I/O failures.
pub(crate) fn duplicate_extents(code: u32) -> Class {
    use win32::*;
    match code {
        ERROR_NOT_SUPPORTED | ERROR_INVALID_FUNCTION | ERROR_CALL_NOT_IMPLEMENTED => {
            Class::Unsupported(
                "the filesystem does not implement block cloning (ERROR_NOT_SUPPORTED)",
            )
        }
        ERROR_NOT_SAME_DEVICE => {
            Class::Unsupported("the files are not on the same volume (ERROR_NOT_SAME_DEVICE)")
        }
        ERROR_INVALID_PARAMETER | ERROR_OFFSET_ALIGNMENT_VIOLATION => Class::Unsupported(
            "the filesystem will not duplicate extents between these files: cluster \
             alignment, sparseness or integrity-stream settings (ERROR_INVALID_PARAMETER)",
        ),
        ERROR_BLOCK_TOO_MANY_REFERENCES => Class::Unsupported(
            "the source region is already shared as widely as the filesystem allows \
             (ERROR_BLOCK_TOO_MANY_REFERENCES)",
        ),
        ERROR_ACCESS_DENIED
        | ERROR_WRITE_PROTECT
        | ERROR_SHARING_VIOLATION
        | ERROR_DISK_FULL
        | ERROR_HANDLE_DISK_FULL
        | ERROR_FILE_EXISTS
        | ERROR_ALREADY_EXISTS
        | ERROR_FILE_NOT_FOUND
        | ERROR_PATH_NOT_FOUND => Class::Failed(CopyErrorCategory::DestinationUnwritable),
        _ => Class::Failed(CopyErrorCategory::Io),
    }
}

/// Turn a classification and the error it came from into the outcome the
/// engine acts on. `error` is whatever the platform reports -- an errno on
/// unix, an [`std::io::Error`] carrying the Win32 code on Windows.
pub(crate) fn describe(class: Class, error: impl Display, call: &str) -> Outcome {
    match class {
        Class::Unsupported(reason) => Outcome::Unsupported(format!(
            "{call} cannot clone this source/destination pair: {reason}: {error}"
        )),
        Class::Failed(category) => Outcome::Failed(category, format!("{call} failed: {error}")),
    }
}
