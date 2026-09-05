//! Native copy-on-write: one small wrapper per platform, and the
//! classification that decides what a failed attempt meant.
//!
//! Authority: gwz-dev `dev-docs/GwzLocalCloneImplementationArchitecture.md`
//! §3 and `GwzLocalCloneDesign.md` §4.
//!
//! - The candidates are Apple's `clonefile` family, Linux `FICLONE` and a
//!   Windows block-clone path. **The operation's result decides** whether a
//!   source/destination pair supports cloning; [`probe`] is only a hint.
//! - A classified unsupported or cross-device result falls back to ordinary
//!   copying for that file. Permission, space and I/O failures are errors,
//!   never "unsupported": not every `EINVAL`, bad descriptor, `ENOSPC` or
//!   `EIO` means a missing capability, so each call is classified from its
//!   own documented error list ([`classify::clonefile`],
//!   [`classify::ficlone`]) rather than from one shared errno table.
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

// The three platform cases. `apple` covers macOS and its siblings; `linux`
// covers Linux except SPARC, where the FICLONE ioctl does not exist; every
// other target, Windows included, gets the documented `unsupported` stub.
#[cfg(target_vendor = "apple")]
use apple as platform;
#[cfg(all(
    target_os = "linux",
    not(any(target_arch = "sparc", target_arch = "sparc64"))
))]
use linux as platform;
#[cfg(not(any(
    target_vendor = "apple",
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Off unix there is no mechanism to probe for, so no device is read.
#[cfg(not(unix))]
fn device_of(_path: &Path) -> Option<u64> {
    None
}

/// The errno tables: what each call's own manual page says a failure meant.
///
/// Both tables are compiled on every unix target, not only the one whose
/// syscall this build can make, so that either platform's classification can
/// be read and unit-tested from one host; only the syscall bindings below
/// are platform-only. That is what the `dead_code` allowance covers -- on
/// Linux nothing calls the `clonefile` table, and on Apple targets nothing
/// calls the `FICLONE` one.
#[cfg(unix)]
pub(crate) mod classify {
    #![allow(dead_code)]

    use gwz_copy_contract::CopyErrorCategory;
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

    /// Turn a classification and its errno into the outcome the engine acts
    /// on.
    pub(crate) fn describe(class: Class, errno: Errno, call: &str) -> Outcome {
        match class {
            Class::Unsupported(reason) => Outcome::Unsupported(format!(
                "{call} cannot clone this source/destination pair: {reason}: {errno}"
            )),
            Class::Failed(category) => Outcome::Failed(category, format!("{call} failed: {errno}")),
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

#[cfg(not(any(
    target_vendor = "apple",
    all(
        target_os = "linux",
        not(any(target_arch = "sparc", target_arch = "sparc64"))
    )
)))]
mod unsupported {
    //! No native mechanism for this target: every copy is ordinary and says
    //! so once, through the copy-wide `NativeUnavailable` warning.
    //!
    //! **Windows** lands here deliberately. ReFS block cloning is
    //! `DeviceIoControl(FSCTL_DUPLICATE_EXTENTS_TO_FILE)` over a
    //! `DUPLICATE_EXTENTS_DATA`, and implementing it here would need three
    //! things this lane does not have: the `Win32_System_IO` feature of
    //! `windows-sys` for `DeviceIoControl` (this crate's manifest enables
    //! only `Win32_Foundation` and `Win32_Storage_FileSystem`, and the
    //! manifest is not this lane's to change); an `unsafe` block, because
    //! `windows-sys` is a raw binding with no safe wrapper, which would
    //! break this crate's `#![forbid(unsafe_code)]`; and an ReFS volume to
    //! test on, without which the cluster-alignment and `SetEndOfFile`
    //! pre-sizing rules that block cloning requires would ship unverified.
    //! An unverified clone path is worse than an honest ordinary copy, so
    //! Windows reports `NativeMechanism::None` until those three exist.

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
