//! Which worktree entries are in an update's way (git's `verify_absent`, judged by
//! the filesystem, plus libgit2's own dry run). Nothing here deletes anything.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::{OsStr, OsString};

use git2::Binding;
use unicode_normalization::UnicodeNormalization;

use super::*;

/// How an entry found in an update's way relates to the index.
enum Tracked {
    /// A tracked entry the update removes: the checkout itself replaces it.
    Removed,
    /// A tracked entry the update keeps (the index path it is).
    Kept(String),
    /// Not an index path: untracked.
    No,
}

cfg_if::cfg_if! {
    if #[cfg(target_os = "macos")] {
        /// libgit2 composes the names it reads from the worktree to NFC before it
        /// matches them to the index when `core.precomposeunicode` is set. It can do
        /// that only on macOS.
        fn precomposes(repo: &git2::Repository) -> bool {
            repo.config()
                .and_then(|config| config.get_bool("core.precomposeunicode"))
                .unwrap_or(false)
        }
    } else {
        fn precomposes(_repo: &git2::Repository) -> bool {
            false
        }
    }
}

fn nfc(name: &str) -> String {
    name.nfc().collect()
}

/// A name with case and normalization folded: two names a filesystem may treat as one
/// entry have the same fold (necessary, not sufficient: the listing decides).
fn fold(name: &str) -> String {
    nfc(name).to_lowercase()
}

/// What the collision rule reads from one repository's worktree for one update.
struct Worktree<'r> {
    root: &'r Path,
    /// The index paths (pull runs only without tracked changes, so HEAD's).
    tracked: HashSet<String>,
    /// The index paths the update removes (HEAD has them, the target does not).
    removed: HashSet<String>,
    /// Under `core.ignorecase` on a filesystem that does not fold case, libgit2
    /// still matches paths with ASCII case folded (`git__strcasecmp`), so it can
    /// replace a tracked entry the filesystem keeps apart from the incoming path:
    /// the index spellings (entries and their directories) by folded path.
    folded_spellings: Option<HashMap<String, String>>,
    /// `core.ignorecase`: libgit2 then matches worktree names to the index with
    /// ASCII case folded.
    ignore_case: bool,
    /// `core.precomposeunicode` on macOS: libgit2 matches NFC-composed names.
    precompose: bool,
    /// Parent paths already resolved as directories, and whether each may be
    /// written into (`check_spelling`).
    directories: HashMap<String, Result<(), String>>,
    /// The names in each directory listed so far (by its path as given).
    listings: HashMap<String, Result<HashSet<OsString>, String>>,
}

impl<'r> Worktree<'r> {
    fn open(
        repo: &git2::Repository,
        root: &'r Path,
        removed: HashSet<String>,
    ) -> ModelResult<Self> {
        let index = repo.index().map_err(git_error)?;
        let tracked = index
            .iter()
            .map(|entry| String::from_utf8_lossy(&entry.path).into_owned())
            .collect::<HashSet<_>>();
        let ignore_case = repo
            .config()
            .and_then(|config| config.get_bool("core.ignorecase"))
            .unwrap_or(false);
        // `.git` resolves as `.GIT` exactly where the filesystem folds ASCII case.
        let folds = std::fs::symlink_metadata(root.join(".GIT")).is_ok();
        let folded_spellings = (ignore_case && !folds).then(|| {
            let mut spellings = HashMap::new();
            for path in &tracked {
                let mut end = path.len();
                loop {
                    let spelled = &path[..end];
                    spellings
                        .entry(spelled.to_ascii_lowercase())
                        .or_insert_with(|| spelled.to_owned());
                    let Some(slash) = spelled.rfind('/') else {
                        break;
                    };
                    end = slash;
                }
            }
            spellings
        });
        Ok(Self {
            root,
            tracked,
            removed,
            folded_spellings,
            ignore_case,
            precompose: precomposes(repo),
            directories: HashMap::new(),
            listings: HashMap::new(),
        })
    }

