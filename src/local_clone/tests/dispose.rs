//! `local_clone::tests::dispose`: `gwz local dispose <name> --keep` and
//! `gwz local disband` on a real family, ordinary deletion still refusing,
//! and the disposal ports exercised directly against real repositories.

use std::fs;
use std::path::{Path, PathBuf};

use gwz_copy_contract::Cancellation;
use gwz_family_model::{MarkerObservation, PointerObservation, TargetObservation};
use gwz_family_store_contract::{FamilyLocation, FamilyObservation, FamilyStore};
use gwz_local_disposal::{DisposalPorts, DisposeEffect, HistoryAnswer, HistoryQuery};
use gwz_repo_contract::{Observation, RepoKey, WorkKind};

use super::fixture::{family_workspace, meta};
use crate::git::Git2Backend;
use crate::local_clone::adapters::disposal::CoreDisposalPorts;
use crate::local_clone::create;
use crate::local_clone::request::validate_clone_local;
use crate::model::ErrorCode;
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

fn family_request(
    op: crate::LocalFamilyOp,
    name: Option<&str>,
    keep: Option<bool>,
) -> crate::LocalFamilyRequest {
    crate::LocalFamilyRequest {
        meta: meta("req-local-family"),
        op,
        name: name.map(ToOwned::to_owned),
        keep,
        force_hazards: Vec::new(),
    }
}

fn clone(root: &Path, name: &str) {
    handle_clone_local_workspace(
        &Git2Backend::without_credential_helpers(),
        root,
        clone_request(name),
        "op-clone",
        &NullSink,
    )
    .expect("clone");
}

fn local(start: &Path, request: crate::LocalFamilyRequest) -> crate::LocalFamilyResponse {
    handle_local_family(
        &Git2Backend::without_credential_helpers(),
        start,
        request,
        "op-local",
        &NullSink,
    )
    .expect("local family operation")
}

fn family(root: &Path) -> Option<gwz_family_model::FamilyView> {
    match gwz_family_store::YamlFamilyStore::new()
        .read_view(&FamilyLocation::new(root))
        .expect("the family reads")
    {
        FamilyObservation::Family { view, .. } => Some(view),
        FamilyObservation::NoFamily => None,
    }
}

/// Every file under `root` with its bytes, except the two family metadata
/// files a detach removes.
fn files_except_family_metadata(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                let relative = path.strip_prefix(root).unwrap();
                if relative == Path::new(gwz_family_model::POINTER_RELATIVE_PATH)
                    || relative == Path::new(gwz_family_model::ALLOCATION_MARKER_RELATIVE_PATH)
                {
                    continue;
                }
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

struct CancelWhenExists(PathBuf);

impl Cancellation for CancelWhenExists {
    fn is_cancelled(&self) -> bool {
        self.0.exists()
    }
}

/// Design §5.2 step 2: `--keep` removes the matching pointer and row and
/// nothing else; every other file stays byte for byte, and the lane is no
/// longer a member.
#[test]
fn keep_detaches_a_and_leaves_every_file() {
    let fixture = family_workspace("dispose-keep");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let before = files_except_family_metadata(&dest);
    assert!(!before.is_empty());

    let response = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), Some(true)),
    );
    let message = response.response.meta.message.expect("a message");
    assert!(message.contains("detached local clone `A`"), "{message}");
    assert!(message.contains("every file at"), "{message}");
    assert!(response.members.is_empty() && response.root_path.is_none());

    assert!(
        !dest.join(".gwz/family-root").exists(),
        "the pointer is gone"
    );
    assert!(
        !dest.join(".gwz/local-clone-allocation").exists(),
        "the marker is gone"
    );
    assert_eq!(
        files_except_family_metadata(&dest),
        before,
        "every other file is retained byte for byte"
    );
    let view = family(&fixture.root).expect("the family stands");
    assert!(view.members.is_empty(), "the row is gone");
    let listed = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::List, None, None),
    );
    assert_eq!(listed.members.len(), 1, "only the root lists");
    // A detached tree is in no family: listing from it is empty.
    let from_a = local(
        &dest,
        family_request(crate::LocalFamilyOp::List, None, None),
    );
    assert!(from_a.members.is_empty() && from_a.root_path.is_none());
    // A second keep has nothing to detach.
    let error = handle_local_family(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), Some(true)),
        "op-again",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MemberNotFound, "{}", error.message);
}

