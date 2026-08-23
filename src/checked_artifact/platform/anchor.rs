//! The closed durability-anchor protocol for the checked-artifact private area.
//!
//! R2-D Phase 4 Step 4.2 — `GwzM5-8R2D-Plan.md` §4 Step 4.2 and the interface
//! freeze's §4.3 row **E22** ("legacy Windows durability anchor retirement …
//! P1 + P5 … random scratch … is the R2 stop-clause violation this step
//! removes").
//!
//! # What the anchor is, and why it survives its retirement
//!
//! Windows exposes no portable directory flush, so a private area that must
//! order its own dirents earns that ordering by renaming a **resident** file out
//! of the directory and back: the metadata transaction is the barrier. That is
//! P5's `AnchoredPrivateArea` arm (`GwzM5-8R2DInterfaceFreeze.md` §4.1 row P5),
//! and the E10/E14 activation annotation records why only this class of
//! directory may hold one. Retiring the anchor *concept* would silently weaken
//! Windows durability for every legacy leaf writer, so what this step retires is
//! the legacy anchor's **machinery**, not its guarantee.
//!
//! # What the closed protocol replaces
//!
//! 1. **A random scratch name.** The legacy create arm allocated
//!    `.ca1-anchor-scratch-<16 random bytes>` per attempt. That is the R2 stop
//!    clause's nonce (`GwzM5-8R4bP1P2-RemPlan-4.md` §4 :1089-1092; plan §4 Step
//!    1.1, "retry reuses names/capacity, never a nonce"), and its concrete harm
//!    was unbounded: the legacy survey classified only `.ca1-durability-anchor-`
//!    entries and `ca1-`-prefixed family state, so a crash between the create and
//!    the publication left an orphan that nothing could see, name, or reclaim —
//!    one per crash, for ever. The protocol below stages through exactly one
//!    deterministic name, resumes onto it, and rewrites it in place when a crash
//!    left it short.
//! 2. **A removal on a correctness-critical path.** The legacy alias
//!    reconciliation called `remove_file`. Here that becomes a **durable
//!    retirement** onto a deterministic retired name through the sealed
//!    publication, so the reconciliation leaves evidence instead of a hole, and a
//!    second outstanding retirement is a typed refusal rather than an
//!    accommodation.
//! 3. **Raw relative renames.** Every anchor edge now publishes through
//!    [`publish_verified_leaf_no_replace`](super::publish_verified_leaf_no_replace)
//!    — E22's "P1" — so each one re-verifies the object it is about to move
//!    through the very handle it renames.
//!
//! # The closed name grammar
//!
//! Four names, and no protocol state can produce a fifth:
//!
//! | Name | Role |
//! | --- | --- |
//! | `.ca1-anchor-scratch-v1` | the one staging name, resumed onto rather than reallocated |
//! | `.ca1-durability-anchor-<32hex>` | the resident anchor, addressed by its own durable identity |
//! | `.ca1-durability-anchor-<32hex>.roundtrip` | the barrier's outbound alias |
//! | `.ca1-anchor-retired-v1` | the retirement destination that replaces the removal |
//!
//! # Portability
//!
//! The protocol is portable code with a Windows-only production caller: off
//! Windows `private_barrier` flushes the directory handle directly and never
//! reaches the anchor (`platform.rs`). Keeping the code portable is what lets
//! every platform execute its interruption/restart rows, rather than leaving the
//! whole closed grammar to a `cfg(windows)` arm no local gate can drive.

use std::ffi::{OsStr, OsString};
use std::io::Write;

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use super::super::CheckedArtifactFact;
use super::super::fault::{CheckedArtifactFault, fault};
use super::super::identity::{self, ObjectIdentity};
use super::super::observation::observe_leaf_exact;
use super::{LeafPublicationSourceV1, error, io_error, publish_verified_leaf_no_replace};
use crate::model::{ErrorCode, ModelResult};

pub(super) const ANCHOR_BYTES: &[u8] = b"GWZ-CHECKED-ARTIFACT-DURABILITY-ANCHOR-V1\n";

const ANCHOR_PREFIX: &str = ".ca1-durability-anchor-";
const ROUNDTRIP_SUFFIX: &str = ".roundtrip";

/// The one staging name. Deterministic by contract: this constant is what
/// retires the legacy nonce, so a resume finds its own predecessor instead of
/// allocating beside it.
const SCRATCH_NAME: &str = ".ca1-anchor-scratch-v1";

/// The retirement destination that replaces the legacy `remove_file`.
const RETIRED_NAME: &str = ".ca1-anchor-retired-v1";

