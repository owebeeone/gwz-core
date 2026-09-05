//! Tier A behavioural tests for the ordinary copy path
//! (`dev-docs/GwzLocalCloneImplementationArchitecture.md` §3, "Tests").
//!
//! Fixtures are tiny temporary trees built with the copy contract's own
//! `TempTree`; there is no process spawn, sleep, service or large tree. The
//! large-tree performance and native-mechanism cases are Tier B/C and are not
//! here.

// Ordinary file I/O in test fixtures: this crate is outside gwz-core's
// merge-writer boundary (gwz-core/clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use gwz_copy_contract::{
    Cancellation, CopyErrorCategory, CopyMode, CopyReport, CopyRequest, CopyWarningKind, Exclusion,
    NeverCancelled, TreeCopier,
    contract_tests::{TempTree, run_all},
};

use crate::{NativeCapability, NativeMechanism, SystemTreeCopier};

// ---------------------------------------------------------------- helpers

/// Cancellation that allows `remaining` polls, then cancels forever.
struct CancelAfter {
    remaining: AtomicU64,
}

impl CancelAfter {
    fn new(polls: u64) -> Self {
        Self {
            remaining: AtomicU64::new(polls),
        }
    }
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        loop {
            let current = self.remaining.load(Ordering::SeqCst);
            if current == 0 {
                return true;
            }
            if self
                .remaining
                .compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return false;
            }
        }
    }
}

/// What one entry is, for whole-tree comparison.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    File { bytes: Vec<u8>, mode: Option<u32> },
    Directory { mode: Option<u32> },
    Symlink { target: PathBuf },
    Other,
}

fn mode_of(metadata: &fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

/// Every entry under `root`, keyed by its root-relative path.
fn observe(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn walk(root: &Path, relative: &Path, into: &mut BTreeMap<PathBuf, Entry>) {
        let Ok(entries) = fs::read_dir(root.join(relative)) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let child = relative.join(entry.file_name());
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            let file_type = metadata.file_type();
            let observed = if file_type.is_symlink() {
                Entry::Symlink {
                    target: fs::read_link(&path).expect("a link has a target"),
                }
            } else if file_type.is_dir() {
                Entry::Directory {
                    mode: mode_of(&metadata),
                }
            } else if file_type.is_file() {
                Entry::File {
                    bytes: fs::read(&path).expect("a copied file is readable"),
                    mode: mode_of(&metadata),
                }
            } else {
                Entry::Other
            };
            let descend = matches!(observed, Entry::Directory { .. });
            into.insert(child.clone(), observed);
            if descend {
                walk(root, &child, into);
            }
        }
    }
    let mut observed = BTreeMap::new();
    walk(root, Path::new(""), &mut observed);
    observed
}

/// The counts a report must match: files, directories, symlinks present.
fn counts(root: &Path) -> (u64, u64, u64) {
    let mut counted = (0, 0, 0);
    for entry in observe(root).values() {
        match entry {
            Entry::File { .. } => counted.0 += 1,
            Entry::Directory { .. } => counted.1 += 1,
            Entry::Symlink { .. } => counted.2 += 1,
            Entry::Other => {}
        }
    }
    counted
}

fn assert_report_matches(report: &CopyReport, destination: &Path) {
    let (files, directories, symlinks) = counts(destination);
    assert_eq!(
        (report.files(), report.directories, report.symlinks),
        (files, directories, symlinks),
        "the report must equal what the destination actually holds"
    );
}

/// A small mixed tree: nested directories, an empty directory, a nested
/// symlink, an empty file, an executable and two excluded subtrees.
fn fixture() -> TempTree {
    let source = TempTree::new("r-src");
    source.file("a.txt", b"alpha");
    source.file("dir/b.bin", &[0u8, 1, 2, 3, 4, 5, 6]);
    source.file("dir/nested/c.txt", b"gamma");
    source.file("dir/empty-file", b"");
    source.file("dir/skipme/inner", b"excluded");
    source.file("skip/x", b"excluded");
    source.file("keep.txt", b"kept");
    source.dir("empty");
    #[cfg(unix)]
    {
        source.symlink("link", "a.txt");
        source.symlink("dir/nested/up", "../b.bin");
        source.symlink("dir/dangling", "nowhere");
        source.symlink("dir/to-dir", "nested");
    }
    source
}

