//! R2-D Phase 4 Step 4.2 — the closed durability-anchor protocol (freeze §4.3
//! row E22).
//!
//! The protocol is portable code with a Windows-only production caller, so these
//! rows execute on every platform rather than only inside a Windows matrix run.
//! What they prove is exactly what the retirement claims: one deterministic
//! staging name instead of a nonce, a durable retirement instead of a removal, a
//! closed name grammar no window can escape, and source association at every
//! edge.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use cap_std::fs::Dir;

use super::super::super::fault::{
    CheckedArtifactFault, fail_next_checked_artifact_at, run_next_checked_artifact_at,
};
use super::super::roundtrip_name;
use super::{
    ANCHOR_BYTES, ANCHOR_PREFIX, RETIRED_PREFIX, SCRATCH_NAME, anchor_name, prepare, retired_name,
    round_trip, round_trip_supplied,
};
use crate::model::ErrorCode;

const CODE: ErrorCode = ErrorCode::MergeRecoveryRequired;
const LABEL: &str = "anchor protocol test";

/// Every boundary the closed protocol announces, in protocol order. The four
/// that pre-date Step 4.2 were `cfg(windows)`; all ten are portable now, and all
/// ten are driven by the matrix below — the Step-4.2 review's [P2-1] was that the
/// last two were announced, injected, and driven by nothing.
const BOUNDARIES: &[CheckedArtifactFault] = &[
    CheckedArtifactFault::BeforeAnchorScratchCreate,
    CheckedArtifactFault::AfterAnchorScratchWrite,
    CheckedArtifactFault::AfterAnchorScratchFlush,
    CheckedArtifactFault::AfterAnchorPublication,
    CheckedArtifactFault::BeforeAnchorRoundTrip,
    CheckedArtifactFault::AfterAnchorOutboundRename,
    CheckedArtifactFault::AfterAnchorReturnRename,
    CheckedArtifactFault::AfterAnchorReobservation,
    CheckedArtifactFault::BeforeAnchorAliasRetirement,
    CheckedArtifactFault::AfterAnchorAliasRetirement,
];

