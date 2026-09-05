//! Shared conformance suite and contract-faithful fakes for [`TreeCopier`].
//!
//! Enabled by the `contract-tests` feature (dev-dependencies only). Every
//! implementation runs [`run_all`] against itself; consumers use
//! [`ScriptedTreeCopier`] to script outcomes without touching disk and
//! [`OrdinaryTreeCopier`] when a real destination tree is needed.
//!
//! The helpers are std-only: a tiny temporary-tree builder, no `git`, no
//! adapter, no application dependency.

// The std writers below are ordinary file I/O in a test-support fake; this
// crate is outside gwz-core's merge-writer boundary (gwz-core/clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::cell::RefCell;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    CancelFlag, Cancellation, CopyError, CopyErrorCategory, CopyMode, CopyReport, CopyRequest,
    Exclusion, TreeCopier,
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique temporary directory removed on drop.
pub struct TempTree {
    path: PathBuf,
}

impl TempTree {
    pub fn new(label: &str) -> Self {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "gwz-copy-contract-{label}-{}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp tree");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write `bytes` at `relative`, creating parents.
    pub fn file(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&path, bytes).expect("write file");
        path
    }

    pub fn dir(&self, relative: &str) -> PathBuf {
        let path = self.path.join(relative);
        fs::create_dir_all(&path).expect("create dir");
        path
    }

    #[cfg(unix)]
    pub fn symlink(&self, relative: &str, target: &str) -> PathBuf {
        let path = self.path.join(relative);
        std::os::unix::fs::symlink(target, &path).expect("create symlink");
        path
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A contract-faithful std-only copier: ordinary read/write only, exclusions
/// applied during traversal, cancellation polled before every entry, no
/// hardlinks, symlinks recreated, unsupported entry types refused.
///
/// It is a fake for consumers and the suite's own witness, not the product
/// copier (`gwz-refcopy` adds native copy-on-write, metadata and fault
/// classification).
#[derive(Clone, Copy, Debug, Default)]
pub struct OrdinaryTreeCopier;

impl TreeCopier for OrdinaryTreeCopier {
    fn copy_tree(
        &self,
        request: &CopyRequest,
        cancellation: &dyn Cancellation,
    ) -> Result<CopyReport, CopyError> {
        let mut report = CopyReport::default();
        let source_meta = fs::symlink_metadata(&request.source).map_err(|error| {
            CopyError::refused(
                &request.source,
                CopyErrorCategory::SourceMissing,
                error.to_string(),
            )
        })?;
        if !source_meta.is_dir() {
            return Err(CopyError::refused(
                &request.source,
                CopyErrorCategory::SourceUnreadable,
                "source is not a directory",
            ));
        }
        match fs::read_dir(&request.destination) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(CopyError::refused(
                        &request.destination,
                        CopyErrorCategory::DestinationNotEmpty,
                        "destination exists and is not empty",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&request.destination).map_err(|error| {
                    CopyError::refused(
                        &request.destination,
                        CopyErrorCategory::DestinationUnwritable,
                        error.to_string(),
                    )
                })?;
            }
            Err(error) => {
                return Err(CopyError::refused(
                    &request.destination,
                    CopyErrorCategory::DestinationNotEmpty,
                    format!("destination is not an enumerable directory: {error}"),
                ));
            }
        }
        copy_dir(request, cancellation, Path::new(""), &mut report)
            .map(|()| report.clone())
            .map_err(|(failed, category, detail)| CopyError {
                failed_path: failed,
                category,
                detail,
                partial: report,
            })
    }
}

type Failure = (PathBuf, CopyErrorCategory, String);

