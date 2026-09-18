//! Recognising regenerable data: what a tool made and the same tool
//! remakes (`GwzLaneCleanFixes.md` R5, R7; plan
//! `GwzLaneCleanFixesPlan.md` S2.1).
//!
//! # Why this exists
//!
//! The lane issues register (gwz-dev `dev-docs/GwzLaneIssues.md`, L1)
//! counted 112 hazard entries for a lane of the gwz-dev workspace. Forty-
//! eight of them are build output: 25 `__pycache__/` directories, 13
//! `CACHEDIR.TAG` caches, 7 bazel and razel output symlinks, gwz-py's
//! `_gwz_core.abi3.so` and its `src/gwz_py.egg-info/`, and one `target/`
//! an older cargo left untagged. None of it is anybody's work, and a
//! disposal that refuses over it refuses over nothing.
//!
//! # The three rules this module keeps
//!
//! 1. **Markers and shape, never a name alone.** The only names that
//!    take part at all are the ones R5 lists — `__pycache__`, `*.egg-info`,
//!    `bazel-*`, `razel-*`, the compiled-extension suffixes — and each of
//!    those still has to look the part: a `cache` directory with no valid
//!    `CACHEDIR.TAG` is not a cache. A build directory whose tool wrote no
//!    tag at all is recognised by that tool's own markers, which is plan
//!    S2.2 and is added next.
//! 2. **Nothing here consults the clone copy record** (R7). Recognition is
//!    a question about what the data *is*, asked of the bytes on disk now.
//!    A cache the lane rebuilt is still a cache, and a cache the lane
//!    created that the family never had is still a cache.
//! 3. **Unreadable is not regenerable.** Every probe that fails leaves the
//!    entry unrecognised, so it stays the lane's own data and still
//!    refuses. The recogniser proves regenerability or says nothing.
//!
//! Nothing here writes, and nothing follows a symlink: a convenience link
//! is read as a link, never as the tree it points at.

use std::path::{Component, Path, PathBuf};

/// The first bytes of a valid `CACHEDIR.TAG`, exactly as the Cache
/// Directory Tagging Specification writes them. The specification requires
/// the file to *begin* with this header, so the file's mere presence is not
/// enough and is not what is checked.
pub const CACHEDIR_TAG_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

/// The name of the tag file itself.
const CACHEDIR_TAG_NAME: &str = "CACHEDIR.TAG";

/// Filename suffixes of a compiled extension module (R5).
const EXTENSION_SUFFIXES: [&str; 3] = ["so", "pyd", "dylib"];

/// The build tools whose convenience links point out of the workspace at
/// their real output tree (R5). The name is a prefix: `bazel-bin`,
/// `bazel-out`, `razel-testlogs`.
const CONVENIENCE_LINK_PREFIXES: [(&str, &str); 2] = [("bazel-", "bazel"), ("razel-", "razel")];

/// Why one entry is regenerable. The variant is the proof, and it is what
/// a report prints: an operator who disagrees with a classification can see
/// which rule made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Regenerable {
    /// A directory holding a valid `CACHEDIR.TAG` (R5).
    TaggedCache,
    /// A `__pycache__/` holding nothing but compiled bytecode (R5).
    BytecodeCache,
    /// An `*.egg-info/` holding a setuptools metadata file (R5).
    EggInfo,
    /// A build tool's convenience symlink, pointing outside the workspace
    /// at the tool's real output tree (R5).
    ConvenienceLink { tool: &'static str },
    /// A compiled extension module inside a source tree (R5).
    CompiledExtension,
}

impl Regenerable {
    /// The rule that recognised it, in words, for a report.
    pub fn reason(self) -> String {
        match self {
            Self::TaggedCache => format!("a cache directory tagged with {CACHEDIR_TAG_NAME}"),
            Self::BytecodeCache => "a Python bytecode cache".to_owned(),
            Self::EggInfo => "setuptools package metadata".to_owned(),
            Self::ConvenienceLink { tool } => {
                format!("a {tool} convenience symlink to output outside the workspace")
            }
            Self::CompiledExtension => "a compiled extension module".to_owned(),
        }
    }
}

/// Whether the entry at `path` is regenerable, and by which rule.
///
/// `workspace` is the tree the entry belongs to: it is the boundary the
/// convenience-link rule asks about, and nothing else. `path` is the entry
/// itself, absolute or relative to the caller's own working directory, and
/// is never followed as a symlink.
pub fn recognise(workspace: &Path, path: &Path) -> Option<Regenerable> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.is_symlink() {
        return convenience_link(workspace, path);
    }
    if metadata.is_file() {
        return compiled_extension(path);
    }
    if !metadata.is_dir() {
        return None;
    }
    if has_valid_cachedir_tag(path) {
        return Some(Regenerable::TaggedCache);
    }
    if is_bytecode_cache(path) {
        return Some(Regenerable::BytecodeCache);
    }
    if is_egg_info(path) {
        return Some(Regenerable::EggInfo);
    }
    None
}

