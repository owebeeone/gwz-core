//! White-box Tier A tests for the copy engine's bounded loops, its error
//! classification and its path resolution. Deterministic values and fakes
//! only: the behavioural tree tests live in `crate::tests`.

use std::cell::Cell;

use gwz_copy_contract::{CancelFlag, CopyMode, NeverCancelled, contract_tests::TempTree};

use super::*;
use crate::native::Attempt;

/// A writer that accepts at most `chunk` bytes per call and interrupts every
/// other call, so the copier's write loop must retry and advance.
#[derive(Default)]
struct StubbornWriter {
    written: Vec<u8>,
    chunk: usize,
    interrupt_next: bool,
    flush_error: Option<ErrorKind>,
}

impl Write for StubbornWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.interrupt_next {
            self.interrupt_next = false;
            return Err(io::Error::from(ErrorKind::Interrupted));
        }
        self.interrupt_next = true;
        let accepted = self.chunk.min(buffer.len());
        self.written.extend_from_slice(&buffer[..accepted]);
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.flush_error {
            Some(kind) => Err(io::Error::from(kind)),
            None => Ok(()),
        }
    }
}

/// A writer whose every call answers `outcome`.
struct FailingWriter {
    outcome: fn() -> io::Result<usize>,
}

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        (self.outcome)()
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A reader that interrupts once before delivering `bytes`, then optionally
/// fails instead of reporting end of file.
struct AwkwardReader {
    bytes: Vec<u8>,
    offset: usize,
    interrupt_next: Cell<bool>,
    fail_at_end: bool,
}

impl AwkwardReader {
    fn new(bytes: &[u8], fail_at_end: bool) -> Self {
        Self {
            bytes: bytes.to_vec(),
            offset: 0,
            interrupt_next: Cell::new(true),
            fail_at_end,
        }
    }
}

impl Read for AwkwardReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.interrupt_next.replace(false) {
            return Err(io::Error::from(ErrorKind::Interrupted));
        }
        if self.offset == self.bytes.len() {
            if self.fail_at_end {
                return Err(io::Error::other("source went away"));
            }
            return Ok(0);
        }
        let take = buffer.len().min(self.bytes.len() - self.offset);
        buffer[..take].copy_from_slice(&self.bytes[self.offset..self.offset + take]);
        self.offset += take;
        Ok(take)
    }
}

fn payload(bytes: usize) -> Vec<u8> {
    (0..bytes).map(|index| (index % 251) as u8).collect()
}

#[test]
fn short_writes_and_interrupts_are_retried_until_every_byte_lands() {
    let source = payload(100);
    let mut reader = AwkwardReader::new(&source, false);
    let mut writer = StubbornWriter {
        chunk: 3,
        ..StubbornWriter::default()
    };
    let mut buffer = [0u8; 16];
    let total = stream_bytes(&mut reader, &mut writer, &mut buffer, &NeverCancelled)
        .expect("short writes are not a failure");
    assert_eq!(total, 100);
    assert_eq!(writer.written, source, "every byte lands in order");
}

#[test]
fn a_write_that_makes_no_progress_is_a_short_write() {
    let mut reader = &payload(64)[..];
    let mut writer = FailingWriter { outcome: || Ok(0) };
    let mut buffer = [0u8; 16];
    let (category, detail) = stream_bytes(&mut reader, &mut writer, &mut buffer, &NeverCancelled)
        .expect_err("a writer that never advances cannot complete");
    assert_eq!(category, CopyErrorCategory::ShortWrite);
    assert!(detail.contains("no progress"), "{detail}");
}

#[test]
fn a_write_zero_error_is_a_short_write() {
    let mut reader = &payload(64)[..];
    let mut writer = FailingWriter {
        outcome: || Err(io::Error::from(ErrorKind::WriteZero)),
    };
    let mut buffer = [0u8; 16];
    let (category, _) = stream_bytes(&mut reader, &mut writer, &mut buffer, &NeverCancelled)
        .expect_err("a write-zero error cannot complete");
    assert_eq!(category, CopyErrorCategory::ShortWrite);
}

