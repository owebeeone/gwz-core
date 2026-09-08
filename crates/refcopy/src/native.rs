//! Native copy-on-write: one small wrapper per platform, and the
//! classification that decides what a failed attempt meant.
//!
//! Authority: gwz-dev `dev-docs/GwzLocalCloneImplementationArchitecture.md`
//! §3 and `GwzLocalCloneDesign.md` §4.
//!
//! - The candidates are Apple's `clonefile` family, Linux `FICLONE` and the
//!   Windows ReFS block clone, `FSCTL_DUPLICATE_EXTENTS_TO_FILE`. **The
//!   operation's result decides** whether a source/destination pair supports
//!   cloning; [`probe`] is only a hint.
//! - A classified unsupported or cross-device result falls back to ordinary
//!   copying for that file. Permission, space and I/O failures are errors,
//!   never "unsupported": not every `EINVAL`, bad descriptor, `ENOSPC` or
//!   `EIO` means a missing capability, so each call is classified from its
//!   own documented error list ([`classify::clonefile`],
//!   [`classify::ficlone`], [`classify::duplicate_extents`]) rather than from
//!   one shared table.
//! - A wrapper leaves either a complete clone at the temporary name or no
//!   file at all. It resets only the file this call created, so the engine's
//!   fallback never appends to partial data and never keeps a stale tail.
//! - Cloning shares physical blocks; it never creates a hardlink. Both sides
//!   stay independently writable, which `crate::tests` asserts by mutating
//!   one side and by reading back the link count.
//!
//! Policy — when to attempt, what to warn, what to count — lives in the
//! engine (`crate::ordinary`) and in [`Plan`]; the wrappers below only make
//! the call and classify its errno.

// Ordinary file I/O in a filesystem copier: this crate is outside gwz-core's
// merge-writer boundary (gwz-core/clippy.toml), whose disallowed writers exist
// to route *merge artifact* mutation through checked entries.
#![allow(clippy::disallowed_methods)]

use std::fs::File;
use std::path::Path;

use gwz_copy_contract::{CopyErrorCategory, CopyMode, CopyRequest};

use crate::{NativeCapability, NativeMechanism};

// The four platform cases. `apple` covers macOS and its siblings; `linux`
// covers Linux except SPARC, where the FICLONE ioctl does not exist;
// `windows` covers every Windows target, where the volume rather than the
// build decides; every other target gets the documented `unsupported` stub.
#[cfg(target_vendor = "apple")]
use apple as platform;
#[cfg(all(
    target_os = "linux",
    not(any(target_arch = "sparc", target_arch = "sparc64"))
))]
use linux as platform;
// `self::` because `windows` is a crate name in the wider ecosystem as well
// as this module's name, and a bare `use windows` would become ambiguous the
// day one of them enters the extern prelude.
#[cfg(windows)]
use self::windows as platform;
#[cfg(not(any(
    target_vendor = "apple",
    windows,
    all(
        target_os = "linux",
        not(any(target_arch = "sparc", target_arch = "sparc64"))
    )
)))]
use unsupported as platform;

/// The mechanism compiled into this build. `None` means no native call
/// exists here, and every report says so once.
pub(crate) const MECHANISM: NativeMechanism = platform::MECHANISM;

/// The copy-wide `NativeUnavailable` details. Static text: one copy names
/// its reason once, whatever its size.
const NO_MECHANISM: &str = "no platform copy-on-write mechanism is compiled into this build for \
                            this target; every regular file was copied by ordinary read/write";
const DIFFERENT_DEVICES: &str = "the source and the destination are on different devices, which no \
                                 copy-on-write mechanism can clone across; every regular file was \
                                 copied by ordinary read/write";