fn copy_dir(
    request: &CopyRequest,
    cancellation: &dyn Cancellation,
    relative: &Path,
    report: &mut CopyReport,
) -> Result<(), Failure> {
    let source_dir = request.source.join(relative);
    let mut entries = fs::read_dir(&source_dir)
        .map_err(|error| {
            (
                relative.to_path_buf(),
                CopyErrorCategory::SourceUnreadable,
                error.to_string(),
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            (
                relative.to_path_buf(),
                CopyErrorCategory::SourceUnreadable,
                error.to_string(),
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let entry_relative = relative.join(entry.file_name());
        if cancellation.is_cancelled() {
            return Err((
                entry_relative,
                CopyErrorCategory::Cancelled,
                "cancelled before entry".to_owned(),
            ));
        }
        if request.is_excluded(&entry_relative) {
            continue;
        }
        let source_path = request.source.join(&entry_relative);
        let destination_path = request.destination.join(&entry_relative);
        let meta = fs::symlink_metadata(&source_path).map_err(|error| {
            (
                entry_relative.clone(),
                CopyErrorCategory::SourceUnreadable,
                error.to_string(),
            )
        })?;
        let file_type = meta.file_type();
        if file_type.is_symlink() {
            let target = fs::read_link(&source_path).map_err(|error| {
                (
                    entry_relative.clone(),
                    CopyErrorCategory::SourceUnreadable,
                    error.to_string(),
                )
            })?;
            make_symlink(&target, &destination_path).map_err(|error| {
                (
                    entry_relative.clone(),
                    CopyErrorCategory::MetadataFailed,
                    error.to_string(),
                )
            })?;
            report.symlinks += 1;
        } else if file_type.is_dir() {
            fs::create_dir(&destination_path).map_err(|error| {
                (
                    entry_relative.clone(),
                    CopyErrorCategory::DestinationUnwritable,
                    error.to_string(),
                )
            })?;
            report.directories += 1;
            copy_dir(request, cancellation, &entry_relative, report)?;
        } else if file_type.is_file() {
            let bytes = copy_file(&source_path, &destination_path)
                .map_err(|(category, detail)| (entry_relative.clone(), category, detail))?;
            report.ordinary_files += 1;
            report.logical_bytes += bytes;
        } else {
            return Err((
                entry_relative,
                CopyErrorCategory::UnsupportedEntry,
                "unsupported entry type".to_owned(),
            ));
        }
    }
    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> Result<u64, (CopyErrorCategory, String)> {
    let mut input = fs::File::open(source)
        .map_err(|error| (CopyErrorCategory::SourceUnreadable, error.to_string()))?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| (CopyErrorCategory::DestinationUnwritable, error.to_string()))?;
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| (CopyErrorCategory::SourceUnreadable, error.to_string()))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| (CopyErrorCategory::ShortWrite, error.to_string()))?;
        total += read as u64;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(source)
            .map_err(|error| (CopyErrorCategory::SourceUnreadable, error.to_string()))?
            .permissions()
            .mode();
        fs::set_permissions(destination, fs::Permissions::from_mode(mode))
            .map_err(|error| (CopyErrorCategory::MetadataFailed, error.to_string()))?;
    }
    Ok(total)
}

#[cfg(unix)]
fn make_symlink(target: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, destination)
}

#[cfg(windows)]
fn make_symlink(target: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, destination)
}

/// A recording copier for consumer tests. It touches no disk: each call is
/// recorded and answered from the scripted queue (a missing script refuses
/// as `Unimplemented`, never succeeds).
#[derive(Debug, Default)]
pub struct ScriptedTreeCopier {
    outcomes: RefCell<Vec<Result<CopyReport, CopyError>>>,
    calls: RefCell<Vec<CopyRequest>>,
}

impl ScriptedTreeCopier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue the next outcome (first queued, first returned).
    pub fn script(&self, outcome: Result<CopyReport, CopyError>) {
        self.outcomes.borrow_mut().push(outcome);
    }

    pub fn calls(&self) -> Vec<CopyRequest> {
        self.calls.borrow().clone()
    }
}

impl TreeCopier for ScriptedTreeCopier {
    fn copy_tree(
        &self,
        request: &CopyRequest,
        _cancellation: &dyn Cancellation,
    ) -> Result<CopyReport, CopyError> {
        self.calls.borrow_mut().push(request.clone());
        let mut outcomes = self.outcomes.borrow_mut();
        if outcomes.is_empty() {
            return Err(CopyError::refused(
                &request.destination,
                CopyErrorCategory::Unimplemented,
                "no scripted outcome",
            ));
        }
        outcomes.remove(0)
    }
}

/// Run every conformance case against `copier`.
pub fn run_all<C: TreeCopier>(copier: &C) {
    copies_included_bytes_and_skips_exclusions(copier);
    admits_an_empty_destination_directory(copier);
    refuses_a_nonempty_destination_without_writing(copier);
    refuses_a_missing_source(copier);
    cancellation_before_the_first_entry_reports_cancelled_with_an_accurate_partial_report(copier);
    // LCM1.0c-rem1 (Code P3-2): the suite must exercise a NON-EMPTY partial
    // report -- a cancellation after some entries are written, and (on unix) a
    // mid-copy destination write failure -- and prove the partial matches the
    // destination's real contents while the source is untouched.
    cancellation_after_the_first_entry_keeps_an_accurate_non_empty_partial_report(copier);
    #[cfg(unix)]
    a_read_only_destination_is_unwritable_and_keeps_an_accurate_partial(copier);
    ordinary_only_mode_reports_no_native_files(copier);
    #[cfg(unix)]
    never_hardlinks_source_files(copier);
    #[cfg(unix)]
    symlinks_remain_links_with_their_target(copier);
}