    /// The entry the filesystem resolves for `given` (`None` when there is none), as
    /// spelled to reach it; under the folded spellings, the tracked entry libgit2
    /// would match when the filesystem resolves nothing.
    fn resolve(&self, given: &str) -> Result<Option<(String, std::fs::Metadata)>, String> {
        let lookup = |spelled: &str| match std::fs::symlink_metadata(self.root.join(spelled)) {
            Ok(meta) => Ok(Some((spelled.to_owned(), meta))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(uninspectable(spelled, &error)),
        };
        if let Some(found) = lookup(given)? {
            return Ok(Some(found));
        }
        match self
            .folded_spellings
            .as_ref()
            .and_then(|spellings| spellings.get(&given.to_ascii_lowercase()))
        {
            Some(spelled) if spelled != given => lookup(spelled),
            _ => Ok(None),
        }
    }

    /// The entry in the way of writing `incoming`, as spelled to reach it: a
    /// non-directory at one of its parent paths, else whatever is at the path
    /// itself; `None` when nothing is; an error when that cannot be told.
    fn blocker(&mut self, incoming: &str) -> Result<Option<String>, String> {
        let parts = incoming.split('/').collect::<Vec<_>>();
        for depth in 1..=parts.len() {
            let given = parts[..depth].join("/");
            let leaf = depth == parts.len();
            if let Some(checked) = self.directories.get(&given).filter(|_| !leaf) {
                checked.clone()?;
                continue;
            }
            let Some((spelled, meta)) = self.resolve(&given)? else {
                return Ok(None);
            };
            if leaf || !meta.is_dir() {
                return Ok(Some(spelled));
            }
            let checked = if spelled == given {
                self.check_spelling(&given, meta)
            } else {
                Ok(())
            };
            self.directories.insert(given, checked.clone());
            checked?;
        }
        Ok(None)
    }

    /// The names in the directory `directory` (relative to the root, `""` for the
    /// root itself), listed once.
    fn listing(&mut self, directory: &str) -> Result<&HashSet<OsString>, String> {
        let root = self.root;
        self.listings
            .entry(directory.to_owned())
            .or_insert_with(|| {
                std::fs::read_dir(root.join(directory))
                    .and_then(|entries| {
                        entries
                            .map(|entry| entry.map(|entry| entry.file_name()))
                            .collect::<std::io::Result<HashSet<_>>>()
                    })
                    .map_err(|error| {
                        uninspectable(if directory.is_empty() { "." } else { directory }, &error)
                    })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// `given` as the directories spell it: each component by its exact name, else by
    /// the one name whose case and normalization fold to the same (the filesystem
    /// resolved it, so it is that entry). `None` when a component is not listed by
    /// either, in which case nothing about the entry is proven.
    fn on_disk(&mut self, given: &str) -> Result<Option<String>, String> {
        let mut spelled = String::new();
        for component in given.split('/') {
            let names = self.listing(&spelled)?;
            let name = if names.contains(OsStr::new(component)) {
                component.to_owned()
            } else {
                let folded = fold(component);
                let mut same = names
                    .iter()
                    .filter_map(|listed| listed.to_str())
                    .filter(|listed| fold(listed) == folded);
                match (same.next(), same.next()) {
                    (Some(only), None) => only.to_owned(),
                    _ => return Ok(None),
                }
            };
            if !spelled.is_empty() {
                spelled.push('/');
            }
            spelled.push_str(&name);
        }
        Ok(Some(spelled))
    }

    /// libgit2 writes an incoming path into the directory the filesystem resolves
    /// for its parent, then matches the worktree to the index by name (ASCII case
    /// folded under `core.ignorecase`, NFC composed under `core.precomposeunicode`,
    /// nothing more). A parent directory the filesystem reaches under another
    /// spelling (a non-ASCII case variant, another normalization) would make the
    /// update look like a tracked change after it moved: it refuses, unless the
    /// update empties it (libgit2 then removes it and creates the incoming spelling).
    fn check_spelling(&mut self, given: &str, meta: std::fs::Metadata) -> Result<(), String> {
        let (parent, name) = given.rsplit_once('/').unwrap_or(("", given));
        let (ignore_case, precompose) = (self.ignore_case, self.precompose);
        let names = self.listing(parent)?;
        if names.contains(OsStr::new(name))
            || names
                .iter()
                .filter_map(|listed| listed.to_str())
                .any(|listed| {
                    (ignore_case && listed.eq_ignore_ascii_case(name))
                        || (precompose && nfc(listed) == name)
                })
        {
            return Ok(());
        }
        let disk = self.on_disk(given)?.unwrap_or_else(|| given.to_owned());
        if !walk_blocker(self, given, &disk, &meta)?.untracked {
            return Ok(());
        }
        Err(format!(
            "the directory '{given}' is spelled otherwise in the worktree (beyond ASCII case), \
             so the checkout cannot write into it; rename it to '{given}' or move its content \
             first"
        ))
    }

    /// The index path libgit2 matches an entry spelled `disk` on disk to: the exact
    /// path, else (macOS, `core.precomposeunicode`) its NFC composition.
    fn index_path(&self, disk: &str) -> Option<String> {
        if self.tracked.contains(disk) {
            return Some(disk.to_owned());
        }
        if self.precompose {
            let composed = nfc(disk);
            if self.tracked.contains(&composed) {
                return Some(composed);
            }
        }
        None
    }

    /// Whether the entry spelled `disk` on disk is a tracked one, by name alone: an
    /// index path (never an identity, which a hard link shares with the tracked file).
    fn tracked_entry(&self, disk: &str) -> Tracked {
        match self.index_path(disk) {
            Some(path) if self.removed.contains(&path) => Tracked::Removed,
            Some(path) => Tracked::Kept(path),
            None => Tracked::No,
        }
    }
}

fn uninspectable(relative: &str, error: &std::io::Error) -> String {
    format!(
        "'{relative}' cannot be inspected ({error}), so pull cannot tell whether it is in the \
         way; move or delete it, or fix its permissions, then run pull again"
    )
}

/// Why a path libgit2's checkout refuses refuses when gwz cannot tell what is in its way.
const UNEXPLAINED: &str = "the checkout refuses to write it, but the entry in its way could not \
                           be identified (check core.ignorecase and core.precomposeunicode)";

/// The paths `diff` adds, changes or retypes (as the new side names them) and those
/// it deletes; a path that is not UTF-8 is returned apart, as it cannot be checked.
#[derive(Default)]
struct DiffPaths {
    written: BTreeSet<String>,
    removed: HashSet<String>,
    unreadable: BTreeSet<String>,
}

fn diff_paths(diff: &git2::Diff<'_>) -> DiffPaths {
    let mut paths = DiffPaths::default();
    for delta in diff.deltas() {
        let (file, removed) = match delta.status() {
            git2::Delta::Deleted => (delta.old_file(), true),
            git2::Delta::Added | git2::Delta::Modified | git2::Delta::Typechange => {
                (delta.new_file(), false)
            }
            _ => continue,
        };
        let Some(bytes) = file.path_bytes() else {
            continue;
        };
        match (std::str::from_utf8(bytes), removed) {
            (Ok(path), true) => {
                paths.removed.insert(path.to_owned());
            }
            (Ok(path), false) => {
                paths.written.insert(path.to_owned());
            }
            (Err(_), true) => {}
            (Err(_), false) => {
                paths
                    .unreadable
                    .insert(String::from_utf8_lossy(bytes).into_owned());
            }
        }
    }
    paths
}

/// The collisions of checking out `tree` over the worktree at `root`, by incoming
/// path; with `written`, only those at the paths a replayed commit writes.
pub(super) fn blocked_entries(
    repo: &git2::Repository,
    root: &Path,
    tree: &git2::Tree<'_>,
    written: Option<&BTreeSet<String>>,
) -> ModelResult<Vec<GitUpdateCollision>> {
    let head_tree = repo.head().and_then(|head| head.peel_to_tree()).ok();
    let diff = repo
        .diff_tree_to_tree(head_tree.as_ref(), Some(tree), None)
        .map_err(git_error)?;
    let paths = diff_paths(&diff);
    let mut worktree = Worktree::open(repo, root, paths.removed)?;
    let reported = dry_run_checkout_conflicts(repo, tree)?;
    let checked = paths
        .written
        .into_iter()
        .filter(|path| !worktree.tracked.contains(path))
        .chain(reported.iter().cloned())
        .collect::<BTreeSet<_>>();
    let refused = |path: String, reason: &str| GitUpdateCollision {
        path,
        reason: Some(reason.to_owned()),
    };
    let mut blocked = paths
        .unreadable
        .into_iter()
        .map(|path| {
            refused(
                path,
                "its name is not UTF-8, so gwz cannot check what is in its way",
            )
        })
        .collect::<Vec<_>>();
    for path in checked {
        if written.is_some_and(|written| !writes(written, &path)) {
            continue;
        }
        // libgit2 refuses a path it reports, so the real checkout would fail after
        // the pull has moved other participants: never let one through unexplained.
        let explained = !reported.contains(&path);
        let blocker = match worktree.blocker(&path) {
            Ok(Some(blocker)) => blocker,
            Ok(None) if explained => continue,
            Ok(None) => {
                blocked.push(refused(path, UNEXPLAINED));
                continue;
            }
            Err(reason) => {
                blocked.push(refused(path, &reason));
                continue;
            }
        };
        let in_the_way = worktree.on_disk(&blocker).and_then(|disk| {
            untracked_in_the_way(
                &worktree,
                &blocker,
                &disk.unwrap_or_else(|| blocker.clone()),
            )
        });
        match in_the_way {
            Err(reason) => blocked.push(refused(path, &reason)),
            Ok(true) => blocked.push(GitUpdateCollision { path, reason: None }),
            // Only tracked entries the update removes are there: the checkout
            // itself replaces them.
            Ok(false) if explained => {}
            Ok(false) => blocked.push(refused(path, UNEXPLAINED)),
        }
    }
    Ok(blocked)
}

/// Whether a replayed commit writing `written` writes `path` (or below it).
fn writes(written: &BTreeSet<String>, path: &str) -> bool {
    let below = format!("{path}/");
    written.contains(path)
        || written
            .range(below.clone()..)
            .next()
            .is_some_and(|next| next.starts_with(&below))
}

/// The entries in the way of what replaying the local commits writes: for each
/// commit in `onto..local` that libgit2's rebase replays (it skips merges), the
/// collisions of its tree at the paths it adds or changes against its parent.
pub(super) fn replay_blocked(
    repo: &git2::Repository,
    root: &Path,
    onto: &str,
    local: &str,
) -> ModelResult<Vec<GitUpdateCollision>> {
    let mut walk = repo.revwalk().map_err(git_error)?;
    walk.push(git2::Oid::from_str(local).map_err(git_error)?)
        .map_err(git_error)?;
    walk.hide(git2::Oid::from_str(onto).map_err(git_error)?)
        .map_err(git_error)?;
    let mut blocked = BTreeMap::new();
    for oid in walk {
        let commit = repo
            .find_commit(oid.map_err(git_error)?)
            .map_err(git_error)?;
        if commit.parent_count() > 1 {
            continue;
        }
        let tree = commit.tree().map_err(git_error)?;
        let parent = commit
            .parents()
            .next()
            .map(|parent| parent.tree())
            .transpose()
            .map_err(git_error)?;
        let diff = repo
            .diff_tree_to_tree(parent.as_ref(), Some(&tree), None)
            .map_err(git_error)?;
        let written = diff_paths(&diff).written;
        for entry in blocked_entries(repo, root, &tree, Some(&written))? {
            blocked.entry(entry.path.clone()).or_insert(entry);
        }
    }
    Ok(blocked.into_values().collect())
}

/// libgit2 >= 1.8 `GIT_CHECKOUT_DRY_RUN` ("checks for conflicts, but doesn't
/// make any actual changes"). git2 0.21 has no builder method for it: its
/// `dry_run()` sets `GIT_CHECKOUT_NONE`, which in libgit2 1.8+ skips the
/// conflict checks too, and libgit2-sys 0.18 names no constant for it.
const GIT_CHECKOUT_DRY_RUN: u32 = 1 << 24;
/// `GIT_CHECKOUT_DONT_WRITE_INDEX`: a dry run otherwise still rewrites
/// `.git/index` on its way out (libgit2 1.9 checkout.c cleanup), dropping
/// extensions such as git's untracked cache and taking `index.lock`.
const GIT_CHECKOUT_DONT_WRITE_INDEX: u32 = 1 << 23;

/// The paths a safe checkout of `tree` would refuse, computed by libgit2
/// itself in dry-run mode with conflict notification.
fn dry_run_checkout_conflicts(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
) -> ModelResult<BTreeSet<String>> {
    let (major, minor, _) = git2::Version::get().libgit2_version();
    if (major, minor) < (1, 8) {
        return Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            format!("libgit2 {major}.{minor} has no dry-run checkout; 1.8 or later is required"),
        ));
    }
    let mut conflicts = BTreeSet::new();
    let mut builder = git2::build::CheckoutBuilder::new();
    // The flags of the real checkouts (`WorktreeRule::checkout_tree`): an ignored
    // entry at a path the update writes is a conflict here as it is there.
    builder
        .safe()
        .overwrite_ignored(false)
        .notify_on(git2::CheckoutNotificationType::CONFLICT)
        .notify(|_, path, _, _, _| {
            if let Some(path) = path {
                conflicts.insert(path.to_string_lossy().replace('\\', "/"));
            }
            true
        });
    // SAFETY: `options` is initialized by libgit2 before use; `builder`
    // (whose notify payload `configure` points at) and `conflicts` outlive the
    // call; `repo` and `tree` are live handles of the same repository. The
    // dry-run bit makes libgit2 stop after computing actions and notifying,
    // before it removes or writes anything (libgit2 checkout.c, after
    // `checkout_get_actions`); the don't-write-index bit keeps its cleanup from
    // writing the index back.
    let code = unsafe {
        let mut options = std::mem::MaybeUninit::<libgit2_sys::git_checkout_options>::zeroed();
        let init = libgit2_sys::git_checkout_init_options(
            options.as_mut_ptr(),
            libgit2_sys::GIT_CHECKOUT_OPTIONS_VERSION,
        );
        if init < 0 {
            return Err(git_error(git2::Error::last_error(init)));
        }
        let options = options.assume_init_mut();
        builder.configure(options);
        options.checkout_strategy |= GIT_CHECKOUT_DRY_RUN | GIT_CHECKOUT_DONT_WRITE_INDEX;
        libgit2_sys::git_checkout_tree(repo.raw(), tree.as_object().raw(), options)
    };
    drop(builder);
    // A conflict without ALLOW_CONFLICTS ends the dry run with GIT_ECONFLICT
    // after every conflict was notified; anything else negative is a failure.
    if code < 0 && code != libgit2_sys::GIT_ECONFLICT {
        return Err(git_error(git2::Error::last_error(code)));
    }
    Ok(conflicts)
}

/// Whether something in the way at `relative` (spelled `disk` on disk) is not a tracked
/// entry the update removes. Refuses (with the reason) a tracked entry the update keeps,
/// a repository, and an entry that cannot be inspected.
fn untracked_in_the_way(
    worktree: &Worktree<'_>,
    relative: &str,
    disk: &str,
) -> Result<bool, String> {
    let meta = match std::fs::symlink_metadata(worktree.root.join(relative)) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(uninspectable(relative, &error)),
    };
    walk_blocker(worktree, relative, disk, &meta).map(|walk| walk.untracked)
}

/// What a walk of an entry in the way found.
struct Walk {
    /// It is, or holds, a tracked entry the update removes.
    holds_tracked: bool,
    /// It is, or holds, an entry that is not an index path: untracked or ignored, a
    /// file, a symlink, or a directory holding nothing tracked (empty ones included).
    untracked: bool,
}

/// Walk the entry at `relative` (spelled `disk` on disk, described by `meta`, which does
/// not follow links, so a symlink is an entry of its own).
fn walk_blocker(
    worktree: &Worktree<'_>,
    relative: &str,
    disk: &str,
    meta: &std::fs::Metadata,
) -> Result<Walk, String> {
    match worktree.tracked_entry(disk) {
        Tracked::Removed => {
            return Ok(Walk {
                holds_tracked: true,
                untracked: false,
            });
        }
        Tracked::Kept(path) if path == relative => {
            return Err(format!("'{relative}' is tracked, and the update keeps it"));
        }
        Tracked::Kept(path) => {
            return Err(format!(
                "'{relative}' is the same file as the tracked '{path}' on this filesystem (the \
                 names differ only in case or normalization), and the update keeps it, so the \
                 update cannot be checked out here"
            ));
        }
        Tracked::No => {}
    }
    if !meta.is_dir() {
        return Ok(Walk {
            holds_tracked: false,
            untracked: true,
        });
    }
    let entries = std::fs::read_dir(worktree.root.join(relative))
        .map_err(|error| uninspectable(relative, &error))?;
    let mut walk = Walk {
        holds_tracked: false,
        untracked: false,
    };
    for entry in entries {
        let entry = entry.map_err(|error| uninspectable(relative, &error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(format!(
                "'{relative}' holds a path that is not UTF-8; move or delete it, then run pull \
                 again"
            ));
        };
        if name == ".git" {
            return Err(format!(
                "'{relative}' holds a repository; move it out of the way, then run pull again"
            ));
        }
        let child = format!("{relative}/{name}");
        let child_meta = std::fs::symlink_metadata(entry.path())
            .map_err(|error| uninspectable(&child, &error))?;
        let inside = walk_blocker(worktree, &child, &format!("{disk}/{name}"), &child_meta)?;
        walk.holds_tracked |= inside.holds_tracked;
        walk.untracked |= inside.untracked;
    }
    // A directory holding nothing tracked is untracked as a whole.
    walk.untracked |= !walk.holds_tracked;
    Ok(walk)
}