#[test]
fn other_write_failures_are_destination_failures_not_unsupported() {
    let mut reader = &payload(64)[..];
    let mut writer = FailingWriter {
        outcome: || Err(io::Error::from(ErrorKind::PermissionDenied)),
    };
    let mut buffer = [0u8; 16];
    let (category, _) = stream_bytes(&mut reader, &mut writer, &mut buffer, &NeverCancelled)
        .expect_err("a permission failure stops the copy");
    assert_eq!(category, CopyErrorCategory::DestinationUnwritable);
    assert_eq!(
        destination_category(&io::Error::from(ErrorKind::StorageFull)),
        CopyErrorCategory::DestinationUnwritable,
        "a full disk is an error, never `unsupported`"
    );
    assert_eq!(
        destination_category(&io::Error::from(ErrorKind::WriteZero)),
        CopyErrorCategory::ShortWrite
    );
}

#[test]
fn a_writer_that_claims_more_than_it_was_offered_does_not_panic() {
    let mut reader = &payload(32)[..];
    let mut writer = FailingWriter {
        outcome: || Ok(usize::MAX),
    };
    let mut buffer = [0u8; 16];
    let total = stream_bytes(&mut reader, &mut writer, &mut buffer, &NeverCancelled)
        .expect("an over-claiming writer is tolerated, not a panic");
    assert_eq!(total, 32);
}

#[test]
fn read_failures_are_source_failures_and_interrupts_are_retried() {
    let mut reader = AwkwardReader::new(&payload(48), true);
    let mut writer = StubbornWriter {
        chunk: 64,
        ..StubbornWriter::default()
    };
    let mut buffer = [0u8; 16];
    let (category, detail) = stream_bytes(&mut reader, &mut writer, &mut buffer, &NeverCancelled)
        .expect_err("a source read failure stops the copy");
    assert_eq!(category, CopyErrorCategory::SourceUnreadable);
    assert!(detail.contains("source went away"), "{detail}");
    assert_eq!(
        writer.written.len(),
        48,
        "the interrupted read was retried, not treated as end of file"
    );
}

#[test]
fn a_failing_flush_is_reported() {
    let mut reader = &payload(8)[..];
    let mut writer = StubbornWriter {
        chunk: 64,
        flush_error: Some(ErrorKind::StorageFull),
        ..StubbornWriter::default()
    };
    let mut buffer = [0u8; 16];
    let (category, detail) = stream_bytes(&mut reader, &mut writer, &mut buffer, &NeverCancelled)
        .expect_err("an unchecked flush failure would hide a lost write");
    assert_eq!(category, CopyErrorCategory::DestinationUnwritable);
    assert!(detail.contains("flush"), "{detail}");
}

#[test]
fn cancellation_is_polled_between_chunks_and_never_before_the_first() {
    let flag = CancelFlag::new();
    flag.cancel();
    // One chunk: the entry is small enough to finish in a single buffered
    // write, so an already-cancelled copy still completes that entry rather
    // than leaving it half written. The per-entry poll is the traversal's.
    let mut reader = &payload(8)[..];
    let mut writer = StubbornWriter {
        chunk: 64,
        ..StubbornWriter::default()
    };
    let mut buffer = [0u8; 16];
    let total = stream_bytes(&mut reader, &mut writer, &mut buffer, &flag)
        .expect("a single-chunk entry is not interrupted mid-entry");
    assert_eq!(total, 8);

    // More than one chunk: the poll between them stops the copy.
    let mut reader = &payload(64)[..];
    let mut writer = StubbornWriter {
        chunk: 64,
        ..StubbornWriter::default()
    };
    let (category, detail) = stream_bytes(&mut reader, &mut writer, &mut buffer, &flag)
        .expect_err("cancellation between buffered writes stops the copy");
    assert_eq!(category, CopyErrorCategory::Cancelled);
    assert!(detail.contains("16 bytes"), "{detail}");
    assert_eq!(writer.written.len(), 16, "only the first chunk was written");
}

#[test]
fn a_new_path_resolves_through_its_nearest_existing_ancestor() {
    let tree = TempTree::new("resolve");
    let existing = fs::canonicalize(tree.path()).expect("the temp tree resolves");
    assert_eq!(
        resolve_new_path(&tree.path().join("not-yet/created")),
        Some(existing.join("not-yet").join("created")),
        "a destination that does not exist yet is still comparable"
    );
    assert_eq!(resolve_new_path(tree.path()), Some(existing));
    assert_eq!(
        resolve_new_path(Path::new("relative-with-no-ancestor-xyzzy")),
        fs::canonicalize(".")
            .ok()
            .map(|cwd| cwd.join("relative-with-no-ancestor-xyzzy")),
        "a relative path resolves through the current directory"
    );
}