/// The two the retirement arm crosses. They are only reachable from the
/// stranded-alias state, so the matrix builds that state before driving them.
const RETIREMENT_BOUNDARIES: &[CheckedArtifactFault] = &[
    CheckedArtifactFault::BeforeAnchorAliasRetirement,
    CheckedArtifactFault::AfterAnchorAliasRetirement,
];

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gwz-anchor-{name}-{}-{}",
            std::process::id(),
            super::super::super::transition::TEMP_SEQUENCE
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn dir(&self) -> Dir {
        Dir::open_ambient_dir(&self.0, cap_std::ambient_authority()).unwrap()
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn names(root: &Path) -> BTreeSet<String> {
    std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn resident_anchor(root: &Path) -> Option<String> {
    let mut matched = names(root)
        .into_iter()
        .filter(|name| name.starts_with(ANCHOR_PREFIX) && !name.ends_with(".roundtrip"))
        .collect::<Vec<_>>();
    assert!(
        matched.len() <= 1,
        "at most one resident anchor: {matched:?}"
    );
    matched.pop()
}

fn retired_ordinals(root: &Path) -> BTreeSet<u32> {
    names(root)
        .into_iter()
        .filter_map(|name| name.strip_prefix(RETIRED_PREFIX).map(str::to_owned))
        .map(|ordinal| ordinal.parse::<u32>().expect("retired ordinal parses"))
        .collect()
}

/// The closed grammar, on a tree this protocol created. Anything outside this
/// set is an orphan the legacy nonce would have produced.
///
/// **[P3-1], stated where it bites:** the grammar is closed *forward*, not
/// retroactively. A `.ca1-anchor-scratch-<32hex>` left by a pre-4.2 Windows crash
/// matches none of these and is tolerated for ever — `survey` ignores it, so it
/// refuses nothing and blocks nothing, but no drive reclaims it either. This
/// assertion therefore describes a fresh tree, and
/// `legacy_nonce_orphans_are_tolerated_and_block_nothing` is the row that pins
/// the upgraded one.
fn assert_closed_grammar(root: &Path) {
    for name in names(root) {
        let closed = name == SCRATCH_NAME
            // Not "parses as a u32" but "is `retired_name`'s own rendering",
            // which is the predicate `survey` enforces from R2-E E6.2 on.
            || name
                .strip_prefix(RETIRED_PREFIX)
                .and_then(|ordinal| ordinal.parse::<u32>().ok())
                .is_some_and(|ordinal| retired_name(ordinal) == name)
            || (name.starts_with(ANCHOR_PREFIX)
                && (name.len() == ANCHOR_PREFIX.len() + 32 || name.ends_with(".roundtrip")));
        assert!(closed, "name outside the closed anchor grammar: {name}");
    }
}

#[test]
fn the_anchor_stages_through_one_deterministic_name_and_settles_resident() {
    let root = TempRoot::new("establish");
    prepare(&root.dir(), true, CODE, LABEL).unwrap();

    let anchor = resident_anchor(&root.0).expect("an anchor is resident");
    assert_eq!(anchor.len(), ANCHOR_PREFIX.len() + 32);
    assert_eq!(std::fs::read(root.0.join(&anchor)).unwrap(), ANCHOR_BYTES);
    assert_eq!(names(&root.0), BTreeSet::from([anchor]));

    // Idempotent: a second drive observes Ready and mutates nothing.
    let before = names(&root.0);
    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    assert_eq!(names(&root.0), before);
}

#[test]
fn a_reader_never_plants_an_anchor_in_an_empty_private_area() {
    let root = TempRoot::new("reader");
    prepare(&root.dir(), false, CODE, LABEL).unwrap();
    assert!(names(&root.0).is_empty());
}

#[test]
fn every_anchor_boundary_restarts_to_one_resident_anchor() {
    for boundary in BOUNDARIES {
        let root = TempRoot::new(&format!("restart-{boundary:?}"));
        if RETIREMENT_BOUNDARIES.contains(boundary) {
            // The retirement arm runs only from the stranded-alias state, which
            // needs one object under two names with one durable identity.
            if strand_alias(&root.0).is_none() {
                assert!(
                    !hard_links_share_durable_identity(),
                    "{boundary:?}: the state is producible here and must be driven"
                );
                continue;
            }
        }
        fail_next_checked_artifact_at(*boundary);
        // The first four boundaries are crossed while establishing; the last
        // four only once an anchor is resident, so the drive that reaches them
        // is the round trip.
        let established = prepare(&root.dir(), true, CODE, LABEL);
        if established.is_ok() {
            assert!(
                round_trip(&root.dir(), CODE, LABEL).is_err(),
                "{boundary:?} must interrupt one of the two drives"
            );
        }
        assert_closed_grammar(&root.0);

        // A fresh drive converges from whatever window the crash left.
        prepare(&root.dir(), true, CODE, LABEL)
            .unwrap_or_else(|error| panic!("{boundary:?}: {error:?}"));
        round_trip(&root.dir(), CODE, LABEL)
            .unwrap_or_else(|error| panic!("{boundary:?}: {error:?}"));
        let anchor =
            resident_anchor(&root.0).unwrap_or_else(|| panic!("{boundary:?}: no resident anchor"));
        assert_eq!(
            std::fs::read(root.0.join(&anchor)).unwrap(),
            ANCHOR_BYTES,
            "{boundary:?}"
        );
        assert_closed_grammar(&root.0);
    }
}

#[test]
fn repeated_crashes_at_one_boundary_keep_the_one_staging_name() {
    // The legacy nonce produced one unreclaimable orphan per crash. Twelve
    // rounds at the boundary that leaves a staged scratch must leave exactly
    // one name, and it must be the deterministic one.
    let root = TempRoot::new("repeated-scratch");
    for round in 0..12 {
        fail_next_checked_artifact_at(CheckedArtifactFault::AfterAnchorScratchFlush);
        assert!(
            prepare(&root.dir(), true, CODE, LABEL).is_err(),
            "round {round}"
        );
        assert_eq!(
            names(&root.0),
            BTreeSet::from([SCRATCH_NAME.to_owned()]),
            "round {round}: the staging name is reused, never reallocated"
        );
    }
    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    let anchor = resident_anchor(&root.0).expect("an anchor is resident");
    assert_eq!(names(&root.0), BTreeSet::from([anchor]));
}

#[test]
fn a_short_scratch_from_a_crashed_write_is_rewritten_in_place() {
    let root = TempRoot::new("short-scratch");
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterAnchorScratchWrite);
    assert!(prepare(&root.dir(), true, CODE, LABEL).is_err());
    // Reproduce the shorter window the platform may leave: the name exists but
    // the bytes never landed.
    std::fs::write(root.0.join(SCRATCH_NAME), b"").unwrap();

    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    let anchor = resident_anchor(&root.0).expect("an anchor is resident");
    assert_eq!(std::fs::read(root.0.join(&anchor)).unwrap(), ANCHOR_BYTES);
    assert_eq!(names(&root.0), BTreeSet::from([anchor]));
}

#[test]
fn an_interrupted_round_trip_returns_the_anchor_from_its_outbound_alias() {
    let root = TempRoot::new("needs-return");
    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    let anchor = resident_anchor(&root.0).expect("an anchor is resident");
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterAnchorOutboundRename);
    assert!(round_trip(&root.dir(), CODE, LABEL).is_err());
    assert_eq!(
        names(&root.0),
        BTreeSet::from([format!("{anchor}.roundtrip")]),
        "the crash leaves exactly the outbound alias"
    );

    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    assert_eq!(names(&root.0), BTreeSet::from([anchor]));
}

