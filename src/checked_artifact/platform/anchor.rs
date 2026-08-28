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
//! # The roaming arm's two names (R2-E Phase E2, DECISION B-3)
//!
//! [`round_trip_supplied`] serves a directory that may retain no permanent
//! anchor. It mints **no vocabulary of its own**: the base name is the caller's
//! schedule-derived reserved leaf, and the outbound name is that leaf under the
//! same `ROUNDTRIP_SUFFIX` the table above already froze — both derivations now
//! live in `platform.rs`, because the residue has to be classifiable on every
//! platform and this module is `cfg(any(windows, test))`.
//!
//! ## What a crash inside the round trip leaves, and who returns it
//!
//! *Corrected at the E2 review's [P2-1]; the first landing of this paragraph
//! claimed a recovery its callers could not reach, and the claim had a
//! behavioural tail.* A crash between the two renames leaves the alias under
//! `<reserved leaf>.roundtrip` with the reserved leaf empty. That window is
//! **converged, and nothing persists** — but the converging caller is
//! [`super::prepare_roaming_target`], **not** this function.
//!
//! The distinction is the whole finding. A drive branching on the reserved leaf
//! alone cannot see the outbound name, so it answered "absent", created a second
//! object over the empty leaf, and only then called the barrier — which found
//! both names resident and refused. One attempt was lost, and the outbound name
//! was then left **permanently**, because the following attempt settled the
//! ordinal and a settled ordinal is never barriered again. The cure is that the
//! entry decision is now `prepare_roaming_target`'s: it owns both names, returns
//! the outbound object before it answers, and hands its caller the ordinary
//! resident state. `round_trip_supplied` calls it too, so a direct caller that
//! did not prepare is equally safe.
//!
//! Stated as states, since three of the four are crash windows — the full table
//! is at [`super::prepare_roaming_target`]:
//!
//! * **reserved leaf resident, outbound absent** — the settled between-barriers
//!   state. Nothing to do.
//! * **reserved leaf absent, outbound resident** — the mid-round-trip crash.
//!   **Converges**: the object is returned to its reserved leaf by a rename,
//!   never a removal, and nothing is left behind. Driven on both target variants
//!   by `a_mid_round_trip_roaming_residue_converges_*`, which builds the state on
//!   disk because no fault key exists inside the round trip.
//! * **both names resident** — unreachable on this tree, because the entry
//!   decision runs before anything is created. Reachable on a tree a
//!   pre-remediation binary wrote. **Not refused**: the ordinal settles through
//!   the stranded entry and the outbound object is left as a tolerated orphan,
//!   because refusing would be a permanent typed refusal on a reachable state
//!   whose only convergence is a removal — the wedge class E16's standard
//!   forbids. Driven by `a_legacy_both_names_tree_settles_with_a_tolerated_orphan_*`.
//! * **neither resident** — the caller creates the alias.
//!
//! So exactly one thing persists, and only from the past: a `<reserved
//! leaf>.roundtrip` written by a **pre-remediation** binary on Windows, on an
//! ordinal that has since settled. That is the legacy-orphan disposition this
//! module already carries for the legacy nonce: bounded by past crashes, unable
//! to grow because nothing on this tree produces it, refusing nothing and
//! blocking nothing — it parses as no scheduled action slot, and every predicate
//! that reads an action directory either ignores it or counts it as one more
//! child of an already-non-exact directory.
//!
//! Within this arm itself, `round_trip_supplied` still refuses a both-names
//! state, and that refusal is correct rather than vestigial: reaching it means a
//! foreign object appeared *after* the entry decision converged, which is an
//! ambiguity this arm must not guess at. It mutates nothing, and the next
//! drive's entry decision resolves it to the tolerated-residue shape above.
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
use super::{
    LeafPublicationSourceV1, error, io_error, leaf_is_resident, prepare_roaming_target,
    publish_verified_leaf_no_replace, roundtrip_name, verify_leaf_bytes,
};
use crate::model::{ErrorCode, ModelResult};