/// What one native attempt did.
///
/// All three outcomes exist on every target, because the engine handles all
/// three on every target; a target with no mechanism simply never produces
/// `Cloned` or `Failed`, which is what the `dead_code` allowance covers.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(
        target_vendor = "apple",
        windows,
        all(
            target_os = "linux",
            not(any(target_arch = "sparc", target_arch = "sparc64"))
        )
    )),
    allow(dead_code)
)]
pub(crate) enum Outcome {
    /// The mechanism cloned the file: the temporary now holds the source's
    /// bytes.
    Cloned,
    /// This source/destination pair does not support the mechanism. The
    /// temporary does not exist; the caller copies that file ordinarily and
    /// notes one `NativeUnsupportedFellBack` warning.
    Unsupported(String),
    /// A real failure — permission, space, I/O, a bad descriptor. The
    /// temporary does not exist; the copy stops with this category.
    Failed(CopyErrorCategory, String),
}

/// Whether a copy attempts a native clone for each regular file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attempt {
    /// Make no native call at all.
    Skip,
    /// Attempt [`MECHANISM`] for every regular file, letting each operation
    /// decide for itself.
    Native,
    /// Test seam: every attempt is classified unsupported, which is how the
    /// fallback path is exercised on a host whose filesystem always clones.
    #[cfg(test)]
    ScriptedUnsupported,
    /// Test seam: every attempt is a real failure of this category.
    #[cfg(test)]
    ScriptedFailure(CopyErrorCategory),
}

impl Attempt {
    /// Whether the engine should call [`Attempt::clone_regular_file`] at all.
    pub(crate) fn is_attempted(self) -> bool {
        self != Self::Skip
    }

    /// Attempt to clone `source` into `temporary`, a sibling name in the
    /// destination directory that does not exist yet.
    ///
    /// On [`Outcome::Cloned`] the temporary holds the file. On every other
    /// outcome the temporary does not exist, so the caller may create it
    /// itself and copy ordinarily.
    pub(crate) fn clone_regular_file(self, source: &File, temporary: &Path) -> Outcome {
        match self {
            // The engine asks `is_attempted` first, so this is only reached
            // if that check is ever dropped; answering "unsupported" keeps
            // the copy correct rather than claiming a clone that never ran.
            Self::Skip => Outcome::Unsupported("no native attempt was made".to_owned()),
            Self::Native => platform::clone_regular_file(source, temporary),
            #[cfg(test)]
            Self::ScriptedUnsupported => {
                Outcome::Unsupported("scripted: this pair does not support cloning".to_owned())
            }
            #[cfg(test)]
            Self::ScriptedFailure(category) => {
                Outcome::Failed(category, "scripted native failure".to_owned())
            }
        }
    }
}

/// One copy's native decision, made once before traversal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) attempt: Attempt,
    /// Why an `Auto` copy will make no native attempt at all — the detail of
    /// its one copy-wide `NativeUnavailable` warning. `None` when attempts
    /// are made, and when the request asked for `OrdinaryOnly`, which was
    /// never promised a native path.
    pub(crate) unavailable: Option<&'static str>,
}

