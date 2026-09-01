//! P-2, the record-root exception's negative tripwire (2026-09-02).
//!
//! Authority: `dev-docs/GwzM5-8R2E-RecordRootAmendment.md` §3 (gwz-dev root),
//! the dated exception to ConsumerCheckpoint §10 row `:280` for
//! `store/rewrite.rs::commit` and only it. §1a's driven finding
//! (`probe/e4-3-detach-window-evidence`, `c9a7303`): routed through the checked
//! door, `commit` DETACHES the open record before publishing the goal, and in
//! between the root of reconciliation does not exist — every discovery path
//! enumerates `.gwz/merge` only — while `rename_durable(replace = true)` is
//! atomic. P-1 counts the `durable_fs` writer CLASS; it cannot say WHICH
//! primitive publishes, nor that the door stayed away. These rows say it.
//!
//! Anti-vacuity: `include_str!` makes a vanished subject a COMPILE error, each
//! region lookup panics by name, and the walk is fenced by a file-count floor
//! and an exact positive control on E4.2's LANDED door. Every read is
//! CRLF-normalized (the `f715ddf` lesson,
//! `interface_tests/r2d_seam_freeze.rs:221`), load bearing because both halves
//! are region-scoped and multi-line. Self-exclusion, two belts: this file sits
//! under `tests/`, which the walk skips as `production_rust_files` does, and
//! the forbidden door is a SPLIT needle joined at run time.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// The carved-out path's own source, bound at compile time.
const REWRITE: &str = include_str!("../../store/rewrite.rs");

/// The rejected candidate's checked door, split so this file never spells it.
const DOOR_HEAD: &str = "rewrite_merge_store";
const DOOR_TAIL: &str = "_record";

/// E4.2's LANDED sibling door and its exact namers: the positive control.
const LANDED_DOOR: &str = "create_merge_store_record";
const LANDED_DOOR_FILES: [&str; 2] = [
    "checked_artifact/entry.rs",
    "workspace_ops/merge/v1_lifecycle/store/rewrite.rs",
];

/// 411 production files at this landing; the floor fires at 350 — up to 61 files may
/// vanish unseen by it, so the EXACT positive control (`LANDED_DOOR` named by exactly its
/// two files) is what catches the blinding that matters.
const PRODUCTION_FILE_FLOOR: usize = 350;

/// CRLF normalized, `//` stripped to end of line, LINES PRESERVED — unlike
/// `catalog_activation_pin.rs:63-68`, whose `collect()` joins with nothing.
fn code(source: &str) -> String {
    source
        .replace("\r\n", "\n")
        .lines()
        // NAMED RESIDUAL (E4.3-B review [P3-3]): this strip is string-unaware, so a door
        // on a line whose earlier `//` sits inside a string literal (e.g. "https://…") is
        // INVISIBLE to the absence half — the strip errs QUIET here, the inverse of the
        // house pin's loud trade. The real threat (a conversion of `commit`) is caught
        // independently by P-1's counts, the `v1_lifecycle/mod.rs` tree digest and
        // `entry.rs`'s byte pin; the pins package's shared scan helper masks string
        // literals before stripping (the checker's `mask_non_code` idiom).
        .map(|line| line.split_once("//").map_or(line, |(kept, _)| kept))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One top-level item's text, from its signature to the first column-zero `}`.
fn body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("store/rewrite.rs no longer declares `{signature}`"));
    let rest = &source[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("`{signature}` has no column-zero close; scan unbounded"));
    &rest[..end]
}