/// Build the one state the legacy arm reconciled by deletion: a single object
/// resident under both the anchor name and its outbound alias.
///
/// `None` when the platform cannot hold the state at all — see
/// [`hard_links_share_durable_identity`].
///
/// The construction links *before* it names: creating a hard link is not
/// identity-neutral everywhere, so the anchor's identity-addressed name has to be
/// derived from the identity the object settles on once both names exist.
fn strand_alias(root: &Path) -> Option<(String, String)> {
    let first = root.join(".gwz-anchor-link-a");
    let second = root.join(".gwz-anchor-link-b");
    std::fs::write(&first, ANCHOR_BYTES).unwrap();
    std::fs::hard_link(&first, &second).unwrap();
    let dir = Dir::open_ambient_dir(root, cap_std::ambient_authority()).unwrap();
    let identity = super::verify(&dir, OsStr::new(".gwz-anchor-link-a"), CODE, LABEL).unwrap();
    let sibling = super::verify(&dir, OsStr::new(".gwz-anchor-link-b"), CODE, LABEL).unwrap();
    if identity != sibling {
        std::fs::remove_file(&first).unwrap();
        std::fs::remove_file(&second).unwrap();
        return None;
    }
    let anchor = anchor_name(&identity.name_digest());
    let alias = format!("{anchor}.roundtrip");
    std::fs::rename(&first, root.join(&anchor)).unwrap();
    std::fs::rename(&second, root.join(&alias)).unwrap();
    Some((anchor, alias))
}

/// Whether two names for one object carry the same durable identity here.
///
/// Linux does (`name_to_handle_at` encodes the inode, which hard links share) and
/// so does Windows (`FILE_ID_INFO.FileId` is per file). **macOS does not**:
/// `ATTR_CMN_OBJPERMANENTID` is allocated per hard link, and creating the second
/// link also re-homes the first onto an indirect node with a fresh id. Measured,
/// not assumed — the fixture above asks the platform rather than reading a `cfg`.
///
/// The consequence is a property of the *state*, not of this step:
/// `NeedsRetireAlias` requires one object under two names with one identity, so
/// on macOS it cannot exist, and the legacy `remove_file` this retirement
/// replaces was equally unreachable there. The two rows below therefore assert
/// the retirement where the state is producible and the refusal where it is not;
/// the Windows probe is what executes the retirement on the platform the anchor
/// actually serves.
fn hard_links_share_durable_identity() -> bool {
    let probe = TempRoot::new("link-identity-probe");
    let a = probe.0.join("a");
    let b = probe.0.join("b");
    std::fs::write(&a, ANCHOR_BYTES).unwrap();
    std::fs::hard_link(&a, &b).unwrap();
    let dir = probe.dir();
    super::verify(&dir, OsStr::new("a"), CODE, LABEL).unwrap()
        == super::verify(&dir, OsStr::new("b"), CODE, LABEL).unwrap()
}

/// Pins the platform fact the two retirement rows branch on, so a green run is
/// evidence of *which* branch they took: on Windows — the platform the anchor
/// actually serves, and the one the probe covers — they execute the retirement,
/// and only on macOS do they fall to the refusal.
#[test]
fn hard_link_identity_sharing_is_what_the_retirement_rows_assume() {
    let shared = hard_links_share_durable_identity();
    if cfg!(target_os = "macos") {
        assert!(
            !shared,
            "macOS allocates ATTR_CMN_OBJPERMANENTID per hard link; if that \
             changed, the retirement rows must stop skipping here"
        );
    } else {
        assert!(
            shared,
            "this target must share durable identity across hard links, or the \
             retirement rows below silently stop covering the retirement"
        );
    }
}