fn request(source: &TempTree, destination: PathBuf, mode: CopyMode) -> CopyRequest {
    CopyRequest {
        source: source.path().to_path_buf(),
        destination,
        exclusions: vec![
            Exclusion::RelativePath(PathBuf::from("skip")),
            Exclusion::RelativePath(PathBuf::from("dir/skipme")),
        ],
        mode,
    }
}

fn copy(request: &CopyRequest) -> CopyReport {
    SystemTreeCopier::new()
        .copy_tree(request, &NeverCancelled)
        .expect("the copy succeeds")
}

// ------------------------------------------------------------ conformance

#[test]
fn the_copier_satisfies_the_copy_contract_conformance_suite() {
    run_all(&SystemTreeCopier::new());
}

// ------------------------------------------------------- copied contents

#[test]
fn auto_and_forced_ordinary_copies_produce_the_same_included_tree() {
    let source = fixture();
    let parent = TempTree::new("r-modes");
    let auto = parent.path().join("auto");
    let ordinary = parent.path().join("ordinary");
    let auto_report = copy(&request(&source, auto.clone(), CopyMode::Auto));
    let ordinary_report = copy(&request(&source, ordinary.clone(), CopyMode::OrdinaryOnly));

    let included = observe(&auto);
    assert_eq!(
        included,
        observe(&ordinary),
        "both modes copy the same tree"
    );
    assert_eq!(
        included,
        observe(source.path())
            .into_iter()
            .filter(|(path, _)| !path.starts_with("skip") && !path.starts_with("dir/skipme"))
            .collect::<BTreeMap<_, _>>(),
        "the copy equals the source minus the exclusions, byte for byte"
    );
    assert_eq!(auto_report.files(), ordinary_report.files());
    assert_eq!(auto_report.logical_bytes, ordinary_report.logical_bytes);
    assert_eq!(
        (auto_report.native_files, ordinary_report.native_files),
        (0, 0),
        "no report may claim a native mechanism ran"
    );
    assert_report_matches(&auto_report, &auto);
}

#[test]
fn each_side_can_be_mutated_without_disturbing_the_other() {
    let source = fixture();
    let parent = TempTree::new("r-independent");
    let destination = parent.path().join("copy");
    copy(&request(&source, destination.clone(), CopyMode::Auto));

    fs::write(destination.join("a.txt"), b"destination edit").unwrap();
    assert_eq!(fs::read(source.path().join("a.txt")).unwrap(), b"alpha");
    fs::write(source.path().join("dir/b.bin"), b"source edit").unwrap();
    assert_eq!(
        fs::read(destination.join("dir/b.bin")).unwrap(),
        [0u8, 1, 2, 3, 4, 5, 6],
        "the copy is independent of later source writes"
    );
    fs::remove_file(source.path().join("keep.txt")).unwrap();
    assert!(destination.join("keep.txt").is_file());
}

#[test]
fn excluded_subtrees_are_never_written_at_any_depth() {
    let source = fixture();
    let parent = TempTree::new("r-exclusions");
    let destination = parent.path().join("copy");
    let report = copy(&request(&source, destination.clone(), CopyMode::Auto));
    for excluded in ["skip", "skip/x", "dir/skipme", "dir/skipme/inner"] {
        assert!(
            !destination.join(excluded).exists(),
            "{excluded} was excluded during traversal, not written and removed"
        );
    }
    assert!(
        destination.join("dir").is_dir(),
        "its parent is still copied"
    );
    assert_report_matches(&report, &destination);
}