/// [`recognise`] for the entry **or any directory it lies inside**, up to
/// but not including `repository`.
///
/// An ignored build tree is reported as one entry, so the directory itself
/// is usually what is asked about. An *untracked* one is reported file by
/// file, because Git's untracked walk recurses, and a single `.pyc` inside
/// a `__pycache__` that no `.gitignore` covers is as regenerable as the
/// directory holding it. Walking the ancestors answers both with one rule.
pub fn recognise_under(workspace: &Path, repository: &Path, path: &Path) -> Option<Regenerable> {
    if let Some(found) = recognise(workspace, path) {
        return Some(found);
    }
    let mut ancestors = path.ancestors().skip(1);
    for ancestor in ancestors.by_ref() {
        if ancestor == repository || !ancestor.starts_with(repository) {
            break;
        }
        if let Some(found) = recognise(workspace, ancestor) {
            return Some(found);
        }
    }
    None
}

/// R5's convenience link: the name is a build tool's, and the link points
/// out of the workspace. A link that stays inside the workspace is not one
/// of these — it is an ordinary link the lane may well have made — and
/// neither is a link whose target cannot be read.
fn convenience_link(workspace: &Path, path: &Path) -> Option<Regenerable> {
    let name = path.file_name()?.to_str()?;
    let tool = CONVENIENCE_LINK_PREFIXES
        .iter()
        .find(|(prefix, _)| name.len() > prefix.len() && name.starts_with(prefix))
        .map(|(_, tool)| *tool)?;
    let target = std::fs::read_link(path).ok()?;
    let absolute = if target.is_absolute() {
        target
    } else {
        path.parent()?.join(target)
    };
    let inside = resolve(&absolute).starts_with(resolve(workspace));
    (!inside).then_some(Regenerable::ConvenienceLink { tool })
}

/// A path as a boundary comparison should see it: canonical where the path
/// exists, and lexically normalised where it does not, so a link to a tree
/// that has since been removed is still compared as a path and not as a
/// literal `..`.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    let mut normalised = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalised.pop();
            }
            other => normalised.push(other),
        }
    }
    normalised
}

/// R5's compiled extension module. The suffix is the whole test: these
/// entries are reported inside a repository's worktree, which is the
/// "source tree" the requirement names, and a `.so` there is output of a
/// compiler and not of a person.
fn compiled_extension(path: &Path) -> Option<Regenerable> {
    let suffix = path.extension()?.to_str()?.to_ascii_lowercase();
    EXTENSION_SUFFIXES
        .contains(&suffix.as_str())
        .then_some(Regenerable::CompiledExtension)
}

/// The Cache Directory Tagging Specification's own test: the directory
/// holds a `CACHEDIR.TAG` that **begins** with the signature line. A file
/// of that name with other contents is not a tag, and is not treated as
/// one.
fn has_valid_cachedir_tag(directory: &Path) -> bool {
    let tag = directory.join(CACHEDIR_TAG_NAME);
    let Ok(metadata) = std::fs::symlink_metadata(&tag) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() < CACHEDIR_TAG_SIGNATURE.len() as u64 {
        return false;
    }
    read_prefix(&tag, CACHEDIR_TAG_SIGNATURE.len())
        .is_some_and(|head| head == CACHEDIR_TAG_SIGNATURE)
}

/// R5's `__pycache__/`: the name CPython uses, holding nothing but the
/// compiled bytecode CPython puts there. Anything else in it — a source
/// file, a directory, something unreadable — makes it a directory that
/// merely shares the name, and it stays the lane's.
fn is_bytecode_cache(directory: &Path) -> bool {
    if directory.file_name().and_then(|name| name.to_str()) != Some("__pycache__") {
        return false;
    }
    children_all(directory, |entry| {
        entry.file_type().is_ok_and(|kind| kind.is_file())
            && entry
                .path()
                .extension()
                .and_then(|suffix| suffix.to_str())
                .is_some_and(|suffix| suffix == "pyc" || suffix == "pyo")
    })
}

/// R5's `*.egg-info/`: setuptools' own metadata directory, proved by the
/// metadata file setuptools always writes into it.
fn is_egg_info(directory: &Path) -> bool {
    let named = directory
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.len() > ".egg-info".len() && name.ends_with(".egg-info"));
    named && directory.join("PKG-INFO").is_file()
}

/// Whether every direct child of `directory` satisfies `accept`. A
/// directory that cannot be listed, or one child that cannot be read,
/// answers `false`: an unreadable entry is never regenerable.
fn children_all(directory: &Path, accept: impl Fn(&std::fs::DirEntry) -> bool) -> bool {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    for entry in entries {
        match entry {
            Ok(entry) if accept(&entry) => {}
            _ => return false,
        }
    }
    true
}

fn read_prefix(path: &Path, limit: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0u8; limit];
    let mut filled = 0;
    while filled < limit {
        match file.read(&mut buffer[filled..]).ok()? {
            0 => break,
            read => filled += read,
        }
    }
    buffer.truncate(filled);
    Some(buffer)
}
