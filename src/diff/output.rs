//! The `diff.output` producer — renders the planned manifest into the log.
//!
//! [`run_producer`] is the byte-emitting half of D3. It walks the resolved
//! per-repo manifests in **final display order** (root first, then members in
//! manifest order — plan §"Root and member ordering") and, for each changed
//! file, emits a small run of [`DiffOutputRecord`]s through the D3
//! [`DiffLog`](super::log_service::DiffLog):
//!
//! - `file_started` — the per-file boundary, always emitted (D0 ruling #4), so a
//!   machine consumer frames per-file output without parsing patch text.
//! - `patch_bytes` — the workspace-relative patch for the file, produced by the
//!   D4 [`render_entry`](crate::diff::render::render_entry) seam.
//! - `file_finished` — the closing boundary.
//!
//! On completion the producer [`seal`](super::log_service::DiffLog::seal)s the
//! log so drained readers see `eof`; a render failure
//! [`close`](super::log_service::DiffLog::close)s it with the error so readers
//! see `failed`. Between files it polls
//! [`should_stop`](super::log_service::DiffLog::should_stop): a `ProducerStop`
//! (last reader gone / pager quit) halts rendering and releases state (D6).
//!
//! # Stale files (worktree races, AD4)
//!
//! The manifest is captured when `diff` accepted the request; the bytes are
//! rendered lazily here. If a worktree file changed in between, the freshly
//! narrowed render no longer describes the planned entry. Rather than silently
//! emit a *different* patch, the producer emits a `stale_file` record
//! (`kind=stale_file, stale=true`) and continues — non-fatal (D0 invariant 8).
//! Staleness is detected by re-running the narrowed diff for the entry and
//! comparing its identity (status + paths + modes + binary flag) against the
//! planned entry; a mismatch (including "the file no longer differs") is stale.

use git2::Repository;

use crate::diff::render::{RenderOptions, render_entry, render_raw_entry};
use crate::diff::{RepoDiffComparison, RepoDiffEntry, RepoDiffOptions, diff_repo};
use crate::model::ModelResult;
use crate::protocol::generated::{
    DiffFileEntry, DiffOutputFormat, DiffOutputRecord, DiffOutputRecordKind, DiffRepoScope,
};

use super::log_service::DiffLog;

/// One repo's rendering job: the opened repository, its resolved comparison and
/// backend options, the per-repo render scope, and the planned manifest entries
/// paired with their wire [`DiffFileEntry`] (for record correlation) in libgit2
/// order. The handler builds one per planned target, root first.
pub struct ProducerTarget {
    pub repo: Repository,
    pub comparison: RepoDiffComparison,
    pub options: RepoDiffOptions,
    pub render: RenderOptions,
    pub scope: DiffRepoScope,
    /// The planned entries in manifest order, each with its wire projection.
    pub entries: Vec<ProducerEntry>,
}

/// One planned changed file: the internal entry (drives rendering + staleness)
/// and its wire projection (the `file_id`, echoed `entry`, and correlation
/// `scope`/`file_id` on every record for this file).
pub struct ProducerEntry {
    pub entry: RepoDiffEntry,
    pub wire: DiffFileEntry,
}

/// Whether to echo the full manifest entry on each file's records (D0 ruling #3
/// / `DiffOptions.echo_manifest_entries`). Off by default: correlate by
/// `scope`+`file_id`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProducerOptions {
    pub echo_manifest_entries: bool,
    pub format: DiffOutputFormat,
}

/// Render every target into `log`, in order, then seal. A render error closes the
/// log with the error (readers see `failed`) and returns the error to the caller.
/// A `ProducerStop` between files stops early and leaves the log as the engine
/// left it (the last reader's departure already closed it).
pub fn run_producer(
    log: &DiffLog,
    targets: Vec<ProducerTarget>,
    opts: ProducerOptions,
) -> ModelResult<()> {
    for target in targets {
        for pe in &target.entries {
            // Cancellation: a ProducerStop (pager quit / last reader gone) halts
            // rendering between files and releases state (D6).
            if log.should_stop() {
                return Ok(());
            }

            match render_one(&target, pe, opts) {
                Ok(records) => {
                    for record in records {
                        log.push(encode_record(&record));
                    }
                }
                Err(err) => {
                    // A render failure is fatal for the log: close it failed so
                    // readers see `failed` with the message, then propagate.
                    log.close(Some(err.message.clone()));
                    return Err(err);
                }
            }
        }
    }
    log.seal();
    Ok(())
}