/// The retirement this step exists to install: the legacy arm called
/// `remove_file` on this alias.
#[test]
fn a_stranded_alias_is_retired_durably_and_never_removed() {
    let root = TempRoot::new("retire-alias");
    let Some((anchor, _)) = strand_alias(&root.0) else {
        assert!(
            !hard_links_share_durable_identity(),
            "the state is producible here and the fixture must produce it"
        );
        return;
    };

    prepare(&root.dir(), true, CODE, LABEL).unwrap();

    assert_eq!(
        names(&root.0),
        BTreeSet::from([anchor.clone(), retired_name(0)]),
        "the alias is retired onto its next free ordinal, not deleted"
    );
    assert_eq!(
        std::fs::read(root.0.join(retired_name(0))).unwrap(),
        ANCHOR_BYTES,
        "the retired object survives as evidence"
    );
    assert_eq!(std::fs::read(root.0.join(&anchor)).unwrap(), ANCHOR_BYTES);
    assert_closed_grammar(&root.0);
}

/// [P2-2]: the state **recurs**, so the retirement must too. Round 1 refused a
/// second stranding for ever and bricked P5 for all seven `AnchoredPrivateArea`
/// callers; this row drives the recurrence across a crash at each retirement
/// boundary and requires convergence every time.
#[test]
fn a_recurring_stranding_retires_onto_the_next_free_ordinal_and_converges() {
    let root = TempRoot::new("retire-recurrence");
    let Some((anchor, alias)) = strand_alias(&root.0) else {
        assert!(
            !hard_links_share_durable_identity(),
            "the state is producible here and the recurrence must be driven"
        );
        return;
    };

    // First stranding: retires onto ordinal 0 and the barrier works.
    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    round_trip(&root.dir(), CODE, LABEL).unwrap();
    assert_eq!(retired_ordinals(&root.0), BTreeSet::from([0]));

    // Second stranding, interrupted *before* the retirement edge: nothing moved,
    // and the drive that follows must still converge.
    std::fs::hard_link(root.0.join(&anchor), root.0.join(&alias)).unwrap();
    fail_next_checked_artifact_at(CheckedArtifactFault::BeforeAnchorAliasRetirement);
    assert!(prepare(&root.dir(), true, CODE, LABEL).is_err());
    assert_eq!(retired_ordinals(&root.0), BTreeSet::from([0]));
    assert_closed_grammar(&root.0);

    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    round_trip(&root.dir(), CODE, LABEL).unwrap();
    assert_eq!(
        retired_ordinals(&root.0),
        BTreeSet::from([0, 1]),
        "the recurrence takes the next free ordinal instead of wedging"
    );

    // Third stranding, interrupted *after* the retirement edge: the retirement is
    // durable, the drive still reports the interruption, and the next one settles.
    std::fs::hard_link(root.0.join(&anchor), root.0.join(&alias)).unwrap();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterAnchorAliasRetirement);
    assert!(prepare(&root.dir(), true, CODE, LABEL).is_err());
    assert_eq!(retired_ordinals(&root.0), BTreeSet::from([0, 1, 2]));
    assert_closed_grammar(&root.0);

    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    round_trip(&root.dir(), CODE, LABEL).unwrap();
    assert_eq!(
        names(&root.0),
        BTreeSet::from([anchor, retired_name(0), retired_name(1), retired_name(2)]),
        "every retired object is bounded evidence of one real stranding"
    );
}

/// [P3-6]: the racing-drive window, executed rather than argued. A second drive
/// takes the ordinal this one computed, between the survey and the edge. The
/// no-replace publication refuses, and the drive that follows re-surveys and
/// takes the next free ordinal — there is no fixed point to wedge on.
#[test]
fn a_racing_drive_that_takes_the_ordinal_first_loses_and_the_next_drive_converges() {
    let root = TempRoot::new("retire-race");
    let Some((anchor, _)) = strand_alias(&root.0) else {
        assert!(
            !hard_links_share_durable_identity(),
            "the state is producible here and the race must be driven"
        );
        return;
    };
    let raced = root.0.join(retired_name(0));
    run_next_checked_artifact_at(
        CheckedArtifactFault::BeforeAnchorAliasRetirement,
        move || {
            // The racing drive's own retirement lands on ordinal 0 after this
            // drive's survey computed it and before its edge reaches it.
            std::fs::write(&raced, ANCHOR_BYTES).unwrap();
        },
    );

    let error = prepare(&root.dir(), true, CODE, LABEL).unwrap_err();
    assert_eq!(error.code, CODE);
    assert_eq!(
        retired_ordinals(&root.0),
        BTreeSet::from([0]),
        "the loser publishes nothing"
    );
    assert_closed_grammar(&root.0);

    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    round_trip(&root.dir(), CODE, LABEL).unwrap();
    assert_eq!(
        retired_ordinals(&root.0),
        BTreeSet::from([0, 1]),
        "the re-survey takes the next free ordinal"
    );
    assert_eq!(std::fs::read(root.0.join(&anchor)).unwrap(), ANCHOR_BYTES);
}

