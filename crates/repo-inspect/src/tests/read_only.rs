//! The read-only assertion.
//!
//! Design §5.1: "The verifier is read-only." Architecture §4: "No implicit
//! fetch, maintenance, index rewrite or flag clearing during inspection" and
//! "Never clear live index flags to implement a read-only check."
//!
//! Every entry in the fixture tree — worktree files, the index, refs, reflogs,
//! the object store, configuration — is recorded before a full round of every
//! contract call and compared afterwards: kind, size, modification time and,
//! for a file, the hash of its bytes. A stale index refresh, a cleared flag or
//! a written object all change at least one of those.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use git2::ObjectFormat;
use gwz_repo_contract::{ObjectReader, ReadLimits, RepoInspector};

use super::admitted;
use crate::fixtures::Fixture;
use crate::{LocalObjectReader, LocalRepoInspector};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Entry {
    Directory,
    File {
        len: u64,
        modified: Option<std::time::SystemTime>,
        content: String,
    },
    Symlink {
        target: PathBuf,
    },
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Entry> {
    let mut entries = BTreeMap::new();
    walk(root, root, &mut entries);
    entries
}

fn walk(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, Entry>) {
    let listing = std::fs::read_dir(directory).expect("read directory");
    for entry in listing {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        let relative = path.strip_prefix(root).expect("relative").to_path_buf();
        let metadata = std::fs::symlink_metadata(&path).expect("metadata");
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            entries.insert(
                relative,
                Entry::Symlink {
                    target: std::fs::read_link(&path).expect("link target"),
                },
            );
        } else if file_type.is_dir() {
            entries.insert(relative, Entry::Directory);
            walk(root, &path, entries);
        } else {
            let bytes = std::fs::read(&path).expect("file bytes");
            entries.insert(
                relative,
                Entry::File {
                    len: metadata.len(),
                    modified: metadata.modified().ok(),
                    content: git2::Oid::hash_object(git2::ObjectType::Blob, &bytes)
                        .expect("hash")
                        .to_string(),
                },
            );
        }
    }
}

fn differences(before: &BTreeMap<PathBuf, Entry>, after: &BTreeMap<PathBuf, Entry>) -> Vec<String> {
    let mut changed = Vec::new();
    for (path, entry) in before {
        match after.get(path) {
            None => changed.push(format!("{} was removed", path.display())),
            Some(now) if now != entry => {
                changed.push(format!("{} changed: {entry:?} -> {now:?}", path.display()));
            }
            Some(_) => {}
        }
    }
    for path in after.keys() {
        if !before.contains_key(path) {
            changed.push(format!("{} was created", path.display()));
        }
    }
    changed
}

/// A repository with something of every kind the inspector touches: dirt in
/// the index and the worktree, ignored data, a suppression flag, a stash, an
/// annotated tag, an unfinished operation and a second branch.
fn busy_fixture(format: ObjectFormat) -> Fixture {
    let fixture = Fixture::checkout(format);
    fixture.write(".gitignore", b"build/\n");
    fixture.write("staged.txt", b"staged\n");
    fixture.write("watched.txt", b"committed\n");
    fixture.commit("setup");

    fixture.write("tracked.txt", b"about to be stashed\n");
    fixture.stash("a stash entry");
    fixture.annotated_tag("v1");
    fixture.reference("refs/gwz/local-imports/t1");

    fixture.write("staged.txt", b"staged change\n");
    fixture.stage_all();
    fixture.write("staged.txt", b"and an unstaged change on top\n");
    fixture.write("untracked.txt", b"untracked\n");
    fixture.write("build/artifact.o", b"\x00ignored\n");
    fixture.set_index_flags("watched.txt", 0x8000, 0);
    fixture.write("watched.txt", b"edited behind assume-unchanged\n");
    fixture.begin_native_operation("MERGE_HEAD");
    fixture
}

#[test]
fn a_full_round_of_inspection_changes_nothing_on_disk() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        let fixture = busy_fixture(format);
        let before = snapshot(&fixture.real_root());

        let inspector = LocalRepoInspector::new();
        let info = admitted(fixture.root());
        let work = inspector.observe_work(&info);
        let history = inspector.inventory_history(&info);
        let reader = LocalObjectReader::open(&info);
        let roots = reader.retained_roots().expect("roots");
        for root in &roots.roots {
            let _ = reader.read_object(&root.oid, &ReadLimits::default());
        }
        // Repeat the whole round: a second pass must not write either.
        let _ = inspector.inspect_layout(fixture.root());
        let _ = inspector.observe_work(&info);
        let _ = inspector.inventory_history(&info);

        let after = snapshot(&fixture.real_root());
        let changed = differences(&before, &after);
        assert!(changed.is_empty(), "{format:?}: {changed:#?}");

        // The observations really were made, so the assertion is not vacuous.
        assert!(work.known().is_some_and(|work| !work.suppressed.is_empty()));
        assert!(history.known().is_some_and(|roots| !roots.roots.is_empty()));
    }
}

#[test]
fn the_suppression_flag_is_still_set_after_inspection() {
    let fixture = busy_fixture(ObjectFormat::Sha1);
    let inspector = LocalRepoInspector::new();
    let info = admitted(fixture.root());
    let _ = inspector.observe_work(&info);
    let _ = inspector.inventory_history(&info);

    let repository = fixture.open();
    let index = repository.index().expect("index");
    let entry = index
        .get_path(Path::new("watched.txt"), 0)
        .expect("the entry survives");
    assert_ne!(
        entry.flags & 0x8000,
        0,
        "assume-unchanged must not be cleared by a read-only check"
    );
}