/// Build the record run for one file: `file_started`, then either `patch_bytes`
/// or a single `stale_file` record when the worktree raced, then `file_finished`.
fn render_one(
    target: &ProducerTarget,
    pe: &ProducerEntry,
    opts: ProducerOptions,
) -> ModelResult<Vec<DiffOutputRecord>> {
    let entry_wire = if opts.echo_manifest_entries {
        Some(pe.wire.clone())
    } else {
        None
    };
    let scope = target.scope.clone();
    let file_id = pe.wire.file_id.clone();

    let mut records = vec![boundary_record(
        DiffOutputRecordKind::FileStarted,
        &scope,
        &file_id,
        &entry_wire,
    )];

    if let Some(stale) = detect_stale(target, &pe.entry)? {
        records.push(DiffOutputRecord {
            kind: DiffOutputRecordKind::StaleFile,
            scope: Some(scope.clone()),
            file_id: Some(file_id.clone()),
            entry: entry_wire.clone(),
            data: None,
            stale: Some(true),
            diagnostic: Some(stale),
        });
    } else {
        let bytes = match opts.format {
            DiffOutputFormat::Raw => render_raw_entry(
                &target.repo,
                &target.comparison,
                &target.options,
                &pe.entry,
                &target.render,
            )?,
            _ => render_entry(
                &target.repo,
                &target.comparison,
                &target.options,
                &pe.entry,
                &target.render,
            )?,
        };
        records.push(DiffOutputRecord {
            kind: DiffOutputRecordKind::PatchBytes,
            scope: Some(scope.clone()),
            file_id: Some(file_id.clone()),
            entry: entry_wire.clone(),
            data: Some(bytes),
            stale: None,
            diagnostic: None,
        });
    }

    records.push(boundary_record(
        DiffOutputRecordKind::FileFinished,
        &scope,
        &file_id,
        &entry_wire,
    ));
    Ok(records)
}

/// Re-run the narrowed diff for this entry and compare identity against the
/// planned entry. Returns `Some(reason)` when the worktree raced (identity no
/// longer matches, or the file no longer differs), else `None`.
fn detect_stale(target: &ProducerTarget, planned: &RepoDiffEntry) -> ModelResult<Option<String>> {
    let mut narrowed = target.options.clone();
    narrowed.pathspecs = entry_pathspecs(planned);
    let fresh = diff_repo(&target.repo, &target.comparison, &narrowed)?;

    // The narrowed diff should describe exactly this one file. Find the entry
    // whose primary path matches; a rename may reorder, so match on the path set.
    let current = fresh.entries.iter().find(|e| same_paths(e, planned));

    match current {
        None => Ok(Some(format!(
            "worktree changed: '{}' no longer differs",
            planned.primary_path().unwrap_or("<unknown>")
        ))),
        Some(current) if !same_identity(current, planned) => Ok(Some(format!(
            "worktree changed: '{}' no longer matches the planned diff",
            planned.primary_path().unwrap_or("<unknown>")
        ))),
        Some(_) => Ok(None),
    }
}

/// The repo-relative pathspecs that narrow a diff to one entry (both sides for a
/// rename so the pair is detectable). Mirrors the render seam's narrowing.
fn entry_pathspecs(entry: &RepoDiffEntry) -> Vec<String> {
    let mut specs: Vec<String> = Vec::new();
    if let Some(old) = &entry.old_path {
        specs.push(old.clone());
    }
    if let Some(new) = &entry.new_path
        && entry.old_path.as_deref() != Some(new.as_str())
    {
        specs.push(new.clone());
    }
    specs
}

/// Same file(s) by path set — old/new both match.
fn same_paths(a: &RepoDiffEntry, b: &RepoDiffEntry) -> bool {
    a.old_path == b.old_path && a.new_path == b.new_path
}

/// Same rendered identity: status, both paths, both modes, and the binary flag.
/// Similarity/line-stats are allowed to drift (they do not change the patch's
/// shape enough to count as a race for v0); a status/mode/path/binary change is
/// a genuine worktree race.
fn same_identity(a: &RepoDiffEntry, b: &RepoDiffEntry) -> bool {
    a.status == b.status
        && a.old_path == b.old_path
        && a.new_path == b.new_path
        && a.old_mode == b.old_mode
        && a.new_mode == b.new_mode
        && a.is_binary == b.is_binary
}

fn boundary_record(
    kind: DiffOutputRecordKind,
    scope: &DiffRepoScope,
    file_id: &str,
    entry: &Option<DiffFileEntry>,
) -> DiffOutputRecord {
    DiffOutputRecord {
        kind,
        scope: Some(scope.clone()),
        file_id: Some(file_id.to_owned()),
        entry: entry.clone(),
        data: None,
        stale: None,
        diagnostic: None,
    }
}

