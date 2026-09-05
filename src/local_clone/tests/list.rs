//! `local_clone::tests::list`: `gwz local list` on a real family --
//! observation-only, the root first, every member with its recorded and
//! observed state, and the observed root in `root_path` from the root and
//! from a clone alike.

use std::fs;
use std::path::{Path, PathBuf};

use gwz_copy_contract::Cancellation;

use super::fixture::{family_workspace, meta};
use crate::git::Git2Backend;
use crate::local_clone::create;
use crate::local_clone::request::validate_clone_local;
use crate::operation::NullSink;
use crate::workspace_ops::{handle_clone_local_workspace, handle_local_family, open_merge_probe};

fn clone_request(name: &str) -> crate::CloneLocalWorkspaceRequest {
    crate::CloneLocalWorkspaceRequest {
        meta: meta("req-clone-local"),
        name: name.to_owned(),
        dest: None,
        mode: crate::LocalCloneMode::Verbatim,
        branch: None,
        copy_source: None,
    }
}

fn list_request() -> crate::LocalFamilyRequest {
    crate::LocalFamilyRequest {
        meta: meta("req-local-list"),
        op: crate::LocalFamilyOp::List,
        name: None,
        keep: None,
        force_hazards: Vec::new(),
    }
}

/// (name, kind, recorded, observed, path) by wire value, the way a driver
/// renders them.
type Row = (String, i64, i64, i64, String);

fn rows(response: &crate::LocalFamilyResponse) -> Vec<Row> {
    response
        .members
        .iter()
        .map(|entry| {
            (
                entry.name.clone(),
                entry.kind.wire(),
                entry.recorded_state.wire(),
                entry.observed_state.wire(),
                entry.path.clone(),
            )
        })
        .collect()
}

fn list_from(start: &Path) -> crate::LocalFamilyResponse {
    handle_local_family(
        &Git2Backend::without_credential_helpers(),
        start,
        list_request(),
        "op-list",
        &NullSink,
    )
    .expect("list observes")
}

fn tree_digest(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut entries = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                pending.push(entry.path());
            }
            entries.push((entry.path(), metadata.len()));
        }
    }
    entries.sort();
    entries
}

struct CancelWhenExists(PathBuf);

impl Cancellation for CancelWhenExists {
    fn is_cancelled(&self) -> bool {
        self.0.exists()
    }
}

/// Design §3.1, §7, §8.1: in the family `root -> A`, `local list` from the
/// root and from A both report root then A, ready, with the root-relative
/// path and the observed root; listing writes nothing.
#[test]
fn the_family_lists_root_and_a_ready_from_the_root_and_from_the_clone() {
    let fixture = family_workspace("list-ready");
    let backend = Git2Backend::without_credential_helpers();
    handle_clone_local_workspace(
        &backend,
        &fixture.root,
        clone_request("A"),
        "op-clone",
        &NullSink,
    )
    .expect("clone A");
    let dest = fixture.sibling("A");
    let before_root = tree_digest(&fixture.root);
    let before_a = tree_digest(&dest);

    let from_root = list_from(&fixture.root);
    assert_eq!(
        rows(&from_root),
        vec![
            ("root".to_owned(), 0, 1, 0, ".".to_owned()),
            ("A".to_owned(), 0, 1, 0, "../root-A".to_owned()),
        ]
    );
    assert_eq!(
        from_root.root_path.as_deref(),
        Some(fixture.root.to_str().unwrap()),
        "the root names itself"
    );
    let from_a = list_from(&dest);
    assert_eq!(
        rows(&from_a),
        rows(&from_root),
        "the same family through A's pointer"
    );
    assert_eq!(
        from_a.root_path.as_deref(),
        Some(fixture.root.to_str().unwrap()),
        "from a clone, root_path is the index's directory reached through the pointer"
    );
    // Listing from inside a member directory still finds the family.
    let from_inside = list_from(&dest.join("app"));
    assert_eq!(rows(&from_inside), rows(&from_root));

    assert_eq!(
        tree_digest(&fixture.root),
        before_root,
        "listing writes nothing at the root"
    );
    assert_eq!(tree_digest(&dest), before_a, "listing writes nothing at A");
}

/// Design §3.1's table: a `creating` row is reported `incomplete` whatever
/// stands at its path, and a row whose directory is gone is `missing`;
/// neither is promoted, repaired or removed by listing.
#[test]
fn an_interrupted_create_lists_as_incomplete_and_a_removed_tree_as_missing() {
    let fixture = family_workspace("list-incomplete");
    let dest = fixture.sibling("A");
    let validated = validate_clone_local(&clone_request("A")).unwrap();
    create::clone_local(
        &fixture.root,
        &fixture.root,
        &validated,
        open_merge_probe,
        &CancelWhenExists(dest.join(".gwz/family-root")),
    )
    .expect_err("interrupted before the manifest");

    let listed = list_from(&fixture.root);
    assert_eq!(
        rows(&listed),
        vec![
            ("root".to_owned(), 0, 1, 0, ".".to_owned()),
            ("A".to_owned(), 0, 0, 1, "../root-A".to_owned()),
        ],
        "recorded creating, observed incomplete"
    );
    assert!(
        listed.members[1]
            .last_error
            .as_deref()
            .is_some_and(|detail| detail.contains("publish manifest")),
        "{:?}",
        listed.members[1].last_error
    );
    assert!(
        dest.join(".gwz/family-root").is_file(),
        "nothing was removed"
    );
    assert!(
        !dest.join("gwz.conf/gwz.yml").exists(),
        "nothing was promoted"
    );

    // The operator removes the tree by hand (an accepted recovery path):
    // the row stays and lists as missing until an explicit dispose.
    fs::remove_dir_all(&dest).unwrap();
    let listed = list_from(&fixture.root);
    assert_eq!(
        listed.members[1].observed_state,
        crate::LocalObservedState::Missing
    );
    assert_eq!(
        listed.members[1].recorded_state,
        crate::LocalMemberState::Creating
    );
}