#[test]
fn an_empty_file_and_an_empty_directory_are_copied() {
    let source = fixture();
    let parent = TempTree::new("r-empty");
    let destination = parent.path().join("copy");
    let report = copy(&request(&source, destination.clone(), CopyMode::Auto));
    assert_eq!(fs::read(destination.join("dir/empty-file")).unwrap(), b"");
    assert!(destination.join("empty").is_dir());
    assert_report_matches(&report, &destination);
}

#[cfg(unix)]
#[test]
fn nested_symlinks_stay_links_with_their_target_and_are_never_followed() {
    let source = fixture();
    let parent = TempTree::new("r-links");
    let destination = parent.path().join("copy");
    let report = copy(&request(&source, destination.clone(), CopyMode::Auto));
    for (link, target) in [
        ("link", "a.txt"),
        ("dir/nested/up", "../b.bin"),
        ("dir/dangling", "nowhere"),
        ("dir/to-dir", "nested"),
    ] {
        let path = destination.join(link);
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink(),
            "{link} is recreated as a link, not as its target"
        );
        assert_eq!(fs::read_link(&path).unwrap(), PathBuf::from(target));
    }
    let under_the_link: Vec<_> = observe(&destination)
        .into_keys()
        .filter(|path| path.starts_with("dir/to-dir"))
        .collect();
    assert_eq!(
        under_the_link,
        vec![PathBuf::from("dir/to-dir")],
        "a directory link is the only entry there: it is not walked into a second copy of the tree"
    );
    assert_eq!(report.symlinks, 4);
    assert_report_matches(&report, &destination);
}

#[test]
fn a_deep_tree_copies_without_recursing_on_the_stack() {
    let source = TempTree::new("r-deep");
    let deep: PathBuf = (0..128).map(|index| format!("{}", index % 10)).collect();
    source.file(&deep.join("leaf.txt").to_string_lossy(), b"bottom");
    let parent = TempTree::new("r-deep-dest");
    let destination = parent.path().join("copy");
    let report = copy(&CopyRequest {
        source: source.path().to_path_buf(),
        destination: destination.clone(),
        exclusions: Vec::new(),
        mode: CopyMode::Auto,
    });
    assert_eq!(
        fs::read(destination.join(&deep).join("leaf.txt")).unwrap(),
        b"bottom"
    );
    assert_eq!(report.directories, 128);
}

#[test]
fn a_sparse_file_reports_its_logical_bytes() {
    const LOGICAL: u64 = 1 << 20;
    let source = TempTree::new("r-sparse");
    let path = source.file("hole.bin", b"");
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    // A hole, not a megabyte of written zeroes: the logical length is what the
    // report must count.
    file.set_len(LOGICAL).unwrap();
    drop(file);
    let parent = TempTree::new("r-sparse-dest");
    let destination = parent.path().join("copy");
    let report = copy(&CopyRequest {
        source: source.path().to_path_buf(),
        destination: destination.clone(),
        exclusions: Vec::new(),
        mode: CopyMode::Auto,
    });
    assert_eq!(report.logical_bytes, LOGICAL);
    let copied = destination.join("hole.bin");
    assert_eq!(fs::metadata(&copied).unwrap().len(), LOGICAL);
    assert!(
        fs::read(&copied).unwrap().iter().all(|byte| *byte == 0),
        "the hole reads back as zeroes"
    );
}

// ------------------------------------------------------------- metadata

