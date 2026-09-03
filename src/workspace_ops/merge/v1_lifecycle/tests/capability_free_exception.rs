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
        carved("workspace_ops/merge/v1_lifecycle/finalization/execute.rs").contains(GIT_CHECKED_SEAM)
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