/// Cancellation that allows `allow` polls, then reports cancelled forever.
struct CancelAfter {
    remaining: std::sync::atomic::AtomicU64,
}

impl CancelAfter {
    fn new(allow: u64) -> Self {
        Self {
            remaining: std::sync::atomic::AtomicU64::new(allow),
        }
    }
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        // Decrement while polls remain; cancel once the budget is spent.
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

/// Count the regular files, directories (excluding `root`) and symlinks that
/// actually exist under `root`, so a partial report can be checked against the
/// destination's real contents.
fn observe_tree(root: &Path) -> (u64, u64, u64) {
    fn walk(dir: &Path, files: &mut u64, dirs: &mut u64, symlinks: &mut u64) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            let file_type = meta.file_type();
            if file_type.is_symlink() {
                *symlinks += 1;
            } else if file_type.is_dir() {
                *dirs += 1;
                walk(&path, files, dirs, symlinks);
            } else if file_type.is_file() {
                *files += 1;
            }
        }
    }
    let (mut files, mut dirs, mut symlinks) = (0, 0, 0);
    if root.exists() {
        walk(root, &mut files, &mut dirs, &mut symlinks);
    }
    (files, dirs, symlinks)
}

fn assert_partial_matches_destination(report: &CopyReport, destination: &Path) {
    let (files, dirs, symlinks) = observe_tree(destination);
    assert_eq!(
        report.files(),
        files,
        "the partial file count equals the destination's actual files"
    );
    assert_eq!(
        report.directories, dirs,
        "the partial directory count equals the destination's actual directories"
    );
    assert_eq!(
        report.symlinks, symlinks,
        "the partial symlink count equals the destination's actual symlinks"
    );
}

fn assert_source_is_untouched(source: &TempTree) {
    assert_eq!(fs::read(source.path().join("a.txt")).unwrap(), b"alpha");
    assert_eq!(
        fs::read(source.path().join("dir/b.bin")).unwrap(),
        [0u8, 1, 2, 3, 4, 5, 6]
    );
}

pub fn cancellation_after_the_first_entry_keeps_an_accurate_non_empty_partial_report<
    C: TreeCopier,
>(
    copier: &C,
) {
    let source = fixture();
    let parent = TempTree::new("cancel-midway-dest");
    let destination = parent.path().join("copy");
    // Allow exactly one poll (the first entry, `a.txt`), then cancel before
    // the next -- so at least one file is written and the partial is non-empty.
    let error = copier
        .copy_tree(
            &request(&source, destination.clone(), CopyMode::Auto),
            &CancelAfter::new(1),
        )
        .expect_err("a mid-copy cancellation stops the copy");
    assert_eq!(error.category, CopyErrorCategory::Cancelled);
    assert!(
        error.partial.files() >= 1,
        "at least one entry was written before cancellation: {:?}",
        error.partial
    );
    assert_partial_matches_destination(&error.partial, &destination);
    assert_source_is_untouched(&source);
}

#[cfg(unix)]
pub fn a_read_only_destination_is_unwritable_and_keeps_an_accurate_partial<C: TreeCopier>(
    copier: &C,
) {
    use std::os::unix::fs::PermissionsExt;
    let source = fixture();
    let parent = TempTree::new("unwritable-dest");
    // A pre-existing, admitted, empty destination that is readable and
    // traversable but not writable: the copy is admitted, then the first
    // write fails as `DestinationUnwritable`, and the partial is accurate.
    let destination = parent.path().join("copy");
    fs::create_dir(&destination).unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o500)).unwrap();
    let outcome = copier.copy_tree(
        &request(&source, destination.clone(), CopyMode::Auto),
        &crate::NeverCancelled,
    );
    // Restore write permission before any assertion can unwind, so the temp
    // tree can be removed on drop.
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
    let error = outcome.expect_err("a read-only destination cannot be written");
    assert_eq!(error.category, CopyErrorCategory::DestinationUnwritable);
    assert_partial_matches_destination(&error.partial, &destination);
    assert_source_is_untouched(&source);
}