#[cfg(unix)]
#[test]
fn unix_permission_bits_round_trip_for_files_directories_and_the_root() {
    use std::os::unix::fs::PermissionsExt;
    let source = TempTree::new("r-modes-unix");
    source.file("plain", b"p");
    source.file("script", b"#!/bin/sh\n");
    source.file("private", b"secret");
    source.dir("group");
    source.file("group/inside", b"i");
    let chmod = |relative: &str, mode: u32| {
        fs::set_permissions(
            source.path().join(relative),
            fs::Permissions::from_mode(mode),
        )
        .unwrap()
    };
    chmod("plain", 0o644);
    chmod("script", 0o755);
    chmod("private", 0o600);
    chmod("group/inside", 0o640);
    chmod("group", 0o750);
    fs::set_permissions(source.path(), fs::Permissions::from_mode(0o700)).unwrap();

    let parent = TempTree::new("r-modes-dest");
    let destination = parent.path().join("copy");
    copy(&CopyRequest {
        source: source.path().to_path_buf(),
        destination: destination.clone(),
        exclusions: Vec::new(),
        mode: CopyMode::Auto,
    });
    let mode_at = |root: &Path, relative: &str| {
        fs::symlink_metadata(root.join(relative))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777
    };
    for relative in ["plain", "script", "private", "group", "group/inside"] {
        assert_eq!(
            mode_at(&destination, relative),
            mode_at(source.path(), relative),
            "{relative} keeps its permission and executable bits"
        );
    }
    assert_eq!(
        fs::metadata(&destination).unwrap().permissions().mode() & 0o7777,
        0o700,
        "a destination root created by the copy takes the source root's mode"
    );
}

#[cfg(unix)]
#[test]
fn a_read_only_source_file_is_copied_and_stays_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let source = TempTree::new("r-readonly");
    let path = source.file("frozen.txt", b"immutable");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let parent = TempTree::new("r-readonly-dest");
    let destination = parent.path().join("copy");
    let report = copy(&CopyRequest {
        source: source.path().to_path_buf(),
        destination: destination.clone(),
        exclusions: Vec::new(),
        mode: CopyMode::Auto,
    });
    let copied = destination.join("frozen.txt");
    assert_eq!(fs::read(&copied).unwrap(), b"immutable");
    assert_eq!(
        fs::metadata(&copied).unwrap().permissions().mode() & 0o7777,
        0o444
    );
    assert_eq!(report.files(), 1);
}

// ------------------------------------------------- failures and refusals

#[cfg(unix)]
#[test]
fn an_unreadable_source_directory_stops_the_copy_with_an_accurate_partial() {
    use std::os::unix::fs::PermissionsExt;
    let source = TempTree::new("r-unreadable");
    source.file("a.txt", b"alpha");
    source.file("locked/inner", b"hidden");
    source.file("z.txt", b"omega");
    let locked = source.path().join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let parent = TempTree::new("r-unreadable-dest");
    let destination = parent.path().join("copy");
    let outcome = SystemTreeCopier::new().copy_tree(
        &CopyRequest {
            source: source.path().to_path_buf(),
            destination: destination.clone(),
            exclusions: Vec::new(),
            mode: CopyMode::Auto,
        },
        &NeverCancelled,
    );
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();

    let error = outcome.expect_err("an unreadable source directory is a failure");
    assert_eq!(error.category, CopyErrorCategory::SourceUnreadable);
    assert_eq!(error.failed_path, PathBuf::from("locked"));
    assert_eq!(
        (error.partial.files(), error.partial.directories),
        (1, 1),
        "the entries written before the failure are reported: {:?}",
        error.partial
    );
    assert_report_matches(&error.partial, &destination);
    assert!(
        !destination.join("z.txt").exists(),
        "the copy stopped at the failure"
    );
    assert_eq!(
        fs::read(source.path().join("a.txt")).unwrap(),
        b"alpha",
        "the source is unchanged"
    );
}

#[test]
fn a_cancellation_mid_tree_reports_what_was_written_and_leaves_it_in_place() {
    let source = fixture();
    let parent = TempTree::new("r-cancel");
    let destination = parent.path().join("copy");
    let error = SystemTreeCopier::new()
        .copy_tree(
            &request(&source, destination.clone(), CopyMode::Auto),
            &CancelAfter::new(3),
        )
        .expect_err("cancellation stops the copy");
    assert_eq!(error.category, CopyErrorCategory::Cancelled);
    assert!(
        error.partial.files() + error.partial.directories >= 1,
        "entries written before the cancellation are reported: {:?}",
        error.partial
    );
    assert_report_matches(&error.partial, &destination);
    assert!(
        destination.exists(),
        "a cancelled copy retains its partial destination for inspection"
    );
    assert!(
        !error.failed_path.as_os_str().is_empty(),
        "the failed path names the entry the copy stopped at"
    );
}