pub(super) const ANCHOR_BYTES: &[u8] = b"GWZ-CHECKED-ARTIFACT-DURABILITY-ANCHOR-V1\n";

const ANCHOR_PREFIX: &str = ".ca1-durability-anchor-";
// `ROUNDTRIP_SUFFIX` and `roundtrip_name` moved to `platform.rs` at R2-E Phase
// E2's remediation: the roaming arm's residue has to be classifiable on every
// platform, and this module is `cfg(any(windows, test))`. One suffix, one
// derivation, now shared by both round trips.

/// The one staging name. Deterministic by contract: this constant is what
/// retires the legacy nonce, so a resume finds its own predecessor instead of
/// allocating beside it.
const SCRATCH_NAME: &str = ".ca1-anchor-scratch-v1";

/// The retirement destination family that replaces the legacy `remove_file`.
///
/// Ordinal-indexed rather than singular, because the state it reconciles
/// **recurs**: a foreign hard link can strand an alias again after an earlier
/// retirement already landed. A single destination would refuse the second one
/// for ever and brick P5 for all seven `AnchoredPrivateArea` callers, which is
/// the wedge the Step-4.2 review's [P2-2] rejected.
///
/// The ordinal is read off the durable state, never allocated: [`survey`]
/// collects the retired ordinals actually resident and the retirement takes the
/// **smallest free** one. Smallest-free rather than count or max+1 because both
/// of those wedge on a gap — with `{0, 2}` resident, "count" recomputes 2 for
/// ever against a no-replace publication that can never succeed. Smallest-free
/// has no such fixed point: it is by construction not a resident name, and if a
/// racing drive takes it first the no-replace publication refuses and the next
/// survey observes the new state and picks again.
const RETIRED_PREFIX: &str = ".ca1-anchor-retired-";

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
        /// The smallest retirement ordinal not resident in the observed state.
        retirement_ordinal: u32,
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
            retirement_ordinal,
        } => {
            verify(dir, &final_name, code, label)?;
            let identity = verify(dir, &alias, code, label)?;
            let retired = OsString::from(retired_name(retirement_ordinal));
            fault(
                CheckedArtifactFault::BeforeAnchorAliasRetirement,
                code,
                label,
            )?;
            publish(dir, &alias, &retired, &identity, code, label)?;
            fault(
                CheckedArtifactFault::AfterAnchorAliasRetirement,
                code,
                label,
            )?;
            verify(dir, &retired, code, label)?;
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

/// R2-E Phase E2, DECISION B-3 — P5's `RoamingAnchoredTarget` arm.
///
/// The directory this serves may retain no permanent anchor of its own, so it
/// is **lent** one: the caller has already created a fresh object carrying
/// `bytes` under the schedule-reserved `alias`, and this rounds that supplied
/// object out of the directory and back, so the metadata transaction orders
/// everything before it exactly as `round_trip` does for a resident anchor.
/// Nothing is surveyed for and nothing permanent is established, which is the
/// whole difference from the `AnchoredPrivateArea` arm.
///
/// **The window a crash can leave is converged, not refused.** A drive that
/// stops between the two renames leaves the alias out under its outbound name;
/// the next barrier returns it before taking its own trip. That is the same
/// `AnchorState::NeedsReturn` discipline the resident protocol uses, reduced to
/// two names because this arm surveys nothing: the alias belongs at its
/// reserved leaf between barriers, so an outbound name resident here is this
/// protocol's own residue.
///
/// Portable code with a Windows-only production caller, for the reason stated
/// in the module header: it is what lets every platform execute this arm's
/// interruption/restart rows rather than leaving them to a `cfg(windows)` arm
/// no local gate can drive.
pub(super) fn round_trip_supplied(
    dir: &Dir,
    alias: &OsStr,
    bytes: &[u8],
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    let roundtrip = roundtrip_name(alias);
    // The caller reached here through `prepare_roaming_target`, which already
    // returned any outbound residue, so this converge step is the direct
    // caller's guarantee rather than the drive's: it keeps the arm safe for a
    // caller that did not prepare, which is how `round_trip` itself opens.
    prepare_roaming_target(dir, alias, bytes, code, label)?;
    if !leaf_is_resident(dir, alias, code, label)?
        || leaf_is_resident(dir, &roundtrip, code, label)?
    {
        return Err(error(
            code,
            label,
            "supplied roaming anchor alias is not exactly resident at its reserved leaf",
        ));
    }
    let identity = verify_leaf_bytes(dir, alias, bytes, code, label)?;
    publish_bytes(dir, alias, &roundtrip, &identity, bytes, code, label)?;
    let moved = verify_leaf_bytes(dir, &roundtrip, bytes, code, label)?;
    publish_bytes(dir, &roundtrip, alias, &moved, bytes, code, label)?;
    let returned = verify_leaf_bytes(dir, alias, bytes, code, label)?;
    if moved != returned || identity != returned {
        return Err(error(
            code,
            label,
            "supplied roaming anchor identity changed across its round trip",
        ));
    }
    Ok(())
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
    publish_bytes(
        dir,
        source,
        destination,
        identity,
        ANCHOR_BYTES,
        code,
        label,
    )
}

/// The same publication over an explicitly named content, for the roaming arm,
/// whose lent object carries the catalog's `ROAMING_ANCHOR_BYTES` rather than
/// this module's own `ANCHOR_BYTES`. The constant is not duplicated here: it is
/// supplied by the caller that owns it.
fn publish_bytes(
    dir: &Dir,
    source: &OsStr,
    destination: &OsStr,
    identity: &ObjectIdentity,
    bytes: &[u8],
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    publish_verified_leaf_no_replace(
        dir,
        source,
        dir,
        destination,
        &LeafPublicationSourceV1 { identity, bytes },
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
    let mut retired = Vec::new();
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
        } else if let Some(rendering) = text.strip_prefix(RETIRED_PREFIX) {
            // A retired name whose ordinal is not `retired_name`'s own
            // rendering is foreign, not ours. Parsing alone did not say that:
            // `u32::from_str` also accepts renderings this protocol never
            // writes — zero-padded (`retired-007`) and sign-prefixed
            // (`retired-+7`) — and each was adopted as the ordinal it parsed
            // to, so a foreign name could hold a residency slot the protocol
            // had never retired onto and push the next retirement past it.
            // Re-rendering closes that: an ordinal is ours only if
            // `retired_name` would have produced this exact name. Convergence
            // is untouched either way, because `smallest_free_ordinal` reads
            // residency and not text (`GwzM5-8R2DSettledTuple.md:659-662`,
            // executed at R2-E E6.2).
            let Ok(ordinal) = rendering.parse::<u32>() else {
                return Ok(AnchorState::Invalid);
            };
            if retired_name(ordinal) != *text {
                return Ok(AnchorState::Invalid);
            }
            retired.push(ordinal);
        } else if text.starts_with("ca1-") {
            family_state = true;
        }
    }
    let retirement_ordinal = smallest_free_ordinal(&mut retired);
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
                retirement_ordinal,
            })
        }
        _ => Ok(AnchorState::Invalid),
    }
}

/// The smallest ordinal not present in `observed`.
///
/// Bounded by the pigeonhole principle: with `n` observed ordinals the answer is
/// at most `n`, so the scan is linear in what the directory already holds and
/// needs no cap of its own. A cap would reintroduce the very wedge this design
/// removes — at the cap there would again be no exit — and each retired object
/// costs one *foreign* stranding event interrupted by a crash, so the count is
/// bounded by real occurrences rather than by anything the protocol can inflate.
fn smallest_free_ordinal(observed: &mut Vec<u32>) -> u32 {
    observed.sort_unstable();
    observed.dedup();
    let mut free = 0;
    for ordinal in observed.iter() {
        if *ordinal != free {
            break;
        }
        free += 1;
    }
    free
}

fn retired_name(ordinal: u32) -> String {
    format!("{RETIRED_PREFIX}{ordinal}")
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
