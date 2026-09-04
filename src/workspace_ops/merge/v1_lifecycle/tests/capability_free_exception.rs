//! P-2, the capability-free exception's negative tripwire (2026-09-02).
//!
//! Authority: `dev-docs/GwzM5-8R2E-CapabilityFreeAmendment.md` §3 (gwz-dev root)
//! and the operator ruling of the same date. E0.2 §5.2's capability-free list
//! STANDS, so every §10 row `:275`-`:279` writer reached from a listed operation
//! keeps its raw publication primitive permanently. P-1 (the checker's inventory)
//! counts what STAYS; it cannot say the carved arms took no checked door. These
//! rows say it.
//!
//! Fifteen files are PURE — every writer in them is carved, so the whole file is
//! asserted door-free. M5d deleted the two MIXED v0 abort arms and three PURE
//! v0-engine files (`finalize.rs`, `preserve/artifacts.rs`, `store/archived.rs`);
//! no mixed row remains. The needle is the `CheckedArtifact` door vocabulary and
//! NOT the git backend's `_checked` suffix — `commit_gwz_paths_checked`
//! (`v1_lifecycle/finalization/execute.rs:57`) is a git call, and a `_checked(`
//! needle would fail that file on day one; row 1 states that exclusion.
//!
//! **M5d step (3) adds the sixteenth row and it fits NEITHER shape** — the
//! third scan below exists for that reason and no other. The entering carved
//! row is `checked_artifact/entry.rs`, whose raw-create arm publishes the merge
//! record on a handle-fail volume (`GwzM5-8M5d-Charter.md` §3). MEASURED, not
//! assumed: under the masker that file names the two needles above ZERO times,
//! because it is INSIDE the boundary and spells the door as the type
//! `CheckedArtifact` (46 times) rather than as the module path
//! `checked_artifact`. So `PURE_CARVED_FILES` would pass it VACUOUSLY — the
//! assertion would hold even if the arm were converted back to the checked
//! door — and `MIXED_CARVED_ARMS`' positive control, which requires the file to
//! name a needle somewhere, would FAIL. The cure is a needle that IS the door
//! as the boundary module spells it, with its live positive control in the same
//! file. `GwzM5-8M5d-GateRevisions.md`'s Q14 anticipated the row's shape and
//! named the type; it did not measure that the needles are the snake_case
//! module paths. This is that correction, on the record.
//!
//! Anti-vacuity: each carved path is read by name and a missing one panics; a byte
//! floor refuses a blinded read; each region lookup panics by name; both needles
//! have a live positive control. Every read is CRLF-normalized (the `f715ddf`
//! lesson), load bearing because the mixed halves are region-scoped. Self-
//! exclusion: this file is under `tests/`, and both needles are SPLIT.

use std::path::PathBuf;

use super::{item_body, masked_code};

/// The checker inventory's rows minus the two mixed files: every writer in each
/// is carved, so the WHOLE file is asserted.
const PURE_CARVED_FILES: [&str; 15] = [
    "stash/mod.rs",
    "workspace_ops/handle_branch.rs",
    "workspace_ops/handle_commit.rs",
    "workspace_ops/handle_create_repo.rs",
    "workspace_ops/handle_init_from_sources.rs",
    "workspace_ops/handle_materialize.rs",
    "workspace_ops/handle_repo_lifecycle.rs",
    "workspace_ops/handle_stage.rs",
    "workspace_ops/handle_stash/commands.rs",
    "workspace_ops/merge/store/gc.rs",
    "workspace_ops/merge/store/retention.rs",
    "workspace_ops/merge/v1_lifecycle/archive.rs",
    "workspace_ops/merge/v1_lifecycle/store/archive.rs",
    "workspace_ops/pull_head_member_preflight.rs",
    "workspace_ops/sync_workspace_boundary.rs",
];

/// The MIXED files, each with the signature of the ARM this exception carves.
/// M5d: both mixed v0 abort arms left with the engine; the array is the pin.
const MIXED_CARVED_ARMS: [(&str, &str); 0] = [];

/// M5d step (3)'s entering row and the signature of its carved arm.
///
/// `entry.rs` is the checked boundary's own module, so its carved arm sits
/// beside forty-six live door references in the same file. That is what makes
/// this row scannable at all — and why it takes the boundary's own needle
/// rather than the module-path needles above (see the file header).
const BOUNDARY_CARVED_ARM: (&str, &str) = (
    "checked_artifact/entry.rs",
    "fn create_merge_store_record_raw(",
);

/// Certainly names the first needle, so an absence is the arms' property.
const DOOR_POSITIVE_CONTROL: &str = "workspace_ops/merge/root/artifact_facts.rs";

/// The git backend's own suffix, which is NOT a checked-boundary door.
const GIT_CHECKED_SEAM: &str = "commit_gwz_paths_checked";

/// The smallest carved file (`store/gc.rs`) is over 500 bytes stripped.
const SOURCE_FLOOR: usize = 400;

/// The two door needles, each split so this file never spells one.
fn doors() -> [String; 2] {
    [
        format!("checked_{}", "artifact"),
        format!("artifact_{}", "facts"),
    ]
}

/// The door as the BOUNDARY MODULE itself spells it: the acquiring type, not
/// the module path. Split for the same self-exclusion reason as `doors`.
fn boundary_door() -> String {
    format!("Checked{}", "Artifact")
}

