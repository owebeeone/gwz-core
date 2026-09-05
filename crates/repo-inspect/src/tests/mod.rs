//! Behavioural tests against small real repositories: architecture §4's test
//! list, design §4.0's complete hazard list, the LCM1.0c checkpoint §6/§8
//! hook-path cases deferred to this lane, and a read-only assertion over the
//! whole fixture tree.

// Fixture setup writes; the merge-path writer boundary governs production
// merge code, not test scaffolding.
#![allow(clippy::disallowed_methods)]

mod history;
mod hooks;
mod layout;
mod objects;
mod read_only;
mod work;

use std::path::Path;

use gwz_repo_contract::{LayoutError, LayoutHazard, RepoInspector, RepositoryInfo};

use crate::LocalRepoInspector;

/// The hazards `inspect_layout` reported, or a panic naming what it did
/// instead. Every §4.0 case asserts against this list rather than against a
/// single hazard, because hazards aggregate.
pub(crate) fn hazards_of<I: RepoInspector>(inspector: &I, path: &Path) -> Vec<LayoutHazard> {
    match inspector.inspect_layout(path) {
        Err(LayoutError::Unsupported {
            path: reported,
            hazards,
        }) => {
            assert_eq!(reported, path, "the refusal names the inspected path");
            hazards
        }
        other => panic!("expected an unsupported layout, got {other:?}"),
    }
}

/// The layout of an admitted repository, or a panic naming the refusal.
pub(crate) fn admitted(path: &Path) -> RepositoryInfo {
    match LocalRepoInspector::new().inspect_layout(path) {
        Ok(info) => info,
        other => panic!("expected an admitted layout, got {other:?}"),
    }
}

/// Assert that exactly one hazard of the expected shape is present, and say
/// what was found when it is not.
pub(crate) fn assert_has(hazards: &[LayoutHazard], predicate: impl Fn(&LayoutHazard) -> bool) {
    assert!(
        hazards.iter().any(predicate),
        "no matching hazard among {hazards:?}"
    );
}

pub(crate) fn assert_none(hazards: &[LayoutHazard], predicate: impl Fn(&LayoutHazard) -> bool) {
    assert!(
        !hazards.iter().any(predicate),
        "an unexpected matching hazard among {hazards:?}"
    );
}
