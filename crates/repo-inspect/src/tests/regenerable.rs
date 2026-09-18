//! The regenerable recogniser, one test per rule (`GwzLaneCleanFixes.md`
//! R5, R7; plan `GwzLaneCleanFixesPlan.md` S2.1, and R17's share
//! of S2.4).
//!
//! Every test builds the real shape on disk and asks the recogniser, so
//! what is proved is the marker and not a name.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::regenerable::{CACHEDIR_TAG_SIGNATURE, Regenerable, recognise, recognise_under};

/// A workspace to recognise inside, plus a tree outside it for the
/// convenience-link rule to point at.
struct Tree {
    _temporary: TempDir,
    workspace: PathBuf,
    /// Only the convenience-link rule looks outside the workspace, and
    /// symlinks are only made here on Unix.
    #[allow(dead_code)]
    outside: PathBuf,
}

impl Tree {
    fn new() -> Self {
        let temporary = TempDir::new().expect("a temporary directory");
        let workspace = temporary.path().join("workspace");
        let outside = temporary.path().join("outside");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        Self {
            _temporary: temporary,
            workspace,
            outside,
        }
    }

    fn directory(&self, relative: &str) -> PathBuf {
        let path = self.workspace.join(relative);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.workspace.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn ask(&self, path: &Path) -> Option<Regenerable> {
        recognise(&self.workspace, path)
    }
}

/// A valid tag file's bytes: the signature line, then the explanatory text
/// the specification suggests.
fn tag_bytes() -> Vec<u8> {
    let mut bytes = CACHEDIR_TAG_SIGNATURE.to_vec();
    bytes.extend_from_slice(b"\n# This file is a cache directory tag.\n");
    bytes
}

/// R5: a directory holding a **valid** `CACHEDIR.TAG` is a cache, and the
/// file's mere presence is not the test — a `CACHEDIR.TAG` that does not
/// begin with the signature is not a tag.
#[test]
fn a_cache_is_recognised_by_a_valid_tag_and_never_by_the_file_alone() {
    let tree = Tree::new();
    let cache = tree.directory("target");
    assert_eq!(tree.ask(&cache), None, "an untagged, unmarked directory");

    tree.file("target/CACHEDIR.TAG", b"Signature: not the right one\n");
    assert_eq!(
        tree.ask(&cache),
        None,
        "a file of the right name is not a tag"
    );

    tree.file("target/CACHEDIR.TAG", &tag_bytes());
    assert_eq!(tree.ask(&cache), Some(Regenerable::TaggedCache));

    // The name plays no part: the same tag anywhere is a cache.
    let elsewhere = tree.directory("docs/examples");
    tree.file("docs/examples/CACHEDIR.TAG", &tag_bytes());
    assert_eq!(tree.ask(&elsewhere), Some(Regenerable::TaggedCache));
}

/// R5: `__pycache__/` holding compiled bytecode, and only that.
#[test]
fn a_bytecode_cache_is_recognised_and_a_namesake_holding_sources_is_not() {
    let tree = Tree::new();
    let cache = tree.directory("pkg/__pycache__");
    assert_eq!(tree.ask(&cache), Some(Regenerable::BytecodeCache), "empty");

    tree.file("pkg/__pycache__/module.cpython-313.pyc", b"\x00pyc");
    tree.file("pkg/__pycache__/other.cpython-313.pyo", b"\x00pyo");
    assert_eq!(tree.ask(&cache), Some(Regenerable::BytecodeCache));

    tree.file("pkg/__pycache__/notes.txt", b"someone's note\n");
    assert_eq!(
        tree.ask(&cache),
        None,
        "a directory of that name holding anything else is the lane's"
    );

    let named = tree.directory("pkg/pycache");
    tree.file("pkg/pycache/module.cpython-313.pyc", b"\x00pyc");
    assert_eq!(
        tree.ask(&named),
        None,
        "and the name is required, not enough"
    );
}

/// R5: `*.egg-info/`, proved by the metadata setuptools writes into it.
#[test]
fn an_egg_info_is_recognised_by_its_metadata() {
    let tree = Tree::new();
    let info = tree.directory("src/gwz_py.egg-info");
    assert_eq!(tree.ask(&info), None, "the suffix alone proves nothing");

    tree.file("src/gwz_py.egg-info/PKG-INFO", b"Metadata-Version: 2.1\n");
    assert_eq!(tree.ask(&info), Some(Regenerable::EggInfo));

    let suffix_only = tree.directory("src/.egg-info");
    tree.file("src/.egg-info/PKG-INFO", b"Metadata-Version: 2.1\n");
    assert_eq!(
        tree.ask(&suffix_only),
        None,
        "`.egg-info` with no package name is not setuptools' own spelling"
    );
}

/// The convenience-link rule, where symlinks are made without ceremony.
/// The whole platform-specific part of these tests is inside this module,
/// so no conditional attribute rides on a bare declaration.
#[cfg(unix)]
mod links {
    use super::*;