/// Smallest-free rather than count or max+1: with a gap in the resident set the
/// retirement fills the gap, and neither of the alternatives could — "count"
/// recomputes an occupied name for ever against a no-replace publication.
#[test]
fn the_retirement_ordinal_is_the_smallest_free_one_observed() {
    let root = TempRoot::new("retire-gap");
    let Some((anchor, _)) = strand_alias(&root.0) else {
        assert!(!hard_links_share_durable_identity());
        return;
    };
    std::fs::write(root.0.join(retired_name(0)), ANCHOR_BYTES).unwrap();
    std::fs::write(root.0.join(retired_name(2)), ANCHOR_BYTES).unwrap();

    prepare(&root.dir(), true, CODE, LABEL).unwrap();

    assert_eq!(retired_ordinals(&root.0), BTreeSet::from([0, 1, 2]));
    assert_eq!(
        std::fs::read(root.0.join(retired_name(1))).unwrap(),
        ANCHOR_BYTES
    );
    assert_eq!(std::fs::read(root.0.join(&anchor)).unwrap(), ANCHOR_BYTES);
}

/// A retired name whose ordinal does not parse was written by something other
/// than this protocol, and the survey must refuse rather than adopt it.
#[test]
fn a_malformed_retired_name_is_refused_not_adopted() {
    let root = TempRoot::new("retire-malformed");
    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    std::fs::write(root.0.join(format!("{RETIRED_PREFIX}v1")), ANCHOR_BYTES).unwrap();

    let error = prepare(&root.dir(), true, CODE, LABEL).unwrap_err();

    assert_eq!(error.code, CODE);
    assert!(error.message.contains("missing or ambiguous"), "{error:?}");
}

/// The renderings `retired_name` never writes are foreign in exactly the same
/// way — and until R2-E E6.2 the survey adopted them. `u32::from_str` accepts
/// zero-padded and sign-prefixed forms, so `retired-007` and `retired-+7` each
/// parsed to an ordinal and joined the residency set, letting a name this
/// protocol did not write hold a slot it had never retired onto. The survey now
/// admits an ordinal only if `retired_name` would have produced that exact name.
///
/// The deferral terms (`GwzM5-8R2DSettledTuple.md:659-662`) called the cure "a
/// canonical two-digit parse". `retired_name` renders the ordinal *unpadded*,
/// so a fixed width would have rejected every name the protocol actually
/// writes; the executed cure re-renders through `retired_name` itself. The
/// first loop below is why that distinction is not a matter of opinion.
#[test]
fn a_non_canonical_retired_ordinal_is_refused_not_adopted() {
    // Every rendering the protocol can write round-trips through the check the
    // survey now applies — including the widths a two-digit rule would have
    // refused.
    for ordinal in [0, 1, 7, 9, 10, 99, 100, 1_000, u32::MAX] {
        let name = retired_name(ordinal);
        let rendering = name
            .strip_prefix(RETIRED_PREFIX)
            .expect("carries the prefix");
        assert_eq!(rendering.parse::<u32>(), Ok(ordinal));
        assert_eq!(retired_name(ordinal), name, "{ordinal}");
    }

    // Each of these parses, and none is a name `retired_name` emits.
    for rendering in ["007", "+7", "00", "0000000010"] {
        let ordinal = rendering.parse::<u32>().expect("the old guard admitted it");
        assert_ne!(
            retired_name(ordinal),
            format!("{RETIRED_PREFIX}{rendering}"),
            "{rendering} must not be a rendering this protocol writes"
        );

        let root = TempRoot::new("retire-non-canonical");
        prepare(&root.dir(), true, CODE, LABEL).unwrap();
        let foreign = format!("{RETIRED_PREFIX}{rendering}");
        std::fs::write(root.0.join(&foreign), ANCHOR_BYTES).unwrap();

        let error = prepare(&root.dir(), true, CODE, LABEL).unwrap_err();

        assert_eq!(error.code, CODE, "{rendering}");
        assert!(
            error.message.contains("missing or ambiguous"),
            "{rendering}: {error:?}"
        );
        assert!(
            root.0.join(&foreign).is_file(),
            "{rendering}: a refusal mutates nothing"
        );
    }

    // The positive control the guard must not over-refuse: canonical retired
    // names on the same tree are read, not refused.
    let root = TempRoot::new("retire-canonical");
    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    for ordinal in [0, 1, 10] {
        std::fs::write(root.0.join(retired_name(ordinal)), ANCHOR_BYTES).unwrap();
    }

    prepare(&root.dir(), true, CODE, LABEL).unwrap();

    assert_eq!(retired_ordinals(&root.0), BTreeSet::from([0, 1, 10]));
    assert_closed_grammar(&root.0);
}