#[test]
fn the_source_is_only_ever_read() {
    let source = fixture();
    let before = observe(source.path());
    let parent = TempTree::new("r-source-untouched");
    copy(&request(&source, parent.path().join("ok"), CopyMode::Auto));
    assert_eq!(
        observe(source.path()),
        before,
        "a successful copy reads only"
    );

    let cancelled = SystemTreeCopier::new().copy_tree(
        &request(&source, parent.path().join("cancelled"), CopyMode::Auto),
        &CancelAfter::new(2),
    );
    assert!(cancelled.is_err());
    let refused = SystemTreeCopier::new().copy_tree(
        &request(&source, parent.path().join("ok"), CopyMode::Auto),
        &NeverCancelled,
    );
    assert!(
        refused.is_err(),
        "the second copy finds a non-empty destination"
    );
    assert_eq!(
        observe(source.path()),
        before,
        "a cancelled or refused copy reads only, too"
    );
}

#[test]
fn no_temporary_file_survives_a_cancelled_entry() {
    // A file large enough to need more than one buffered write, cancelled
    // between chunks: the incomplete entry is removed, so the partial report
    // still equals the destination's contents.
    let source = TempTree::new("r-temp");
    source.file("big.bin", &vec![7u8; 256 * 1024]);
    let parent = TempTree::new("r-temp-dest");
    let destination = parent.path().join("copy");
    let error = SystemTreeCopier::new()
        .copy_tree(
            &CopyRequest {
                source: source.path().to_path_buf(),
                destination: destination.clone(),
                exclusions: Vec::new(),
                mode: CopyMode::Auto,
            },
            &CancelAfter::new(1),
        )
        .expect_err("the copy is cancelled inside the file");
    assert_eq!(error.category, CopyErrorCategory::Cancelled);
    assert_eq!(error.failed_path, PathBuf::from("big.bin"));
    assert_eq!(error.partial.files(), 0);
    assert_eq!(
        fs::read_dir(&destination).unwrap().count(),
        0,
        "the incomplete entry and its temporary are both gone"
    );
    assert_report_matches(&error.partial, &destination);
}

#[cfg(unix)]
#[test]
fn an_unsupported_entry_type_refuses_by_type_without_opening_it() {
    let source = TempTree::new("r-sock");
    source.file("a.txt", b"alpha");
    // A socket is the one special file std can create; opening it would block,
    // so the copier must refuse from its link-level metadata alone.
    let socket = source.path().join("s");
    let Ok(listener) = std::os::unix::net::UnixListener::bind(&socket) else {
        eprintln!("skipped: this host cannot bind a unix socket at {socket:?}");
        return;
    };
    let parent = TempTree::new("r-sock-dest");
    let destination = parent.path().join("copy");
    let error = SystemTreeCopier::new()
        .copy_tree(
            &CopyRequest {
                source: source.path().to_path_buf(),
                destination: destination.clone(),
                exclusions: Vec::new(),
                mode: CopyMode::Auto,
            },
            &NeverCancelled,
        )
        .expect_err("an unsupported entry type refuses");
    drop(listener);
    assert_eq!(error.category, CopyErrorCategory::UnsupportedEntry);
    assert_eq!(error.failed_path, PathBuf::from("s"));
    assert!(error.detail.contains("socket"), "{}", error.detail);
    assert_eq!(error.partial.files(), 1, "a.txt was copied first");
    assert_report_matches(&error.partial, &destination);
    assert!(!destination.join("s").exists(), "nothing stands in for it");
}

