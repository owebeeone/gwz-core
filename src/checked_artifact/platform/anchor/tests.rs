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
use super::{
    ANCHOR_BYTES, ANCHOR_PREFIX, RETIRED_PREFIX, SCRATCH_NAME, anchor_name, prepare, retired_name,
    round_trip, roundtrip_name,
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
            || name
                .strip_prefix(RETIRED_PREFIX)
                .is_some_and(|ordinal| ordinal.parse::<u32>().is_ok())
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