#[derive(Debug)]
enum AnchorState {
    Ready {
        final_name: OsString,
        identity: ObjectIdentity,
    },
    NeedsReturn {
        roundtrip: OsString,
        final_name: OsString,
    },
    NeedsRetireAlias {
        alias: OsString,
        final_name: OsString,
        retirement_occupied: bool,
    },
    Missing {
        family_state: bool,
        scratch_present: bool,
    },
    Invalid,
}

/// Establish the anchor, or converge whatever window a previous drive left.
///
/// `create` false keeps the legacy short circuit exactly: an area with no anchor
/// and no family state is left alone, because a reader must not plant one.
pub(super) fn prepare(dir: &Dir, create: bool, code: ErrorCode, label: &str) -> ModelResult<()> {
    match survey(dir, code, label)? {
        AnchorState::Ready { .. } => Ok(()),
        AnchorState::NeedsReturn {
            roundtrip,
            final_name,
        } => {
            let identity = verify(dir, &roundtrip, code, label)?;
            publish(dir, &roundtrip, &final_name, &identity, code, label)?;
            verify(dir, &final_name, code, label).map(|_| ())
        }
        AnchorState::NeedsRetireAlias {
            alias,
            final_name,
            retirement_occupied,
        } => {
            verify(dir, &final_name, code, label)?;
            if retirement_occupied {
                // Closed rather than accommodating: a second outstanding
                // retirement would need a second destination name, and minting
                // one is the nonce this step exists to remove.
                return Err(error(
                    code,
                    label,
                    "durability anchor retirement slot is already occupied",
                ));
            }
            let identity = verify(dir, &alias, code, label)?;
            fault(
                CheckedArtifactFault::BeforeAnchorAliasRetirement,
                code,
                label,
            )?;
            publish(
                dir,
                &alias,
                OsStr::new(RETIRED_NAME),
                &identity,
                code,
                label,
            )?;
            fault(
                CheckedArtifactFault::AfterAnchorAliasRetirement,
                code,
                label,
            )?;
            verify(dir, OsStr::new(RETIRED_NAME), code, label)?;
            round_trip(dir, code, label)
        }
        AnchorState::Missing {
            family_state: false,
            ..
        } if !create => Ok(()),
        AnchorState::Missing {
            family_state: false,
            scratch_present,
        } => establish(dir, scratch_present, code, label),
        AnchorState::Missing {
            family_state: true, ..
        }
        | AnchorState::Invalid => Err(error(
            code,
            label,
            "private durability anchor is missing or ambiguous while family state exists",
        )),
    }
}

/// P5's `AnchoredPrivateArea` arm: rename the resident anchor out of the
/// directory and back, so the dirent transaction orders everything before it.
pub(super) fn round_trip(dir: &Dir, code: ErrorCode, label: &str) -> ModelResult<()> {
    prepare(dir, false, code, label)?;
    let AnchorState::Ready {
        final_name,
        identity,
    } = survey(dir, code, label)?
    else {
        return Err(error(code, label, "private durability anchor is not ready"));
    };
    let roundtrip = roundtrip_name(&final_name);
    fault(CheckedArtifactFault::BeforeAnchorRoundTrip, code, label)?;
    publish(dir, &final_name, &roundtrip, &identity, code, label)?;
    fault(CheckedArtifactFault::AfterAnchorOutboundRename, code, label)?;
    let moved = verify(dir, &roundtrip, code, label)?;
    publish(dir, &roundtrip, &final_name, &moved, code, label)?;
    fault(CheckedArtifactFault::AfterAnchorReturnRename, code, label)?;
    let returned = verify(dir, &final_name, code, label)?;
    if moved != returned {
        return Err(error(code, label, "durability anchor identity changed"));
    }
    fault(CheckedArtifactFault::AfterAnchorReobservation, code, label)
}