impl Plan {
    /// The plan for `request`: attempt natively only in `Auto` mode, with a
    /// mechanism compiled in, and with the probe not already ruling the pair
    /// out.
    pub(crate) fn for_request(request: &CopyRequest) -> Self {
        if request.mode == CopyMode::OrdinaryOnly {
            return Self {
                attempt: Attempt::Skip,
                unavailable: None,
            };
        }
        if MECHANISM == NativeMechanism::None {
            return Self {
                attempt: Attempt::Skip,
                unavailable: Some(NO_MECHANISM),
            };
        }
        match probe(&request.source, &request.destination) {
            NativeCapability::Unavailable => Self {
                attempt: Attempt::Skip,
                unavailable: Some(DIFFERENT_DEVICES),
            },
            // `Unknown` is the ordinary answer for a pair that could clone:
            // the attempt itself decides.
            NativeCapability::Unknown | NativeCapability::Available => Self {
                attempt: Attempt::Native,
                unavailable: None,
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn scripted(attempt: Attempt) -> Self {
        Self {
            attempt,
            unavailable: None,
        }
    }
}

/// Hint whether `source` and `destination` may share a native copy-on-write
/// path. Never authoritative: it answers [`NativeCapability::Unavailable`]
/// only for the one condition that rules cloning out without trying it —
/// no mechanism, or two different devices — and [`NativeCapability::Unknown`]
/// otherwise, because whether the filesystem supports cloning is decided by
/// the operation.
///
/// The destination usually does not exist yet; the device it will be created
/// on is its nearest existing ancestor's.
pub(crate) fn probe(source: &Path, destination: &Path) -> NativeCapability {
    if MECHANISM == NativeMechanism::None {
        return NativeCapability::Unavailable;
    }
    let (Some(source_device), Some(destination_device)) =
        (device_of(source), device_of_nearest_existing(destination))
    else {
        // Nothing could be examined, so nothing is ruled out. The per-file
        // attempts still answer for themselves.
        return NativeCapability::Unknown;
    };
    if source_device == destination_device {
        // Necessary, not sufficient: the filesystem may still refuse.
        NativeCapability::Unknown
    } else {
        NativeCapability::Unavailable
    }
}

fn device_of_nearest_existing(path: &Path) -> Option<u64> {
    path.ancestors().find_map(device_of)
}

#[cfg(unix)]
fn device_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|metadata| metadata.dev())
}

/// The Windows device is the volume serial number, which is what
/// `FSCTL_DUPLICATE_EXTENTS_TO_FILE` means by "the same volume". Read only for
/// a path that exists, so this answers the same question as the unix arm: a
/// destination that does not exist yet is still resolved through
/// [`device_of_nearest_existing`].
#[cfg(windows)]
fn device_of(path: &Path) -> Option<u64> {
    std::fs::symlink_metadata(path).ok()?;
    windows::facts(path).map(|volume| u64::from(volume.serial))
}

/// On the remaining targets there is no mechanism to probe for, so no device
/// is read.
#[cfg(not(any(unix, windows)))]
fn device_of(_path: &Path) -> Option<u64> {
    None
}

/// The failure tables: what each call's own manual page or reference topic
/// says a failure meant.
///
/// Every table is compiled on every host that could have any of them -- the
/// two errno tables on every unix target, the Win32 table everywhere -- and
/// not only on the platform whose call this build can make, so that any
/// platform's classification can be read and unit-tested from one host; only
/// the syscall bindings below are platform-only. That is what the `dead_code`
/// allowance covers -- on Linux nothing calls the `clonefile` table, on Apple
/// targets nothing calls the `FICLONE` one, and off Windows nothing calls the
/// duplicate-extents one.
pub(crate) mod classify {
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
            return Class::Unsupported(
                "source and destination are on different filesystems (EXDEV)",
            );
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
            return Class::Unsupported(
                "the filesystem does not implement the FICLONE ioctl (ENOTTY)",
            );
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
}

/// The arithmetic of a `FSCTL_DUPLICATE_EXTENTS_TO_FILE` request, kept off the
/// platform so it can be read and tested from any host -- the same reason the
/// failure tables above are.
///
/// Block cloning duplicates a *range*, not a file, and the range has to obey
/// two rules the caller must satisfy itself: every offset and length is a
/// multiple of the volume's allocation unit, and one call moves at most
/// [`block_clone::MAX_DUPLICATE_BYTES`]. [`block_clone::next_range`] turns a
/// file length and a cluster size into the sequence of requests that obeys
/// both.
pub(crate) mod block_clone {
    #![allow(dead_code)]

    /// Most bytes one `FSCTL_DUPLICATE_EXTENTS_TO_FILE` may duplicate. Its
    /// reference topic caps a single request at 4 GiB; 4 GiB is a whole number
    /// of clusters for every cluster size a volume can be formatted with (all
    /// are powers of two no larger than it), so chunking here never breaks the
    /// alignment rule.
    pub(crate) const MAX_DUPLICATE_BYTES: u64 = 4 << 30;

    /// One duplicate-extents request.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct Range {
        /// Source and target file offset. Always cluster-aligned.
        pub(crate) offset: u64,
        /// `ByteCount`: cluster-aligned, and for the final range **rounded
        /// up** past the file's length.
        pub(crate) count: u64,
        /// Bytes of the file this request accounts for -- `count` except in
        /// the final range, where it is the unrounded remainder.
        pub(crate) advance: u64,
    }