/// Every production source under `src/`, as (path, comment-stripped text), by
/// `production_rust_files` (`check_checked_artifact_boundaries.py:974-981`)
/// extended with the `_tests.rs` stem it misses.
fn production_sources() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let (mut queue, mut sources) = (vec![root.clone()], Vec::new());
    while let Some(directory) = queue.pop() {
        for entry in std::fs::read_dir(&directory).expect("a readable production directory") {
            let path = entry.expect("a readable production entry").path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if path.is_dir() {
                if name != "tests" && name != "interface_tests" {
                    queue.push(path.clone());
                }
                continue;
            }
            if !name.ends_with(".rs") || name.starts_with("tests") || name.ends_with("_tests.rs") {
                continue;
            }
            let relative = path.strip_prefix(&root).expect("a source under the root");
            let text = std::fs::read_to_string(&path).expect("a readable production source");
            sources.push((relative.to_string_lossy().replace('\\', "/"), code(&text)));
        }
    }
    sources
}

/// The positive half: `commit` publishes by atomic in-place replacement, and
/// the latent `create_dir_all` stays bounded to `create_temporary`'s shape
/// (amendment §2's acknowledged latent, §3's P-2).
#[test]
fn the_record_root_rewrite_publishes_by_atomic_rename_and_creates_no_parent() {
    let rewrite = code(REWRITE);
    let commit = body(&rewrite, "pub(super) fn commit(");

    assert!(
        commit.contains("rename_durable(&temporary, path, true)"),
        "the record's rewrite no longer publishes by atomic in-place replacement — the carved-out \
         path of GwzM5-8R2E-RecordRootAmendment.md §2, where a detach-then-publish shape leaves a \
         window in which no shipped discovery path finds the open merge"
    );
    assert!(
        commit.contains("sync_dir(path.parent()"),
        "the rewrite no longer flushes the parent after publication; the atomic replace is \
         durable only with its barrier"
    );
    for bypass in ["fs::rename(", "fs::write(", "fs::copy("] {
        assert!(
            !commit.contains(bypass),
            "`commit` publishes through a raw std::fs writer ({bypass}); the exception carves out \
             `durable_fs::rename_durable` + `sync_dir` and nothing else in this denylist \
             (the `v1_lifecycle/mod.rs` tree digest sees every byte)"
        );
    }
    assert_eq!(
        rewrite.matches("create_dir_all").count(),
        1,
        "the rewrite path's parent-creation surface moved; row `:274`'s clause still binds"
    );
    assert!(
        body(&rewrite, "fn create_temporary(").contains("fs::create_dir_all(parent)"),
        "the one admitted `create_dir_all` — DECLINED as a refusal at E4.3-B because it is \
         structurally undrivable (no fault hook between `read_regular` and `create_temporary`, \
         so a refusal would ship unexercised) — race-only code that `read_regular` at the head of \
         `commit` proves unreached in every driven behavior — left `create_temporary`'s shape"
    );
}

/// The negative half: the checked rewrite door is absent from every production
/// source, measured against a live positive control.
#[test]
fn the_checked_rewrite_door_is_absent_from_production_sources() {
    let sources = production_sources();
    let named = |needle: &str| {
        let hit = |(_, text): &&(String, String)| text.contains(needle);
        let paths = sources.iter().filter(hit).map(|(p, _)| &**p);
        paths.collect::<BTreeSet<&str>>()
    };
    let door = format!("{DOOR_HEAD}{DOOR_TAIL}");
    assert!(
        sources.len() >= PRODUCTION_FILE_FLOOR,
        "the production scan reached {} files, under the {PRODUCTION_FILE_FLOOR} floor: a subtree \
         is unreachable, so the absence below would be an artefact of the blinding",
        sources.len()
    );
    assert_eq!(
        named(LANDED_DOOR),
        BTreeSet::from(LANDED_DOOR_FILES),
        "the positive control moved: until `{LANDED_DOOR}` is restored to its two production \
         files, or re-pinned in a reviewed commit, this scan proves nothing about the absence \
         asserted next"
    );
    assert!(
        named(&door).is_empty(),
        "production sources name the checked rewrite door `{door}` at {:?} — the rejected E4.3 \
         conversion, whose detach-then-publish window no shipped reconciler closes. \
         GwzM5-8R2E-RecordRootAmendment.md §2 must be revised, at O14's fork, before it may exist",
        named(&door)
    );
}