#[test]
fn a_destination_inside_the_source_is_refused_before_anything_is_written() {
    let source = fixture();
    let destination = source.path().join("dir/inside");
    let error = SystemTreeCopier::new()
        .copy_tree(
            &request(&source, destination.clone(), CopyMode::Auto),
            &NeverCancelled,
        )
        .expect_err("copying a tree into itself is refused");
    assert_eq!(error.category, CopyErrorCategory::DestinationUnwritable);
    assert_eq!(error.partial, CopyReport::default());
    assert!(!destination.exists(), "nothing was created");
}

#[test]
fn a_source_inside_the_destination_is_refused() {
    let outer = TempTree::new("r-outer");
    let source = outer.path().join("inner");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("a.txt"), b"alpha").unwrap();
    // The destination already exists and contains the source: refused before
    // the emptiness check even considers it.
    let error = SystemTreeCopier::new()
        .copy_tree(
            &CopyRequest {
                source,
                destination: outer.path().to_path_buf(),
                exclusions: Vec::new(),
                mode: CopyMode::Auto,
            },
            &NeverCancelled,
        )
        .expect_err("copying a tree into its own parent is refused");
    assert_eq!(error.category, CopyErrorCategory::DestinationUnwritable);
    assert_eq!(error.partial, CopyReport::default());
}

#[test]
fn a_destination_that_is_a_file_is_refused_without_writing() {
    let source = fixture();
    let parent = TempTree::new("r-file-dest");
    let destination = parent.file("occupied", b"not a directory");
    let error = SystemTreeCopier::new()
        .copy_tree(
            &request(&source, destination.clone(), CopyMode::Auto),
            &NeverCancelled,
        )
        .expect_err("a file destination is refused");
    assert_eq!(error.category, CopyErrorCategory::DestinationNotEmpty);
    assert_eq!(fs::read(&destination).unwrap(), b"not a directory");
}

#[cfg(unix)]
#[test]
fn neither_a_symlinked_source_nor_a_symlinked_destination_is_followed() {
    let real = fixture();
    let parent = TempTree::new("r-symlinked");
    let source_link = parent.path().join("source-link");
    std::os::unix::fs::symlink(real.path(), &source_link).unwrap();
    let error = SystemTreeCopier::new()
        .copy_tree(
            &CopyRequest {
                source: source_link,
                destination: parent.path().join("copy"),
                exclusions: Vec::new(),
                mode: CopyMode::Auto,
            },
            &NeverCancelled,
        )
        .expect_err("a symlinked source is not followed");
    assert_eq!(error.category, CopyErrorCategory::SourceUnreadable);

    let empty = TempTree::new("r-symlinked-target");
    let destination_link = parent.path().join("destination-link");
    std::os::unix::fs::symlink(empty.path(), &destination_link).unwrap();
    let error = SystemTreeCopier::new()
        .copy_tree(
            &request(&real, destination_link, CopyMode::Auto),
            &NeverCancelled,
        )
        .expect_err("a symlinked destination is not followed");
    assert_eq!(error.category, CopyErrorCategory::DestinationNotEmpty);
    assert_eq!(
        fs::read_dir(empty.path()).unwrap().count(),
        0,
        "the link's target was not written through"
    );
}

// -------------------------------------------------- what the report says

#[test]
fn auto_mode_reports_that_native_copy_on_write_was_unavailable() {
    let source = fixture();
    let parent = TempTree::new("r-warnings");
    let auto = copy(&request(
        &source,
        parent.path().join("auto"),
        CopyMode::Auto,
    ));
    let native: Vec<_> = auto
        .warnings
        .iter()
        .filter(|warning| warning.kind == CopyWarningKind::NativeUnsupportedFellBack)
        .collect();
    assert_eq!(native.len(), 1, "one copy-wide warning, not one per file");
    assert!(
        native[0].detail.contains("unavailable in this build"),
        "{}",
        native[0].detail
    );
    assert_eq!(auto.native_files, 0);
    assert_eq!(
        auto.ordinary_files,
        auto.files(),
        "every file is reported by the mechanism that actually copied it"
    );
    assert!(
        auto.warnings
            .iter()
            .any(|warning| warning.kind == CopyWarningKind::AncillaryMetadataUnsupported),
        "unsupported ancillary metadata is reported, not silently dropped"
    );

    let ordinary = copy(&request(
        &source,
        parent.path().join("ordinary"),
        CopyMode::OrdinaryOnly,
    ));
    assert!(
        !ordinary
            .warnings
            .iter()
            .any(|warning| warning.kind == CopyWarningKind::NativeUnsupportedFellBack),
        "an ordinary-only copy was never promised a native path"
    );
}