    /// The request that continues a file of `length` bytes whose first
    /// `offset` bytes are already duplicated, or `None` when there is nothing
    /// left. An empty file yields nothing at all: it has no extents.
    pub(crate) fn next_range(offset: u64, length: u64, cluster_bytes: u64) -> Option<Range> {
        if offset >= length {
            return None;
        }
        let advance = (length - offset).min(MAX_DUPLICATE_BYTES);
        Some(Range {
            offset,
            count: round_up_to_cluster(advance, cluster_bytes),
            advance,
        })
    }

    /// Round `bytes` up to a whole number of `cluster_bytes`.
    ///
    /// A file's length is almost never a multiple of the allocation unit, and
    /// the call refuses a `ByteCount` that is not. Rounding **up** rather than
    /// down is the documented pattern and is what makes the last cluster of
    /// the file arrive: the extra bytes lie inside the cluster the source has
    /// already had allocated to it, and inside the one the pre-sized
    /// destination has too, so the request stays within both allocations even
    /// though it runs past the valid data length. Rounding down would silently
    /// drop the tail.
    pub(crate) fn round_up_to_cluster(bytes: u64, cluster_bytes: u64) -> u64 {
        // Not a volume geometry; answering `bytes` keeps this total, and the
        // wrapper has already declined a volume that describes itself so.
        if cluster_bytes == 0 {
            return bytes;
        }
        match bytes % cluster_bytes {
            0 => bytes,
            // Every call site bounds `bytes` by `MAX_DUPLICATE_BYTES`, so this
            // cannot overflow; saturating keeps the function total for a
            // caller that ignores the bound.
            remainder => bytes.saturating_add(cluster_bytes - remainder),
        }
    }
}

#[cfg(target_vendor = "apple")]
mod apple {
    //! `fclonefileat(source, destination_directory, name, 0)`.
    //!
    //! `clonefile` creates its destination and fails with `EEXIST` if the
    //! name is taken, which suits the engine exactly: it clones into the
    //! sibling temporary name it would have written ordinarily and renames
    //! afterwards, so an interrupted entry never appears under its final
    //! name.
    //!
    //! The clone carries the source's mode, timestamps, ACLs and extended
    //! attributes. The engine still applies the source's permission bits
    //! itself, so a native copy and an ordinary copy are observably the same
    //! copy.

    use std::fs::{self, File};
    use std::path::Path;

    use gwz_copy_contract::CopyErrorCategory;
    use rustix::fs::{CloneFlags, fclonefileat};

    use super::Outcome;
    use crate::NativeMechanism;

    pub(super) const MECHANISM: NativeMechanism = NativeMechanism::AppleClonefile;

    pub(super) fn clone_regular_file(source: &File, temporary: &Path) -> Outcome {
        let (Some(parent), Some(name)) = (temporary.parent(), temporary.file_name()) else {
            return Outcome::Failed(
                CopyErrorCategory::DestinationUnwritable,
                format!("{} has no directory to be created in", temporary.display()),
            );
        };
        // The directory handle lives only for this call, so the copier still
        // holds at most one source and one destination-side handle at a time.
        let directory = match File::open(parent) {
            Ok(directory) => directory,
            Err(error) => {
                return Outcome::Failed(
                    CopyErrorCategory::DestinationUnwritable,
                    format!("the destination directory could not be opened: {error}"),
                );
            }
        };
        // No flags: `CLONE_NOFOLLOW` is about a source *path*, and this call
        // takes an already-open source; `CLONE_NOOWNERCOPY` only matters to
        // a superuser.
        match fclonefileat(source, &directory, name, CloneFlags::empty()) {
            Ok(()) => Outcome::Cloned,
            Err(errno) => {
                // A failed clone may still have created the destination.
                // Remove it before the engine falls back — only ever this
                // temporary, never an entry already under its final name.
                let _ = fs::remove_file(temporary);
                super::classify::describe(super::classify::clonefile(errno), errno, "clonefile")
            }
        }
    }
}