/// [P3-1]: a pre-4.2 nonce orphan is tolerated, never reclaimed. It must block
/// nothing — the closed grammar is forward-looking, and an upgraded tree keeps
/// working with its old litter in place.
#[test]
fn legacy_nonce_orphans_are_tolerated_and_block_nothing() {
    let root = TempRoot::new("legacy-orphan");
    let orphan = format!(".ca1-anchor-scratch-{}", "ab".repeat(16));
    std::fs::write(root.0.join(&orphan), ANCHOR_BYTES).unwrap();

    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    round_trip(&root.dir(), CODE, LABEL).unwrap();

    let anchor = resident_anchor(&root.0).expect("an anchor is resident");
    assert_eq!(
        names(&root.0),
        BTreeSet::from([anchor, orphan.clone()]),
        "the orphan neither blocks the protocol nor is reclaimed by it"
    );
    assert_eq!(std::fs::read(root.0.join(&orphan)).unwrap(), ANCHOR_BYTES);
}

/// A foreign hard link onto a *settled* anchor is not the state above: on macOS
/// it re-homes the anchor and its identity-addressed name goes stale, and the
/// survey must refuse rather than adopt either name. Pinned because the
/// retirement arm's guard is what makes the distinction, and because the
/// platform fact behind it is not obvious.
#[test]
fn a_foreign_link_that_re_homes_the_anchor_is_refused_without_mutation() {
    let root = TempRoot::new("foreign-link");
    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    let anchor = resident_anchor(&root.0).expect("an anchor is resident");
    let alias = format!("{anchor}.roundtrip");
    std::fs::hard_link(root.0.join(&anchor), root.0.join(&alias)).unwrap();

    let before = names(&root.0);
    let settled = super::verify(&root.dir(), OsStr::new(&anchor), CODE, LABEL).unwrap();
    let re_homed = anchor_name(&settled.name_digest()) != anchor;
    let outcome = prepare(&root.dir(), true, CODE, LABEL);

    if re_homed {
        let error = outcome.unwrap_err();
        assert_eq!(error.code, CODE);
        assert!(error.message.contains("missing or ambiguous"), "{error:?}");
        assert_eq!(names(&root.0), before, "a refusal mutates nothing");
    } else {
        // Identity-neutral linking: the state is the retirement arm's, and it
        // converges onto the same closed grammar as `strand_alias` produces.
        outcome.unwrap();
        assert_eq!(names(&root.0), BTreeSet::from([anchor, retired_name(0)]));
    }
}

/// P1 at the anchor's own edges (freeze §4.3 row E22, "P1 + P5"): a same-byte
/// substitution inside the window between the survey's proof and the outbound
/// rename is refused before the edge.
#[test]
fn the_round_trip_refuses_a_substituted_anchor_before_the_edge() {
    let root = TempRoot::new("substituted-anchor");
    prepare(&root.dir(), true, CODE, LABEL).unwrap();
    let anchor = resident_anchor(&root.0).expect("an anchor is resident");
    let resident = root.0.join(&anchor);
    run_next_checked_artifact_at(CheckedArtifactFault::BeforeAnchorRoundTrip, move || {
        // Staged beside the original so a recycled inode number cannot falsify
        // the new-object precondition (the `exact_source` idiom).
        let staged = resident.with_file_name(".gwz-anchor-substitution-staging");
        std::fs::write(&staged, ANCHOR_BYTES).unwrap();
        std::fs::rename(&staged, &resident).unwrap();
    });

    let error = round_trip(&root.dir(), CODE, LABEL).unwrap_err();

    assert_eq!(error.code, CODE);
    assert!(
        error
            .message
            .contains("publication source identity changed"),
        "{error:?}"
    );
    assert_eq!(
        names(&root.0),
        BTreeSet::from([anchor]),
        "a refused round trip performs no namespace mutation"
    );
}