/// Taut-encode a record (D17: the log payload is the append-type message already
/// taut-encoded). The reader decodes with `DiffOutputRecord::from_cbor(decode(&p))`.
pub fn encode_record(record: &DiffOutputRecord) -> Vec<u8> {
    crate::cbor::encode(&record.to_cbor())
}

/// Decode a record from a log payload (the reader half of [`encode_record`]).
///
/// Panics on a payload [`encode_record`] could not have produced: the log is
/// written and read by the same crate, so a decode failure means log
/// corruption. (Matches the pre-0.8.0 generated codec, which panicked inside
/// `from_cbor` on malformed wire data.)
pub fn decode_record(payload: &[u8]) -> DiffOutputRecord {
    DiffOutputRecord::from_cbor(&crate::cbor::decode(payload))
        .expect("diff log record failed to decode; log is corrupt")
}

#[cfg(test)]
mod tests {
    //! A genuine worktree-race test for the producer: capture a manifest entry,
    //! mutate the worktree so the planned entry no longer describes a real diff,
    //! then run the producer and assert a `stale_file` record (non-fatal) is
    //! emitted and the log seals normally.

    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use git2::Repository;

    use crate::diff::render::{PrefixPolicy, RenderOptions, ScopeRender};
    use crate::diff::{
        ComparisonSpec, DiffLogRegistry, LogReadRequest, RepoDiffOptions, diff_repo,
        resolve_comparison,
    };
    use crate::protocol::generated::{
        DiffFileEntry, DiffOutputRecordKind, DiffRepoScope, DiffStatus,
    };

    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn temp() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("gwz-core-stale-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn worktree_race_emits_stale_file_and_seals() {
        let dir = temp();
        Repository::init(&dir).unwrap();
        git(&dir, &["config", "user.name", "GWZ"]);
        git(&dir, &["config", "user.email", "gwz@example.invalid"]);
        fs::write(dir.join("a.txt"), b"one\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-m", "init"]);
        // Dirty the worktree, then build the manifest capturing the change.
        fs::write(dir.join("a.txt"), b"two\n").unwrap();

        let repo = Repository::open(&dir).unwrap();
        let options = RepoDiffOptions::full_repo();
        let comparison = resolve_comparison(&repo, &ComparisonSpec::default()).unwrap();
        let manifest = diff_repo(&repo, &comparison, &options).unwrap();
        assert_eq!(manifest.entries.len(), 1);
        let planned = manifest.entries[0].clone();

        // THE RACE: revert the file so the planned modification no longer differs.
        fs::write(dir.join("a.txt"), b"one\n").unwrap();

        let scope = DiffRepoScope {
            root: Some(true),
            ..Default::default()
        };
        let wire = DiffFileEntry {
            file_id: "@root#0".to_owned(),
            scope: scope.clone(),
            status: DiffStatus::Modified,
            new_path: planned.new_path.clone(),
            old_path: planned.old_path.clone(),
            ..Default::default()
        };
        let target = ProducerTarget {
            repo,
            comparison,
            options,
            render: RenderOptions {
                scope: ScopeRender::root(PrefixPolicy::Default),
                ..Default::default()
            },
            scope,
            entries: vec![ProducerEntry {
                entry: planned,
                wire,
            }],
        };

        let registry = DiffLogRegistry::new();
        let (log_id, log) = registry.create();
        run_producer(&log, vec![target], ProducerOptions::default()).unwrap();

        let resp = registry
            .read(
                &log_id,
                &LogReadRequest {
                    stream_id: "s".to_owned(),
                    max_records: Some(10),
                    timeout_ms: Some(0),
                    ..Default::default()
                },
            )
            .unwrap();
        let kinds: Vec<DiffOutputRecordKind> = resp
            .records
            .iter()
            .map(|r| decode_record(&r.payload).kind)
            .collect();
        assert!(
            kinds.contains(&DiffOutputRecordKind::StaleFile),
            "expected a stale_file record, got {kinds:?}"
        );
        // Non-fatal: no patch bytes for the raced file, and the log seals to eof.
        assert!(!kinds.contains(&DiffOutputRecordKind::PatchBytes));
        let tail = registry
            .read(
                &log_id,
                &LogReadRequest {
                    stream_id: "s".to_owned(),
                    cursor: Some(resp.next_cursor),
                    timeout_ms: Some(0),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(tail.state, crate::diff::LogReadState::Eof);

        let _ = fs::remove_dir_all(&dir);
    }
}