#[cfg(all(
    target_os = "linux",
    not(any(target_arch = "sparc", target_arch = "sparc64"))
))]
mod linux {
    //! `ioctl(destination, FICLONE, source)`.
    //!
    //! FICLONE clones into an *open* destination, so this wrapper creates the
    //! temporary itself and removes it again unless the clone succeeded. The
    //! engine therefore finds either a complete clone or no file at all, and
    //! never a stale tail to append to.

    use std::fs::{self, File, OpenOptions};
    use std::path::Path;

    use gwz_copy_contract::CopyErrorCategory;
    use rustix::fs::ioctl_ficlone;

    use super::Outcome;
    use crate::NativeMechanism;

    pub(super) const MECHANISM: NativeMechanism = NativeMechanism::LinuxFiclone;

    pub(super) fn clone_regular_file(source: &File, temporary: &Path) -> Outcome {
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
        let outcome = match ioctl_ficlone(&destination, source) {
            Ok(()) => Outcome::Cloned,
            Err(errno) => {
                super::classify::describe(super::classify::ficlone(errno), errno, "FICLONE")
            }
        };
        // Close before the engine renames or recreates the name.
        drop(destination);
        if outcome != Outcome::Cloned {
            // Reset the file this call created, so the fallback starts from
            // nothing rather than appending to a partial attempt.
            let _ = fs::remove_file(temporary);
        }
        outcome
    }
}

#[cfg(windows)]
mod windows {
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
    //!   rounded up; see [`super::block_clone`] for why that is the documented
    //!   pattern rather than a shortcut.
    //! - A file larger than
    //!   [`MAX_DUPLICATE_BYTES`](super::block_clone::MAX_DUPLICATE_BYTES) is
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

    use super::block_clone::next_range;
    use super::{Outcome, classify};
    use crate::NativeMechanism;

    pub(super) const MECHANISM: NativeMechanism = NativeMechanism::WindowsBlockClone;

    /// UTF-16 units reserved for a `GetVolumePathNameW` answer. A mount point
    /// is usually `X:\`, but a volume can be mounted on a directory path, so
    /// this is generous; a path that still does not fit answers `None` and the
    /// copy falls back rather than guessing a geometry.
    const VOLUME_PATH_UNITS: usize = 1024;

    /// What the destination volume says about itself: enough to build a legal
    /// duplicate-extents request, and whether it could serve one at all.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct VolumeFacts {
        /// The volume serial number: this platform's device identity, and what
        /// the call means by "the same volume".
        pub(super) serial: u32,
        /// Allocation unit in bytes. Every offset and byte count in a request
        /// is a multiple of it.
        pub(super) cluster_bytes: u64,
        /// Whether the filesystem advertises sharing logical clusters between
        /// files. `false` rules the call out; `true` promises nothing.
        pub(super) block_cloning: bool,
    }

    pub(super) fn clone_regular_file(source: &File, temporary: &Path) -> Outcome {
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
    pub(super) fn facts(path: &Path) -> Option<VolumeFacts> {
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

#[cfg(not(any(
    target_vendor = "apple",
    windows,
    all(
        target_os = "linux",
        not(any(target_arch = "sparc", target_arch = "sparc64"))
    )
)))]
mod unsupported {
    //! No native mechanism for this target: every copy is ordinary and says
    //! so once, through the copy-wide `NativeUnavailable` warning.
    //!
    //! What lands here is a target with no copy-on-write call this crate
    //! binds -- the BSDs, Solaris, SPARC Linux -- not a filesystem that cannot
    //! clone. A filesystem is never ruled out in advance: on the three
    //! platforms with a mechanism the operation decides, per file.

    use std::fs::File;
    use std::path::Path;

    use super::Outcome;
    use crate::NativeMechanism;

    pub(super) const MECHANISM: NativeMechanism = NativeMechanism::None;

    /// Never called: [`Plan::for_request`](super::Plan::for_request) makes no
    /// attempt when [`MECHANISM`] is `None`. It exists so the platform seam
    /// has the same shape on every target.
    pub(super) fn clone_regular_file(_source: &File, _temporary: &Path) -> Outcome {
        Outcome::Unsupported(
            "no native copy-on-write mechanism is compiled in for this target".to_owned(),
        )
    }
}

#[cfg(test)]
mod tests;