/// Design §3.1, §12 ("Dirty/open/incomplete lane with keep: detach matching
/// metadata and retain all remaining files"): keep works on an interrupted
/// create and repairs nothing.
#[test]
fn keep_detaches_an_incomplete_target_and_retains_its_remainder() {
    let fixture = family_workspace("dispose-keep-incomplete");
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
    let before = files_except_family_metadata(&dest);

    let response = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), Some(true)),
    );
    assert!(
        response
            .response
            .meta
            .message
            .as_deref()
            .is_some_and(|message| message.contains("detached local clone `A`"))
    );
    assert!(family(&fixture.root).unwrap().members.is_empty());
    assert!(!dest.join(".gwz/family-root").exists());
    assert_eq!(files_except_family_metadata(&dest), before);
    assert!(
        !dest.join("gwz.conf/gwz.yml").exists(),
        "keep does not repair the tree: no manifest appears"
    );
}

/// Design §3.1, §8.6, §12 ("Disband after partial pointer removal"):
/// disband removes every pointer and marker and finally the index, leaves
/// every tree, works from a clone through its pointer, and is a no-op when
/// repeated.
#[test]
fn disband_removes_pointers_and_the_index_and_leaves_the_trees() {
    let fixture = family_workspace("dispose-disband");
    clone(&fixture.root, "A");
    clone(&fixture.root, "B");
    let a = fixture.sibling("A");
    let b = fixture.sibling("B");
    let before_root = files_except_family_metadata(&fixture.root)
        .into_iter()
        .filter(|(path, _)| !path.ends_with("local-family.yml"))
        .collect::<Vec<_>>();
    let before_a = files_except_family_metadata(&a);
    let before_b = files_except_family_metadata(&b);
    // An interrupted earlier disband: A's pointer is already gone.
    fs::remove_file(a.join(".gwz/family-root")).unwrap();

    let response = local(
        &b,
        family_request(crate::LocalFamilyOp::Disband, None, None),
    );
    let message = response.response.meta.message.expect("a message");
    assert!(message.contains("disbanded local family"), "{message}");
    assert!(
        message.contains("1 pointer(s) and 2 marker(s) removed"),
        "{message}"
    );
    assert!(message.contains("index removed"), "{message}");

    assert!(family(&fixture.root).is_none(), "the index is gone");
    assert!(!fixture.root.join(".gwz/local-family.yml").exists());
    for (dest, before) in [(&a, &before_a), (&b, &before_b)] {
        assert!(!dest.join(".gwz/family-root").exists());
        assert!(!dest.join(".gwz/local-clone-allocation").exists());
        assert_eq!(
            &files_except_family_metadata(dest),
            before,
            "{}",
            dest.display()
        );
    }
    assert_eq!(
        files_except_family_metadata(&fixture.root)
            .into_iter()
            .filter(|(path, _)| !path.ends_with("local-family.yml"))
            .collect::<Vec<_>>(),
        before_root,
        "the root's own files are retained"
    );
    let again = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Disband, None, None),
    );
    assert_eq!(
        again.response.meta.aggregate_status,
        crate::AggregateStatus::Noop,
        "an explicit repeat is a no-op"
    );
}

/// Ordinary deletion still refuses -- after the family observation, before
/// any effect (LCM2.1 owns its fresh checks); `--keep` with a hazard list is
/// malformed.
#[test]
fn ordinary_deletion_still_refuses_without_effect() {
    let fixture = family_workspace("dispose-ordinary");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let before = files_except_family_metadata(&dest);
    let error = handle_local_family(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), None),
        "op-delete",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::UnsupportedOperation,
        "{}",
        error.message
    );
    assert!(error.message.contains("local dispose"), "{}", error.message);
    assert_eq!(files_except_family_metadata(&dest), before);
    assert!(dest.join(".gwz/family-root").is_file());
    let view = family(&fixture.root).unwrap();
    assert!(view.member("A").is_some(), "the row stands");
}