    /// R5: a build tool's convenience symlink, pointing **outside** the
    /// workspace. A link of the same name that stays inside is the lane's.
    #[test]
    fn a_convenience_link_is_recognised_only_when_it_leaves_the_workspace() {
        let tree = Tree::new();
        std::fs::create_dir_all(tree.outside.join("bazel-out-base")).unwrap();
        let out = tree.workspace.join("bazel-out");
        std::os::unix::fs::symlink(tree.outside.join("bazel-out-base"), &out).unwrap();
        assert_eq!(
            tree.ask(&out),
            Some(Regenerable::ConvenienceLink { tool: "bazel" })
        );

        let razel = tree.workspace.join("razel-testlogs");
        std::os::unix::fs::symlink(tree.outside.join("razel-base"), &razel).unwrap();
        assert_eq!(
            tree.ask(&razel),
            Some(Regenerable::ConvenienceLink { tool: "razel" }),
            "a target that no longer exists is still outside the workspace"
        );

        tree.directory("real-out");
        let inside = tree.workspace.join("bazel-inside");
        std::os::unix::fs::symlink(tree.workspace.join("real-out"), &inside).unwrap();
        assert_eq!(
            tree.ask(&inside),
            None,
            "a link into the workspace is data the lane may hold alone"
        );

        let other = tree.workspace.join("notes-link");
        std::os::unix::fs::symlink(tree.outside.join("notes.txt"), &other).unwrap();
        assert_eq!(tree.ask(&other), None, "and the name is a build tool's");
    }
}

/// R5: compiled extension modules, by suffix, and nothing else.
#[test]
fn a_compiled_extension_is_recognised_by_its_suffix() {
    let tree = Tree::new();
    for name in ["_gwz_core.abi3.so", "_native.pyd", "_native.dylib"] {
        let path = tree.file(&format!("src/{name}"), b"\x7fELF");
        assert_eq!(
            tree.ask(&path),
            Some(Regenerable::CompiledExtension),
            "{name}"
        );
    }
    let source = tree.file("src/native.rs", b"fn main() {}\n");
    assert_eq!(tree.ask(&source), None);
    let directory = tree.directory("src/weird.so");
    assert_eq!(
        tree.ask(&directory),
        None,
        "a directory of that name is not a module"
    );
}

/// R7: recognition is a question about the data, not about its history.
/// The recogniser reads no record and takes no baseline, so a cache the
/// lane rebuilt, and a cache the lane created that never existed anywhere
/// else, are both still caches.
#[test]
fn a_rebuilt_or_lane_made_cache_is_still_a_cache() {
    let tree = Tree::new();
    let cache = tree.directory("target");
    tree.file("target/CACHEDIR.TAG", &tag_bytes());
    tree.file("target/debug/build.bin", b"built once\n");
    assert_eq!(tree.ask(&cache), Some(Regenerable::TaggedCache));
    tree.file("target/debug/build.bin", b"built again, in the lane\n");
    tree.file("target/debug/fresh.bin", b"made only in the lane\n");
    assert_eq!(
        tree.ask(&cache),
        Some(Regenerable::TaggedCache),
        "a rebuilt cache is still a cache"
    );

    let only_here = tree.directory("new-cache");
    tree.file("new-cache/CACHEDIR.TAG", &tag_bytes());
    assert_eq!(
        tree.ask(&only_here),
        Some(Regenerable::TaggedCache),
        "and so is one the lane made from nothing"
    );
}

/// An untracked file is reported file by file, so the rule has to reach
/// the directory above it — and it stops at the repository, which is never
/// itself regenerable however it is furnished.
#[test]
fn an_entry_inside_a_regenerable_directory_is_regenerable() {
    let tree = Tree::new();
    let repository = tree.directory("repo");
    tree.file("repo/pkg/__pycache__/module.cpython-313.pyc", b"\x00pyc");
    let inside = repository.join("pkg/__pycache__/module.cpython-313.pyc");
    assert_eq!(recognise(&tree.workspace, &inside), None, "the file alone");
    assert_eq!(
        recognise_under(&tree.workspace, &repository, &inside),
        Some(Regenerable::BytecodeCache)
    );

    let plain = tree.file("repo/pkg/module.py", b"x = 1\n");
    assert_eq!(
        recognise_under(&tree.workspace, &repository, &plain),
        None,
        "nothing above it is regenerable"
    );

    // A repository whose own root carries a marker is still a repository.
    tree.file("repo/CACHEDIR.TAG", &tag_bytes());
    assert_eq!(
        recognise_under(&tree.workspace, &repository, &plain),
        None,
        "the walk stops below the repository root"
    );
}

/// Nothing regenerable is claimed about what cannot be read.
#[test]
fn an_absent_entry_is_not_regenerable() {
    let tree = Tree::new();
    assert_eq!(tree.ask(&tree.workspace.join("gone")), None);
    assert_eq!(
        recognise_under(
            &tree.workspace,
            &tree.workspace,
            &tree.workspace.join("gone/deeper")
        ),
        None
    );
}