fn fixture() -> TempTree {
    let source = TempTree::new("source");
    source.file("a.txt", b"alpha");
    source.file("dir/b.bin", &[0u8, 1, 2, 3, 4, 5, 6]);
    source.file("dir/skipme/inner", b"excluded");
    source.file("skip/x", b"excluded");
    source.dir("empty");
    #[cfg(unix)]
    source.symlink("link", "a.txt");
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

pub fn copies_included_bytes_and_skips_exclusions<C: TreeCopier>(copier: &C) {
    let source = fixture();
    let parent = TempTree::new("dest");
    let destination = parent.path().join("copy");
    let report = copier
        .copy_tree(
            &request(&source, destination.clone(), CopyMode::Auto),
            &crate::NeverCancelled,
        )
        .expect("copy succeeds");
    assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(
        fs::read(destination.join("dir/b.bin")).unwrap(),
        [0u8, 1, 2, 3, 4, 5, 6]
    );
    assert!(
        destination.join("empty").is_dir(),
        "empty directory is copied"
    );
    assert!(
        !destination.join("skip").exists(),
        "excluded subtree is never written"
    );
    assert!(
        !destination.join("dir/skipme").exists(),
        "nested exclusion is never written"
    );
    assert_eq!(report.files(), 2, "two regular files are included");
    assert_eq!(report.logical_bytes, 5 + 7);
    assert!(
        report.directories >= 2,
        "dir and empty are created: {report:?}"
    );
    assert_eq!(
        fs::read(source.path().join("a.txt")).unwrap(),
        b"alpha",
        "source is unchanged"
    );
}

pub fn admits_an_empty_destination_directory<C: TreeCopier>(copier: &C) {
    let source = fixture();
    let destination = TempTree::new("empty-dest");
    let report = copier
        .copy_tree(
            &request(&source, destination.path().to_path_buf(), CopyMode::Auto),
            &crate::NeverCancelled,
        )
        .expect("empty destination is admitted");
    assert_eq!(report.files(), 2);
}

pub fn refuses_a_nonempty_destination_without_writing<C: TreeCopier>(copier: &C) {
    let source = fixture();
    let destination = TempTree::new("nonempty-dest");
    destination.file("existing", b"keep");
    let error = copier
        .copy_tree(
            &request(&source, destination.path().to_path_buf(), CopyMode::Auto),
            &crate::NeverCancelled,
        )
        .expect_err("non-empty destination refuses");
    assert_eq!(error.category, CopyErrorCategory::DestinationNotEmpty);
    assert_eq!(error.partial, CopyReport::default(), "nothing was written");
    let names: Vec<_> = fs::read_dir(destination.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("existing")]);
    assert_eq!(
        fs::read(destination.path().join("existing")).unwrap(),
        b"keep"
    );
}

pub fn refuses_a_missing_source<C: TreeCopier>(copier: &C) {
    let parent = TempTree::new("missing-source");
    let request = CopyRequest {
        source: parent.path().join("absent"),
        destination: parent.path().join("copy"),
        exclusions: Vec::new(),
        mode: CopyMode::Auto,
    };
    let error = copier
        .copy_tree(&request, &crate::NeverCancelled)
        .expect_err("missing source refuses");
    assert_eq!(error.category, CopyErrorCategory::SourceMissing);
    assert!(
        !request.destination.exists(),
        "no destination is created for a missing source"
    );
}

pub fn cancellation_before_the_first_entry_reports_cancelled_with_an_accurate_partial_report<
    C: TreeCopier,
>(
    copier: &C,
) {
    let source = fixture();
    let parent = TempTree::new("cancel-dest");
    let destination = parent.path().join("copy");
    let flag = CancelFlag::new();
    flag.cancel();
    let error = copier
        .copy_tree(
            &request(&source, destination.clone(), CopyMode::Auto),
            &flag,
        )
        .expect_err("pre-cancelled copy stops");
    assert_eq!(error.category, CopyErrorCategory::Cancelled);
    assert_eq!(
        error.partial.files(),
        0,
        "no entry is copied after cancellation"
    );
    if destination.exists() {
        let count = fs::read_dir(&destination).unwrap().count();
        assert_eq!(
            count, 0,
            "a retained destination holds only what the partial report counts"
        );
    }
}