/// Stage the anchor through the one deterministic scratch name and publish it
/// onto its identity-derived home.
///
/// `scratch_present` is the resume signal, and it is load-bearing rather than
/// decorative: a fresh drive opens `create_new`, so a racing creator fails the
/// open closed, while a resume opens the *same* name for truncation. A crash
/// before the flush leaves a short scratch, which is write-ahead staging that
/// never committed and is therefore rewritten in place — the house pattern of
/// `execute_owner_prepare_or_rewrite_staging` and `write_or_rewrite_marker`, and
/// the reason no second name is ever needed.
fn establish(dir: &Dir, scratch_present: bool, code: ErrorCode, label: &str) -> ModelResult<()> {
    let scratch = OsStr::new(SCRATCH_NAME);
    fault(CheckedArtifactFault::BeforeAnchorScratchCreate, code, label)?;
    let mut options = OpenOptions::new();
    options.write(true).follow(FollowSymlinks::No);
    if scratch_present {
        options.truncate(true);
    } else {
        options.create_new(true);
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_WRITE_THROUGH);
    }
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = dir
        .open_with(scratch, &options)
        .map_err(|cause| io_error(code, label, cause))?;
    file.write_all(ANCHOR_BYTES)
        .map_err(|cause| io_error(code, label, cause))?;
    fault(CheckedArtifactFault::AfterAnchorScratchWrite, code, label)?;
    file.sync_all()
        .map_err(|cause| io_error(code, label, cause))?;
    fault(CheckedArtifactFault::AfterAnchorScratchFlush, code, label)?;
    let identity = identity::file_identity(&file).map_err(|cause| io_error(code, label, cause))?;
    drop(file);
    let final_name = OsString::from(anchor_name(&identity.name_digest()));
    publish(dir, scratch, &final_name, &identity, code, label)?;
    fault(CheckedArtifactFault::AfterAnchorPublication, code, label)?;
    verify(dir, &final_name, code, label).map(|_| ())
}

/// Every anchor edge is one P1 publication: the object is re-verified — identity
/// and the frozen anchor bytes — through the handle that is then renamed, so a
/// foreign substitution inside the window is refused before the namespace edge
/// rather than moved and rejected afterwards. Every anchor edge is also
/// same-directory, which is why one parameter names the parent.
fn publish(
    dir: &Dir,
    source: &OsStr,
    destination: &OsStr,
    identity: &ObjectIdentity,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    publish_verified_leaf_no_replace(
        dir,
        source,
        dir,
        destination,
        &LeafPublicationSourceV1 {
            identity,
            bytes: ANCHOR_BYTES,
        },
        code,
        label,
    )
}

/// Classify the whole anchor namespace in one bounded pass. Every name the
/// protocol can produce is recognized here — including the staging and retired
/// names the legacy survey could not see, which is what made its orphans
/// unreclaimable.
fn survey(dir: &Dir, code: ErrorCode, label: &str) -> ModelResult<AnchorState> {
    let mut anchors = Vec::new();
    let mut family_state = false;
    let mut scratch_present = false;
    let mut retirement_occupied = false;
    for entry in dir
        .entries()
        .map_err(|cause| io_error(code, label, cause))?
    {
        let entry = entry.map_err(|cause| io_error(code, label, cause))?;
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if text.starts_with(ANCHOR_PREFIX) {
            anchors.push(name);
        } else if text == SCRATCH_NAME {
            scratch_present = true;
        } else if text == RETIRED_NAME {
            retirement_occupied = true;
        } else if text.starts_with("ca1-") {
            family_state = true;
        }
    }
    if anchors.is_empty() {
        return Ok(AnchorState::Missing {
            family_state,
            scratch_present,
        });
    }
    if anchors.len() > 2 {
        return Ok(AnchorState::Invalid);
    }
    let mut final_entry = None;
    let mut roundtrip_entry = None;
    for name in anchors {
        let identity = verify(dir, &name, code, label)?;
        let expected = OsString::from(anchor_name(&identity.name_digest()));
        if name == expected {
            if final_entry.replace((name, identity)).is_some() {
                return Ok(AnchorState::Invalid);
            }
        } else if name == roundtrip_name(&expected) {
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
        (Some((final_name, identity)), None) => Ok(AnchorState::Ready {
            final_name,
            identity,
        }),
        (None, Some((roundtrip, final_name, _))) => Ok(AnchorState::NeedsReturn {
            roundtrip,
            final_name,
        }),
        (Some((final_name, final_identity)), Some((alias, expected, alias_identity)))
            if final_name == expected && final_identity == alias_identity =>
        {
            Ok(AnchorState::NeedsRetireAlias {
                alias,
                final_name,
                retirement_occupied,
            })
        }
        _ => Ok(AnchorState::Invalid),
    }
}

fn roundtrip_name(final_name: &OsStr) -> OsString {
    let mut roundtrip = final_name.to_os_string();
    roundtrip.push(ROUNDTRIP_SUFFIX);
    roundtrip
}

fn verify(dir: &Dir, name: &OsStr, code: ErrorCode, label: &str) -> ModelResult<ObjectIdentity> {
    let observed = observe_leaf_exact(dir, name, code, label)?;
    if observed.fact != CheckedArtifactFact::Bytes(ANCHOR_BYTES.to_vec()) {
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

fn anchor_name(identity: &[u8; 16]) -> String {
    format!("{ANCHOR_PREFIX}{}", super::hex(identity))
}

#[cfg(test)]
mod tests;
