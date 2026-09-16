//! One native attempt and the per-copy plan around it: what a wrapper can
//! report, how a report is folded into the copy-wide answer, and the
//! device probe that decides whether a native call is worth making.

use std::fs::File;
use std::path::Path;

use gwz_copy_contract::{CopyErrorCategory, CopyMode, CopyRequest};

use crate::native::{DIFFERENT_DEVICES, MECHANISM, NO_MECHANISM, platform};
use crate::{NativeCapability, NativeMechanism};

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

pub(crate) fn device_of_nearest_existing(path: &Path) -> Option<u64> {
    path.ancestors().find_map(device_of)
}

#[cfg(unix)]
pub(crate) fn device_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|metadata| metadata.dev())
}

/// The Windows device is the volume serial number, which is what
/// `FSCTL_DUPLICATE_EXTENTS_TO_FILE` means by "the same volume". Read only for
/// a path that exists, so this answers the same question as the unix arm: a
/// destination that does not exist yet is still resolved through
/// [`device_of_nearest_existing`].
#[cfg(windows)]
pub(crate) fn device_of(path: &Path) -> Option<u64> {
    std::fs::symlink_metadata(path).ok()?;
    super::windows::imp::facts(path).map(|volume| u64::from(volume.serial))
}

/// On the remaining targets there is no mechanism to probe for, so no device
/// is read.
#[cfg(not(any(unix, windows)))]
pub(crate) fn device_of(_path: &Path) -> Option<u64> {
    None
}