/// One carved production source, by crate-relative path, comment-stripped.
fn carved(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("carved path `{relative}` is unreadable: {error}"));
    let stripped = masked_code(relative, &text);
    assert!(
        stripped.len() >= SOURCE_FLOOR,
        "`{relative}` read as {} bytes, under the {SOURCE_FLOOR} floor: an absence \
         asserted of it would be an artefact of a blinded or truncated read",
        stripped.len()
    );
    stripped
}

/// Every PURE carved file names no checked-boundary door.
#[test]
fn the_pure_carved_capability_free_files_name_no_checked_boundary_door() {
    let doors = doors();
    let control = carved(DOOR_POSITIVE_CONTROL);
    assert!(
        control.contains(doors[0].as_str()),
        "the door needle no longer matches `{DOOR_POSITIVE_CONTROL}`, so every absence \
         below proves nothing"
    );
    assert!(
        carved("workspace_ops/merge/v1_lifecycle/finalization/execute.rs")
            .contains(GIT_CHECKED_SEAM)
            && !doors.iter().any(|d| GIT_CHECKED_SEAM.contains(d.as_str())),
        "the git backend's `{GIT_CHECKED_SEAM}` is DELIBERATELY not a boundary door: a \
         `_checked(` needle would fail `finalization/execute.rs` on day one. Re-scope \
         the needle deliberately, or restore the seam this exclusion is about"
    );
    for relative in PURE_CARVED_FILES {
        let text = carved(relative);
        for door in &doors {
            assert!(
                !text.contains(door.as_str()),
                "`{relative}` names the checked boundary door vocabulary `{door}`, but \
                 GwzM5-8R2E-CapabilityFreeAmendment.md §3 carves ALL of its writers out \
                 PERMANENTLY: converting one places a capability-free operation on the \
                 durable-identity probe, which E0.2 §5.2's list forbids. Revise the \
                 amendment, at DR-1, before this may exist"
            );
        }
    }
}

/// In each MIXED file the carved ARM names no door, measured against the same
/// file's converted arms as a live positive control.
#[test]
fn the_mixed_files_carved_arms_name_no_checked_boundary_door() {
    let doors = doors();
    for (relative, signature) in MIXED_CARVED_ARMS {
        let text = carved(relative);
        assert!(
            doors.iter().any(|door| text.contains(door.as_str())),
            "`{relative}` names no checked door at all: it is pinned as a MIXED file \
             whose converted arms control for its carved one, so it belongs in \
             `PURE_CARVED_FILES` — or a conversion was reverted"
        );
        let carved_arm = item_body(&text, relative, signature);
        assert!(
            carved_arm.len() >= SOURCE_FLOOR,
            "`{signature}` extracted as {} bytes; the region scan collapsed",
            carved_arm.len()
        );
        for door in &doors {
            assert!(
                !carved_arm.contains(door.as_str()),
                "the carved arm `{signature}` in `{relative}` names the boundary door \
                 vocabulary `{door}`. That arm runs from merge abort, which E0.2 §5.2's \
                 capability-free list keeps off the durable-identity probe; \
                 GwzM5-8R2E-CapabilityFreeAmendment.md §3 carves it PERMANENTLY and must \
                 be revised, at DR-1, first"
            );
        }
    }
}

/// The boundary module's own carved arm — the record create's RAW publication
/// on a handle-fail volume — names no checked door.
///
/// This is the fail-closed half of the entering inventory row. The checker's
/// side counts that `entry.rs` names the raw primitive exactly once; this side
/// says the arm that names it reaches the boundary NOT AT ALL. Converting the
/// arm back to `CheckedArtifact::acquire` would put an ordinary merge start —
/// on E0.2 §5.2's capability-free list — back onto the durable-identity probe
/// on the one class of volume `GwzM5-8M5d-Charter.md` §3 wrote the arm for, and
/// it fails here as well as in the checker.
#[test]
fn the_boundary_modules_carved_raw_create_arm_names_no_checked_door() {
    let (relative, signature) = BOUNDARY_CARVED_ARM;
    let needle = boundary_door();
    let text = carved(relative);
    assert!(
        text.contains(needle.as_str()),
        "`{relative}` names the boundary door `{needle}` nowhere, so the absence \
         asserted of its carved arm below proves nothing. Either the door was \
         renamed — re-point this needle — or this file is no longer the boundary"
    );
    let carved_arm = item_body(&text, relative, signature);
    assert!(
        carved_arm.len() >= SOURCE_FLOOR,
        "`{signature}` extracted as {} bytes; the region scan collapsed",
        carved_arm.len()
    );
    for door in std::iter::once(needle.clone()).chain(doors()) {
        assert!(
            !carved_arm.contains(door.as_str()),
            "the carved arm `{signature}` in `{relative}` names the checked door \
             vocabulary `{door}`. That arm is an ordinary merge start's record create \
             on a volume without persistent file handles; converting it places a \
             capability-free operation on the durable-identity probe, which E0.2 §5.2's \
             list forbids, and strands the very start GwzM5-8M5d-Charter.md §3 wrote it \
             for. GwzM5-8R2E-CapabilityFreeAmendment.md §3 carves it PERMANENTLY and \
             must be revised first"
        );
    }
}
