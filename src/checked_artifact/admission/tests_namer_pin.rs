//! Namer pin for the R1 admission classifier's widened visibility.
//!
//! Round-2 remediation of the settled State-axis [P3-3]. R2-D Phase 1 widened
//! `ObservedActionDirectoryV1{,::exact}` and
//! `CatalogAdmissionOwnerV1::{new,classify_handoff,admit}` from `pub(super)` to
//! `pub(in crate::checked_artifact)` — the minimum Rust visibility that lets the
//! physical half consume the frozen classifier, and ratified as such on both
//! review axes. The accepted cost is that any `checked_artifact`-internal
//! sibling could now synthesize observations and mint an `AdmittedActionV1`
//! without touching a filesystem. Nothing pinned the set of modules that do.
//!
//! This is that pin, in the `FAULT_INJECTION_SOURCES` completeness-anchor idiom
//! (`interface_tests/fault_expected_keys.rs`): it rescans the production tree
//! rather than trusting a declared list, so a new namer is a deliberate,
//! reviewed edit to this file rather than a silent widening. The classifier is
//! `pub(in crate::checked_artifact)`, so no namer can exist outside
//! `src/checked_artifact/` and scanning that subtree is exhaustive.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! (`scripts/checks/check_checked_artifact_boundaries.py`), so the scan's own
//! mention of the pinned names cannot make it a namer of itself.
//!
//! The canonical long-term home for a pin of this shape is
//! `interface_tests/`; it is written here because R2-D Phase 1's round-2
//! remediation does not own that tree. Relocating it is a lane-owner decision
//! and changes nothing about the property.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Every production module allowed to name the admission classifier or to
/// construct the observations it judges.
///
/// The first two are what the widening exists for; the rest are the
/// classifier's own module tree, which could name it at `pub(super)` and is
/// unaffected by the widening.
///
/// * `admission/driver.rs` — the physical half: it consumes `classify_handoff`
///   and `admit` as the sole deciders of staging/final and of issuance.
/// * `capability/pre_catalog/provider/interior.rs` — the observer that builds
///   `ObservedActionDirectoryV1` from a real directory, inside the sealed
///   provider owner.
/// * `protocol/admission.rs` — declares and re-exports the classifier.
/// * `protocol/admission/owner.rs` — defines it.
/// * `protocol/admission/test_support.rs` — the classifier's own `#[cfg(test)]`
///   fixture, inside the same module tree; it holds no production path.
const DECLARED_CLASSIFIER_NAMERS: &[&str] = &[
    "admission/driver.rs",
    "capability/pre_catalog/provider/interior.rs",
    "protocol/admission.rs",
    "protocol/admission/owner.rs",
    "protocol/admission/test_support.rs",
];

/// The widened items. Naming any of them from production code is what the pin
/// governs; `AdmittedActionV1`'s fields stay module-private, so the handoff
/// itself still cannot be forged outside `protocol/admission`.
const PINNED_CLASSIFIER_ITEMS: &[&str] = &["CatalogAdmissionOwnerV1", "ObservedActionDirectoryV1"];

fn production_sources_naming_the_classifier(root: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("a production directory is readable") {
            let path = entry
                .expect("a production directory entry is readable")
                .path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("source tree names are UTF-8")
                .to_owned();
            if path.is_dir() {
                if name != "tests" && name != "interface_tests" {
                    pending.push(path);
                }
                continue;
            }
            if !name.ends_with(".rs") || name.starts_with("tests") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("a production source is readable");
            if PINNED_CLASSIFIER_ITEMS
                .iter()
                .any(|item| source.contains(item))
            {
                found.insert(relative_slash_path(root, &path));
            }
        }
    }
    found
}

fn relative_slash_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("a scanned source lies under the scan root")
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .expect("source tree names are UTF-8")
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn only_the_declared_production_modules_name_the_admission_classifier() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/checked_artifact");
    let scanned = production_sources_naming_the_classifier(&root);
    let declared = DECLARED_CLASSIFIER_NAMERS
        .iter()
        .map(|relative| (*relative).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        scanned, declared,
        "the admission classifier's production namer set drifted: `CatalogAdmissionOwnerV1` and \
         `ObservedActionDirectoryV1` are `pub(in crate::checked_artifact)`, so any internal \
         sibling can synthesize observations and mint an `AdmittedActionV1`. A new namer must be \
         a reviewed edit to DECLARED_CLASSIFIER_NAMERS, not a side effect"
    );
}