#[test]
fn a_temporary_name_is_a_sibling_of_the_entry_and_unique_per_entry() {
    let request = CopyRequest {
        source: PathBuf::from("/source"),
        destination: PathBuf::from("/destination"),
        exclusions: Vec::new(),
        mode: CopyMode::OrdinaryOnly,
    };
    let mut run = CopyRun {
        request: &request,
        cancellation: &NeverCancelled,
        report: CopyReport::default(),
        buffer: Vec::new(),
        temporaries: 0,
        plan: Plan::scripted(Attempt::Skip),
        native_fallback_noted: false,
    };
    let first = run.temporary_path(Path::new("/destination/dir/a.txt"));
    let second = run.temporary_path(Path::new("/destination/dir/b.txt"));
    assert_eq!(
        first.parent(),
        Some(Path::new("/destination/dir")),
        "the temporary is a sibling, so the rename stays on one filesystem"
    );
    assert_ne!(first, second, "each entry gets its own temporary name");
    for path in [&first, &second] {
        let name = path.file_name().expect("a temporary has a name");
        let name = name.to_string_lossy();
        assert!(name.starts_with(".gwz-refcopy."), "{name}");
        assert!(name.ends_with(".tmp"), "{name}");
    }
}

/// A copy that will make no native attempt says so once, up front; a copy
/// that will make them promises nothing until an attempt is actually
/// rejected, which is `note_native_fallback`'s warning, not this one.
#[test]
fn only_a_copy_that_makes_no_native_attempt_opens_with_the_unavailable_warning() {
    let unavailable = opening_warnings(Plan {
        attempt: Attempt::Skip,
        unavailable: Some("no mechanism here"),
    });
    assert_eq!(unavailable.len(), 2);
    assert_eq!(unavailable[0].kind, CopyWarningKind::NativeUnavailable);
    assert_eq!(unavailable[0].detail, "no mechanism here");
    assert_eq!(
        unavailable[0].path,
        PathBuf::new(),
        "the warning names the copy"
    );
    assert_eq!(
        unavailable[1].kind,
        CopyWarningKind::AncillaryMetadataUnsupported
    );

    for plan in [
        // An ordinary-only copy was never promised a native path.
        Plan::scripted(Attempt::Skip),
        Plan::scripted(Attempt::Native),
    ] {
        let warnings = opening_warnings(plan);
        assert_eq!(warnings.len(), 1, "{plan:?}: {warnings:?}");
        assert_eq!(
            warnings[0].kind,
            CopyWarningKind::AncillaryMetadataUnsupported
        );
    }
}

#[cfg(unix)]
#[test]
fn a_metadata_failure_is_reported_as_an_error_not_as_unsupported() {
    let tree = TempTree::new("r-meta");
    let permissions = fs::symlink_metadata(tree.path())
        .expect("the fixture exists")
        .permissions();
    let request = CopyRequest {
        source: tree.path().to_path_buf(),
        // Nothing was created here, so applying the directory's mode fails.
        destination: tree.path().join("absent"),
        exclusions: Vec::new(),
        mode: CopyMode::Auto,
    };
    let mut run = CopyRun {
        request: &request,
        cancellation: &NeverCancelled,
        report: CopyReport::default(),
        buffer: Vec::new(),
        temporaries: 0,
        plan: Plan::scripted(Attempt::Skip),
        native_fallback_noted: false,
    };
    let frame = Frame {
        relative: PathBuf::from("gone"),
        names: Vec::new().into_iter(),
        permissions: Some(permissions),
    };
    let (path, category, detail) = run
        .finish_directory(&frame)
        .expect_err("a mode that cannot be applied is a failure");
    assert_eq!(path, PathBuf::from("gone"));
    assert_eq!(category, CopyErrorCategory::MetadataFailed);
    assert!(detail.contains("permissions"), "{detail}");
}