pub fn ordinary_only_mode_reports_no_native_files<C: TreeCopier>(copier: &C) {
    let source = fixture();
    let parent = TempTree::new("ordinary-dest");
    let report = copier
        .copy_tree(
            &request(&source, parent.path().join("copy"), CopyMode::OrdinaryOnly),
            &crate::NeverCancelled,
        )
        .expect("ordinary copy succeeds");
    assert_eq!(report.native_files, 0);
    assert_eq!(report.ordinary_files, 2);
}

#[cfg(unix)]
pub fn never_hardlinks_source_files<C: TreeCopier>(copier: &C) {
    use std::os::unix::fs::MetadataExt;
    let source = fixture();
    let parent = TempTree::new("inode-dest");
    let destination = parent.path().join("copy");
    copier
        .copy_tree(
            &request(&source, destination.clone(), CopyMode::Auto),
            &crate::NeverCancelled,
        )
        .expect("copy succeeds");
    let source_ino = fs::metadata(source.path().join("a.txt")).unwrap().ino();
    let destination_ino = fs::metadata(destination.join("a.txt")).unwrap().ino();
    assert_ne!(
        source_ino, destination_ino,
        "destination file is not a hardlink to the source"
    );
    fs::write(destination.join("a.txt"), b"changed").unwrap();
    assert_eq!(
        fs::read(source.path().join("a.txt")).unwrap(),
        b"alpha",
        "writes are independent"
    );
}

#[cfg(unix)]
pub fn symlinks_remain_links_with_their_target<C: TreeCopier>(copier: &C) {
    let source = fixture();
    let parent = TempTree::new("link-dest");
    let destination = parent.path().join("copy");
    let report = copier
        .copy_tree(
            &request(&source, destination.clone(), CopyMode::Auto),
            &crate::NeverCancelled,
        )
        .expect("copy succeeds");
    let meta = fs::symlink_metadata(destination.join("link")).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "symlink is recreated as a link"
    );
    assert_eq!(
        fs::read_link(destination.join("link")).unwrap(),
        PathBuf::from("a.txt")
    );
    assert_eq!(report.symlinks, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_copier_satisfies_the_conformance_suite() {
        run_all(&OrdinaryTreeCopier);
    }

    /// LCM1.0c-rem1 (Code P3-2): the accuracy assertion is load-bearing -- a
    /// copier that writes an entry but reports an empty partial fails the
    /// suite. A copier that merely matched the category would pass without it.
    #[test]
    fn the_accuracy_check_rejects_a_copier_that_lies_about_its_partial_report() {
        /// Writes `a.txt` to the destination but reports a cancellation with
        /// an empty partial report -- an inaccurate partial.
        struct LyingCopier;
        impl TreeCopier for LyingCopier {
            fn copy_tree(
                &self,
                request: &CopyRequest,
                _cancellation: &dyn Cancellation,
            ) -> Result<CopyReport, CopyError> {
                let _ = fs::create_dir_all(&request.destination);
                let _ = fs::write(request.destination.join("a.txt"), b"alpha");
                Err(CopyError {
                    failed_path: PathBuf::from("a.txt"),
                    category: CopyErrorCategory::Cancelled,
                    detail: "lies about the partial".to_owned(),
                    partial: CopyReport::default(),
                })
            }
        }
        let panicked = std::panic::catch_unwind(|| {
            cancellation_after_the_first_entry_keeps_an_accurate_non_empty_partial_report(
                &LyingCopier,
            );
        })
        .is_err();
        assert!(
            panicked,
            "the suite must reject a copier whose partial report is inaccurate"
        );
    }

    #[test]
    fn scripted_copier_records_calls_and_refuses_without_a_script() {
        let copier = ScriptedTreeCopier::new();
        let request = CopyRequest {
            source: PathBuf::from("/src"),
            destination: PathBuf::from("/dst"),
            exclusions: Vec::new(),
            mode: CopyMode::Auto,
        };
        let error = copier
            .copy_tree(&request, &crate::NeverCancelled)
            .expect_err("unscripted call refuses");
        assert_eq!(error.category, CopyErrorCategory::Unimplemented);
        copier.script(Ok(CopyReport {
            ordinary_files: 3,
            ..CopyReport::default()
        }));
        let report = copier.copy_tree(&request, &crate::NeverCancelled).unwrap();
        assert_eq!(report.files(), 3);
        assert_eq!(copier.calls().len(), 2);
    }
}