#[test]
fn a_partial_report_keeps_the_copy_wide_warnings() {
    let source = fixture();
    let parent = TempTree::new("r-warn-partial");
    let error = SystemTreeCopier::new()
        .copy_tree(
            &request(&source, parent.path().join("copy"), CopyMode::Auto),
            &CancelAfter::new(2),
        )
        .expect_err("cancellation stops the copy");
    assert!(
        error
            .partial
            .warnings
            .iter()
            .any(|warning| warning.kind == CopyWarningKind::NativeUnsupportedFellBack),
        "a partial report still says how the copy was performed"
    );
}

#[test]
fn the_native_probe_and_mechanism_are_honest_about_this_build() {
    let copier = SystemTreeCopier::new();
    let source = TempTree::new("r-probe");
    let destination = source.path().join("copy");
    assert_eq!(
        copier.probe_native(source.path(), &destination),
        NativeCapability::Unavailable,
        "no native mechanism is linked; the probe must not suggest one"
    );
    assert_eq!(copier.mechanism(), NativeMechanism::None);
}

#[cfg(unix)]
#[test]
fn a_private_directory_is_never_wider_than_its_source_while_its_subtree_is_copied() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    /// Cancellation is polled once per entry, which makes it the one hook that
    /// can observe the destination *during* a copy. This one never cancels; it
    /// records the modes it sees.
    struct ModeObserver {
        watched: Vec<PathBuf>,
        seen: Mutex<Vec<(PathBuf, u32)>>,
    }

    impl Cancellation for ModeObserver {
        fn is_cancelled(&self) -> bool {
            let mut seen = self.seen.lock().expect("no test thread panics here");
            for path in &self.watched {
                if let Ok(metadata) = fs::symlink_metadata(path) {
                    seen.push((path.clone(), metadata.permissions().mode() & 0o7777));
                }
            }
            false
        }
    }

    let source = TempTree::new("r-private");
    source.dir("private");
    source.file("private/one.txt", b"1");
    source.file("private/two.txt", b"2");
    fs::set_permissions(
        source.path().join("private"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::set_permissions(source.path(), fs::Permissions::from_mode(0o700)).unwrap();

    let parent = TempTree::new("r-private-dest");
    let destination = parent.path().join("copy");
    let observer = ModeObserver {
        watched: vec![destination.clone(), destination.join("private")],
        seen: Mutex::new(Vec::new()),
    };
    let report = SystemTreeCopier::new()
        .copy_tree(
            &CopyRequest {
                source: source.path().to_path_buf(),
                destination: destination.clone(),
                exclusions: Vec::new(),
                mode: CopyMode::Auto,
            },
            &observer,
        )
        .expect("the copy succeeds");
    assert_eq!(report.files(), 2);

    let seen = observer.seen.into_inner().unwrap();
    assert!(
        seen.iter().any(|(path, _)| path.ends_with("private")),
        "the observer saw the directory mid-copy: {seen:?}"
    );
    for (path, mode) in &seen {
        assert_eq!(
            mode & 0o077,
            0,
            "{path:?} was group/world accessible mid-copy with mode {mode:o}"
        );
    }
    for relative in ["", "private"] {
        assert_eq!(
            fs::metadata(destination.join(relative))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o700,
            "the exact source mode lands once the subtree is complete"
        );
    }
}