#[test]
fn foreign_bytes_under_the_anchor_prefix_are_refused_not_adopted() {
    let root = TempRoot::new("foreign-anchor");
    std::fs::write(root.0.join(anchor_name(&[7; 16])), b"foreign\n").unwrap();
    let error = prepare(&root.dir(), true, CODE, LABEL).unwrap_err();
    assert_eq!(error.code, CODE);
    assert!(
        error.message.contains("anchor bytes are invalid"),
        "{error:?}"
    );
}

#[test]
fn the_outbound_alias_name_is_derived_natively() {
    assert_eq!(
        roundtrip_name(OsStr::new(".ca1-durability-anchor-0123")),
        OsString::from(".ca1-durability-anchor-0123.roundtrip")
    );
}

/// R2-E Phase E2, DECISION B-3 — the `RoamingAnchoredTarget` arm's own rows.
///
/// The supplied alias carries the catalog's roaming-anchor bytes, not this
/// module's `ANCHOR_BYTES`: the roaming arm takes its content from the caller
/// that owns the constant, so nothing is duplicated here either.
const ROAMING_BYTES: &[u8] = b"GWZ-ROAMING-ANCHOR-V1\n";

/// The reserved leaf a real barrier would use is a schedule-derived action-slot
/// name (OPEN-B3). Its exact spelling is irrelevant to the platform arm; what
/// matters is that it is a caller-supplied name this protocol never surveys for.
const RESERVED_LEAF: &str = "action-00-roaming-alias-v1";

fn place_alias(root: &Path, leaf: &str) {
    std::fs::write(root.join(leaf), ROAMING_BYTES).unwrap();
}

/// **OPEN-B7's probe, in the `hard_link_identity_sharing_is_what_the_retirement_rows_assume`
/// shape: it measures the platform rather than reading a `cfg`.**
///
/// The open question is whether the P5 round trip behaves the same when it
/// renames a *freshly created* alias as when it renames a long-resident anchor.
/// This asks the platform directly, on two shapes, and asserts they agree: the
/// object survives its round trip under its own name, with its own bytes and
/// its own durable identity.
///
/// **What the second shape actually is** (E2 review [P3-4], which found the old
/// `"long-resident"` label overstated): one directory mutation — a sibling
/// written and unlinked — before the same freshly created alias is round-tripped.
/// It is a proxy for residency across other dirent activity, not a long-lived
/// object, and it is labelled `"aged-directory"` for exactly that reason. A true
/// long-residency shape is not constructible in a unit test.
///
/// What it does not prove, stated so a green run is not over-read: the *dirent
/// ordering* the Windows round trip exists to deliver is not observable from
/// inside the process. This row proves the mechanism is available and
/// identity-preserving on both shapes; the native Windows leg runs at the
/// three-platform dispatch.
#[test]
fn the_supplied_roaming_round_trip_is_measured_on_both_alias_shapes() {
    for (label, age_the_directory) in [("fresh", false), ("aged-directory", true)] {
        let root = TempRoot::new(&format!("roaming-{label}"));
        place_alias(&root.0, RESERVED_LEAF);
        if age_the_directory {
            let noise = root.0.join("noise");
            std::fs::write(&noise, b"noise\n").unwrap();
            std::fs::remove_file(&noise).unwrap();
        }
        let dir = root.dir();
        let before = super::super::verify_leaf_bytes(
            &dir,
            OsStr::new(RESERVED_LEAF),
            ROAMING_BYTES,
            CODE,
            LABEL,
        )
        .unwrap();

        round_trip_supplied(&dir, OsStr::new(RESERVED_LEAF), ROAMING_BYTES, CODE, LABEL).unwrap();

        let after = super::super::verify_leaf_bytes(
            &dir,
            OsStr::new(RESERVED_LEAF),
            ROAMING_BYTES,
            CODE,
            LABEL,
        )
        .unwrap();
        assert_eq!(
            before, after,
            "{label}: the supplied alias must survive its round trip as the same object"
        );
        assert_eq!(
            names(&root.0),
            BTreeSet::from([RESERVED_LEAF.to_owned()]),
            "{label}: the round trip must leave only the reserved leaf"
        );
    }
}

