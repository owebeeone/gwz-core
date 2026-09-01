//! The A1 activation tripwire: how many production callers the catalog owner
//! has, pinned exactly.
//!
//! A1's coexistence gate — no production catalog activation until the R2-F
//! relocation lands — was enforced only by the ABSENCE of a caller, kept quiet
//! by an `#[allow(dead_code)]` on the owner: nothing FIRED when a caller
//! appeared. This file is that check. The gate has since been satisfied and
//! R2-E E4.1 added the first caller, so the pin is now an exact count rather
//! than a zero: a second production namer is as much a reviewed decision as the
//! first was.
//!
//! Shape: the file-set idiom of `admission/tests_namer_pin.rs` — rescan the
//! production tree for files that NAME the entry point, and subtract the
//! owner's own. Matching the bare name rather than a call site is what round
//! 1's [P1-1] bought: a module-qualified call, a fully qualified one and an
//! aliased one (`use … as rc; rc(lease)`) all put the literal name in the file,
//! while a call-site matcher sees only the spellings its prefix rules admit.
//! Inside the owner's two files the file set says nothing, so call-shaped
//! occurrences there are pinned separately; a caller aliased inside either
//! owner file stays the one uncovered shape.
//!
//! The root is exhaustive because the entry point is
//! `pub(in crate::checked_artifact)` (`catalog/bootstrap.rs:233`, re-exported at
//! `catalog.rs:35-37`) and `CatalogOwnerV1::recover_or_create` is module-private
//! — a premise ASSERTED below, not merely documented. Living under
//! `interface_tests/` keeps this file out of the scan's own production set.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Production files naming the entry point outside the owner's own two.
///
/// 2026-09-01 (R2-F R1.2): ZERO, and deliberately so — A1 forbade production
/// catalog activation until the relocation landed.
///
/// **2026-09-01 (R2-E E4.1): ONE — this step is the mover.** The relocation
/// landed (R1.1 `027da5b`), the A1 coexistence gate is satisfied, and the first
/// production catalog activation is `checked_artifact/entry.rs`'s
/// `activate_workspace_catalog`, called from the FORWARD v1 paths only —
/// `V1MutationLease::acquire_activated` (checked start/resume) and dispatch's
/// pre-upgrade viability window; the plain abort lease never activates. The pin
/// moved in that same reviewed commit, as this comment promised it would. A
/// SECOND production namer is E4.2-E4.6 work and must move this pin again,
/// deliberately.
const PRODUCTION_CALLER_COUNT: usize = 1;

/// The entry point's declaration and definition sites. Neither calls it; both
/// name it, so both are subtracted before the pin above is counted.
const OWNER_FILES: &[&str] = &["catalog.rs", "catalog/bootstrap.rs"];

/// Call-shaped occurrences inside `OWNER_FILES`: the free function, its inherent
/// method and the delegation between them (`catalog/bootstrap.rs:233,236,240`).
/// Also the anti-vacuity anchor: a blinded scan reads zero here and fails first.
const OWNER_CALL_SHAPED: usize = 3;

/// The visibility premise that makes the scan root exhaustive.
const ENTRY_POINT_DECLARATION: &str = "pub(in crate::checked_artifact) fn recover_or_create(";

/// The scan's reading of a source file: `//` comments stripped to end of line.
/// Shared with the premise assertion below, so a
/// `// was: pub(in crate::checked_artifact) fn recover_or_create(` remnant
/// cannot satisfy it after a real widening.
fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(kept, _)| kept))
        .collect()
}

/// `production_rust_files` (`check_checked_artifact_boundaries.py:829-836`),
/// extended by the `_tests.rs` stem it misses — `catalog_tests.rs`,
/// `directory_mutation_tests.rs`, `mutation_tests.rs` and `production_tests.rs`
/// are `#[cfg(test)] mod`s at `provider.rs:313-320`, two of them holding real
/// calls. The house rule is left un-extended, so this narrower "production" is
/// local to this pin. `//` comments are stripped to end of line; `/* … */`
/// blocks, inline `#[cfg(test)] mod tests` and off-convention test files are
/// scanned — all the loud direction.
fn scan(root: &Path, directory: &Path, named: &mut BTreeSet<String>, calls: &mut usize) {
    for entry in std::fs::read_dir(directory).expect("a production directory is readable") {
        let path = entry.expect("a readable production directory entry").path();
        let name = path.file_name().expect("a named entry").to_string_lossy();
        if path.is_dir() {
            if name != "tests" && name != "interface_tests" {
                scan(root, &path, named, calls);
            }
            continue;
        }
        if !name.ends_with(".rs") || name.starts_with("tests") || name.ends_with("_tests.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("a production source is readable");
        let code = strip_line_comments(&source);
        if !code.contains("recover_or_create") {
            continue;
        }
        let under_root = path.strip_prefix(root).expect("a source under the root");
        let relative = under_root.to_string_lossy().replace('\\', "/");
        if OWNER_FILES.contains(&relative.as_str()) {
            *calls += code.matches("recover_or_create(").count();
        }
        named.insert(relative);
    }
}

#[test]
fn the_catalog_owner_gains_its_first_production_caller_only_at_e4_1() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/checked_artifact");
    let (mut named, mut calls) = (BTreeSet::new(), 0);
    scan(&root, &root, &mut named, &mut calls);

    assert!(
        strip_line_comments(
            &std::fs::read_to_string(root.join("catalog/bootstrap.rs"))
                .expect("the catalog owner is readable")
        )
        .contains(ENTRY_POINT_DECLARATION),
        "the entry point is no longer declared `{ENTRY_POINT_DECLARATION}`; the root is \
         exhaustive only while it stays `pub(in crate::checked_artifact)`, since a wider one \
         admits callers outside `src/checked_artifact/`, where nothing here looks"
    );
    assert_eq!(
        calls, OWNER_CALL_SHAPED,
        "the owner's own files hold {calls} call-shaped occurrences, not {OWNER_CALL_SHAPED}. A \
         gained one is a caller inside the owner, invisible to the file set below; a lost one \
         means the thin delegate was collapsed, or the scan no longer reaches the owner at all"
    );

    let owners = OWNER_FILES.iter().copied().map(str::to_owned).collect();
    let callers = named.difference(&owners).collect::<Vec<_>>();
    assert_eq!(
        callers.len(),
        PRODUCTION_CALLER_COUNT,
        "the production files outside the owner naming `recover_or_create` — usually callers, \
         possibly mentions — are {callers:?}, which is not {PRODUCTION_CALLER_COUNT}. E4.1 added \
         the first deliberately (`entry.rs`'s `activate_workspace_catalog`); a further one is an \
         E4.2-E4.6 conversion and moves this pin in its own reviewed commit, and a LOST one means \
         production catalog activation was removed"
    );
}
