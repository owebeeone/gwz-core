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

use crate::NativeMechanism;

mod apple;
pub(crate) mod attempt;
pub(crate) mod block_clone;
pub(crate) mod classify;
mod linux;
mod unsupported;
mod windows;

// The four platform cases. `apple` covers macOS and its siblings; `linux`
// covers Linux except SPARC, where the FICLONE ioctl does not exist;
// `windows` covers every Windows target, where the volume rather than the
// build decides; every other target gets the documented `unsupported` stub.
// Each arm's body is a `#[cfg]`-gated braced module in its own file, so the
// condition can never drift onto a neighbouring declaration.
#[cfg(windows)]
use self::windows::imp as platform;
#[cfg(target_vendor = "apple")]
use apple::imp as platform;
#[cfg(all(
    target_os = "linux",
    not(any(target_arch = "sparc", target_arch = "sparc64"))
))]
use linux::imp as platform;
#[cfg(not(any(
    target_vendor = "apple",
    windows,
    all(
        target_os = "linux",
        not(any(target_arch = "sparc", target_arch = "sparc64"))
    )
)))]
use unsupported::imp as platform;

pub(crate) use attempt::{Outcome, Plan, probe};

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

#[cfg(test)]
mod tests;