/// The roaming arm surveys for nothing and establishes nothing: an empty target
/// parent is a typed refusal, never a directory that quietly gains a permanent
/// `.ca1-durability-anchor-*`. That is the whole reason DECISION B-3 needed a
/// third class instead of passing `AnchoredPrivateArea`.
#[test]
fn the_roaming_arm_never_establishes_an_anchor_of_its_own() {
    let root = TempRoot::new("roaming-empty");
    let error = round_trip_supplied(
        &root.dir(),
        OsStr::new(RESERVED_LEAF),
        ROAMING_BYTES,
        CODE,
        LABEL,
    )
    .unwrap_err();
    assert_eq!(error.code, CODE);
    assert!(error.message.contains("not exactly resident"), "{error:?}");
    assert!(
        names(&root.0).is_empty(),
        "the refused barrier planted something in the target parent"
    );
}

/// The crash window inside the round trip converges rather than stranding, for a
/// caller that enters this arm **directly**: `round_trip_supplied` opens with
/// `prepare_roaming_target`, which returns the object from its outbound name
/// before the trip begins.
///
/// The drive does not reach the arm in this state — its own entry decision
/// converges first, which is the E2 review's [P2-1] cure and is driven at
/// `a_mid_round_trip_roaming_residue_converges_*`. This row keeps the arm safe
/// for a caller that did not prepare.
#[test]
fn an_outbound_roaming_alias_is_returned_when_the_arm_is_entered_directly() {
    let root = TempRoot::new("roaming-outbound");
    let outbound = roundtrip_name(OsStr::new(RESERVED_LEAF));
    place_alias(&root.0, outbound.to_str().unwrap());

    round_trip_supplied(
        &root.dir(),
        OsStr::new(RESERVED_LEAF),
        ROAMING_BYTES,
        CODE,
        LABEL,
    )
    .unwrap();

    assert_eq!(names(&root.0), BTreeSet::from([RESERVED_LEAF.to_owned()]));
    assert_eq!(
        std::fs::read(root.0.join(RESERVED_LEAF)).unwrap(),
        ROAMING_BYTES
    );
}

/// A both-names state reached *inside* this arm is refused rather than guessed
/// at, exactly as the resident protocol's `survey` refuses its own ambiguous
/// shapes.
///
/// **The justification is not "unproducible", and the first landing of this test
/// said it was** (E2 review [P2-1]): the drive's own leaf-only entry decision
/// produced it, by creating a second object over an empty reserved leaf while
/// the outbound name was resident. That entry decision is now
/// `super::super::prepare_roaming_target`'s, which converges the outbound name
/// before anything is created — so the state is unreachable *from the drive*,
/// and this refusal now covers the case it is actually for: a foreign object
/// appearing after that decision. It mutates nothing, and the next drive's entry
/// decision resolves it (`a_legacy_both_names_tree_settles_with_a_tolerated_orphan_*`
/// in `namespace/tests_barrier_matrix.rs`).
#[test]
fn a_roaming_alias_resident_under_both_names_is_refused_inside_the_arm() {
    let root = TempRoot::new("roaming-ambiguous");
    place_alias(&root.0, RESERVED_LEAF);
    place_alias(
        &root.0,
        roundtrip_name(OsStr::new(RESERVED_LEAF)).to_str().unwrap(),
    );

    let error = round_trip_supplied(
        &root.dir(),
        OsStr::new(RESERVED_LEAF),
        ROAMING_BYTES,
        CODE,
        LABEL,
    )
    .unwrap_err();
    assert!(error.message.contains("not exactly resident"), "{error:?}");
    assert_eq!(names(&root.0).len(), 2, "a refused barrier mutates nothing");
}

/// Foreign bytes under the reserved leaf are refused before any rename, so the
/// roaming arm cannot lend a barrier to an object it did not recognise.
#[test]
fn foreign_bytes_under_the_reserved_leaf_are_refused_before_the_edge() {
    let root = TempRoot::new("roaming-foreign");
    std::fs::write(root.0.join(RESERVED_LEAF), b"foreign\n").unwrap();
    let error = round_trip_supplied(
        &root.dir(),
        OsStr::new(RESERVED_LEAF),
        ROAMING_BYTES,
        CODE,
        LABEL,
    )
    .unwrap_err();
    assert!(error.message.contains("bytes are invalid"), "{error:?}");
    assert_eq!(names(&root.0), BTreeSet::from([RESERVED_LEAF.to_owned()]));
}