/// The disposal ports against real repositories: `observe_target` reads the
/// target's metadata through the store and every included repository
/// through the inspector; `check_history` asks `gwz-history-check` once per
/// witness store and combines; `remove_directory` removes a tree once.
#[test]
fn the_disposal_ports_observe_check_history_per_witness_and_remove() {
    let fixture = family_workspace("dispose-ports");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let view = family(&fixture.root).unwrap();
    let name = gwz_family_model::MemberName::parse("A").unwrap();
    let mut ports = CoreDisposalPorts::new(
        fixture.root.clone(),
        view.clone(),
        name.clone(),
        open_merge_probe,
    );

    let evidence = ports.observe_target(&dest).expect("observed");
    assert_eq!(
        evidence.target,
        TargetObservation::Present {
            pointer: PointerObservation::Matches,
            marker: MarkerObservation::Matches,
        }
    );
    assert!(evidence.unknown.is_empty(), "{:?}", evidence.unknown);
    let keys: Vec<&RepoKey> = evidence
        .repositories
        .iter()
        .map(|repository| &repository.key)
        .collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert_eq!(keys[0], &RepoKey::Root);
    assert!(
        matches!(keys[1], RepoKey::Member { id } if id.starts_with("mem_")),
        "{keys:?}"
    );
    let app = &evidence.repositories[1];
    let work = app.work.known().expect("the member's work is known");
    assert!(
        work.entries
            .iter()
            .any(|entry| entry.kind == WorkKind::Untracked && entry.path == b"notes.txt"),
        "the copied untracked note is observed: {:?}",
        work.entries
    );
    let root_repo = &evidence.repositories[0];
    assert!(root_repo.gwz.merge == gwz_work_detector::EvidenceState::None);
    let protected = match &root_repo.history {
        Observation::Known(protected) => protected.clone(),
        Observation::Unknown(reasons) => panic!("history unknown: {reasons:?}"),
    };
    assert!(protected.is_complete() && !protected.roots.is_empty());

    // The source root's repository is the witness paired with @root, and it
    // holds every object A's protected roots reach: preserved.
    let query = HistoryQuery {
        target: RepoKey::Root,
        protected: protected.clone(),
    };
    assert_eq!(ports.check_history(&query), HistoryAnswer::Preserved);

    // A commit made only in A is reached by no surviving witness root.
    let repository = git2::Repository::open(&dest).unwrap();
    let signature = gwz_local_testrepo::fixture_signature();
    let tree_id = {
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("README")).unwrap();
        index.write().unwrap();
        index.write_tree().unwrap()
    };
    let tree = repository.find_tree(tree_id).unwrap();
    let parent = repository.head().unwrap().peel_to_commit().unwrap();
    let only_in_a = repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "only in A",
            &tree,
            &[&parent],
        )
        .unwrap();
    drop(tree);
    drop(parent);
    drop(repository);
    let evidence = ports.observe_target(&dest).expect("observed again");
    let protected = evidence.repositories[0]
        .history
        .known()
        .cloned()
        .expect("known");
    match ports.check_history(&HistoryQuery {
        target: RepoKey::Root,
        protected,
    }) {
        HistoryAnswer::Unpreserved { detail } => {
            assert!(detail.contains(&only_in_a.to_string()), "{detail}");
        }
        other => panic!("a commit only in A is unpreserved, got {other:?}"),
    }
    // A nested repository has no paired witness.
    assert!(matches!(
        ports.check_history(&HistoryQuery {
            target: RepoKey::Member {
                id: "nested:vendor/thing".to_owned()
            },
            protected: gwz_repo_contract::ProtectedRoots::default(),
        }),
        HistoryAnswer::Unpreserved { .. }
    ));

    // The remover, on a scratch tree beside the family (never the clone).
    let scratch = fixture.tree.dir("scratch/deep");
    fs::write(scratch.join("file"), b"x").unwrap();
    ports
        .remove_directory(&fixture.tree.join("scratch"))
        .expect("removed");
    assert!(!fixture.tree.join("scratch").exists());
    assert!(
        dest.join(".gwz/family-root").is_file(),
        "the clone is untouched"
    );
    let _ = DisposeEffect::RowDetached;
}
