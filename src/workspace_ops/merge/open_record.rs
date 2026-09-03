//! Open-record occupancy: which lifecycle owns the one record under `.gwz/merge`,
//! decided from its envelope alone.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §2).** 0.14 has one merge lifecycle, v1, and
//! it must not "understand a v0 partial merge". So classification here reads
//! the record's YAML **header** — `schema` and `record_schema_version` — and
//! stops. It never constructs a v0 body. A v0 envelope is not a merge that
//! this binary can continue, abort, migrate or project; it is also not
//! nothing. It is a third occupancy — an **open operation** — and every merge
//! verb and every gated command answers it with one sentence:
//!
//! ```text
//! this is a pre-0.14 merge; use gwz 0.13.0 (the last release before 0.14) to continue or abort
//! ```
//!
//! The open-operation remedy the gates print for an open **v1** merge ("use
//! merge status, merge continue, or merge abort") is **suppressed** for a v0
//! envelope: under 0.14 that remedy is false, because all three of those verbs
//! refuse. `v0.13.0` is the whole remedy (charter §2, revision 4 L-P3-3).
//!
//! Archived `done/` v0 records are a different question and keep their
//! read-only projection — see `record_wire::archive` and charter §5.

use std::fs;
use std::path::{Path, PathBuf};

use super::RecordVersion;
use super::model::v1::MergeOperationRecordV1;
use super::record_wire::{
    HeaderClassificationError, InstalledMergeRecordVersions, MergeRecordDispatch,
    classify_merge_record_header, read_merge_record_header,
};
use super::store::{MERGE_DIR, RecordLocation, location_unreadable, record_files, validate_merge_id};
use crate::model::{ErrorCode, ModelError, ModelResult};

/// The one open record under a root, named by its envelope.
///
/// There is no body here and no `AdaptationPrecheck`: the A1 adapter that
/// field gated is deleted (charter §2, "No whitelist. No `open_v0`.").
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenRecordEnvelope {
    pub(crate) root: PathBuf,
    pub(crate) merge_id: String,
    pub(crate) version: RecordVersion,
}

impl OpenRecordEnvelope {
    /// Whether this occupancy is a merge **this binary** can act on.
    pub(crate) fn is_v1(&self) -> bool {
        self.version == RecordVersion::V1
    }

    /// The refusal a v0 occupancy owes every caller, or `Ok(())` for v1.
    pub(crate) fn refuse_if_pre_014(&self) -> ModelResult<()> {
        if self.is_v1() {
            Ok(())
        } else {
            Err(pre_014_merge_error())
        }
    }
}

/// The charter §2 sentence, typed.
///
/// `OpenOperation` is the code because that is exactly what the occupancy is:
/// "not a merge lifecycle, not idle". The message carries no merge id — the
/// charter freezes this sentence verbatim, and the id would invite the reader
/// to pass it back to a verb that refuses.
pub(crate) fn pre_014_merge_error() -> ModelError {
    ModelError::new(
        ErrorCode::OpenOperation,
        "this is a pre-0.14 merge; use gwz 0.13.0 (the last release before 0.14) to continue or abort",
    )
}

/// Classify the single open record under `root` from its header alone.
///
/// The header reader and the contract-§1 envelope registry already suffice
/// (`record_wire/header.rs`), and stopping here is the point: charter §2's
/// "Envelope classification does not decode the v0 body" is a property of
/// this function, not a convention its callers keep.
pub(crate) fn classify_open_record(root: &Path) -> ModelResult<Option<OpenRecordEnvelope>> {
    let directory = root.join(MERGE_DIR);
    let paths = record_files(&directory)?;
    let mut found: Vec<OpenRecordEnvelope> = Vec::with_capacity(paths.len());
    for path in paths {
        let merge_id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    format!(
                        "merge record at '{}' is unreadable: record file name is not valid UTF-8",
                        path.display()
                    ),
                )
            })?;
        let bytes = fs::read(&path)
            .map_err(|error| location_unreadable(&path, merge_id, RecordLocation::Open, error))?;
        found.push(OpenRecordEnvelope {
            root: root.to_path_buf(),
            merge_id: merge_id.to_owned(),
            version: classify_bytes(&bytes, merge_id)?,
        });
    }
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        _ => Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!(
                "multiple merge records exist under '{}'",
                directory.display()
            ),
        )),
    }
}

/// The envelope of one record's bytes.
///
/// A malformed or unallocated header is still `MergeRecordUnreadable`, as it
/// was when the store decoded the body: the classification refuses, it does
/// not fall through to "no merge".
fn classify_bytes(bytes: &[u8], merge_id: &str) -> ModelResult<RecordVersion> {
    let document = super::record_wire::parse_strict_yaml(bytes)
        .map_err(|_| envelope_unreadable(merge_id, "record is not a strict YAML document"))?;
    let header = read_merge_record_header(&document)
        .map_err(|reason| envelope_unreadable(merge_id, format!("{reason:?}")))?;
    match classify_merge_record_header(&header, InstalledMergeRecordVersions::PRODUCTION) {
        Ok(MergeRecordDispatch::V0) => Ok(RecordVersion::V0),
        Ok(MergeRecordDispatch::V1) => Ok(RecordVersion::V1),
        Err(HeaderClassificationError::Malformed(reason)) => {
            Err(envelope_unreadable(merge_id, format!("{reason:?}")))
        }
        Err(HeaderClassificationError::Unsupported { header, .. }) => {
            Err(super::store::unsupported_open_envelope(merge_id, &header))
        }
    }
}

fn envelope_unreadable(merge_id: &str, detail: impl std::fmt::Display) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecordUnreadable,
        format!("merge record '{merge_id}' is unreadable: {detail}"),
    )
}

/// Search ancestors for an open record's envelope before parsing the manifest
/// or lock, so recovery stays reachable when the workspace's own GWZ metadata
/// is conflicted or temporarily invalid.
pub(crate) fn discover_open_envelope_before_manifest(
    start: &Path,
) -> ModelResult<Option<OpenRecordEnvelope>> {
    let mut current = if start.is_file() {
        start.parent().unwrap_or(start).to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        if current.join(MERGE_DIR).try_exists().unwrap_or(true)
            && let Some(found) = classify_open_record(&current)?
        {
            return Ok(Some(found));
        }
        // Recovery state belongs to the nearest workspace boundary. Inspect
        // runtime state first so an invalid/conflicted manifest cannot hide an
        // operation, then stop instead of capturing this nested workspace with
        // an enclosing workspace's open merge.
        if current
            .join(crate::workspace::WORKSPACE_MANIFEST)
            .try_exists()
            .unwrap_or(true)
        {
            return Ok(None);
        }
        if !current.pop() {
            return Ok(None);
        }
    }
}

/// The one open record under a root, decoded.
///
/// Opaque on purpose: a caller outside `merge` reads it only through
/// [`MergeStatusRecordView`](super::status::MergeStatusRecordView), the
/// projection every non-merge consumer of merge state already speaks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenMergeRecord(Box<MergeOperationRecordV1>);

impl OpenMergeRecord {
    pub(in crate::workspace_ops) fn view(&self) -> super::status::MergeStatusRecordView<'_> {
        super::status::MergeStatusRecordView::from_v1(&self.0)
    }

    /// Build one from literal YAML, for suites outside `merge` that drive the
    /// view directly rather than through a real merge.
    #[cfg(test)]
    pub(in crate::workspace_ops) fn from_yaml_for_test(yaml: &str) -> Self {
        Self(Box::new(serde_yaml::from_str(yaml).expect("a v1 record")))
    }
}

/// Read the one open record under `root` as a v1 body.
///
/// This is the reader every non-merge consumer of merge state uses — the
/// open-merge gate, the workspace mutation guard, `gwz stage` and `gwz add`.
/// A v0 envelope refuses with the charter §2 sentence **without decoding**;
/// a v1 envelope is decoded and validated exactly as the lifecycle decodes it.
pub(crate) fn discover_open_v1_record(root: &Path) -> ModelResult<Option<OpenMergeRecord>> {
    let Some(open) = classify_open_record(root)? else {
        return Ok(None);
    };
    open.refuse_if_pre_014()?;
    let path = super::store::open_path(root, &open.merge_id);
    let bytes = fs::read(&path).map_err(|error| {
        location_unreadable(&path, &open.merge_id, RecordLocation::Open, error)
    })?;
    let decoded = super::record_wire::decode_production_v1(&bytes)
        .map_err(|error| super::store::open_decode_error(&path, &open.merge_id, error))?;
    validate_merge_id(&decoded.record.merge_id)?;
    if decoded.record.merge_id != open.merge_id {
        return Err(envelope_unreadable(
            &open.merge_id,
            "record id does not match its file name",
        ));
    }
    Ok(Some(OpenMergeRecord(Box::new(decoded.record))))
}

/// One ARCHIVED record, for suites that assert on a finished merge's durable
/// body.
///
/// **M5d.** `FileMergeStore::load` served this before; the store that could
/// load an open OR archived v0 record left with the engine. This is the I2 §7
/// archive projection, over both envelopes, and it is the only durable body a
/// finished merge has.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct ArchivedMergeRecord(super::record_wire::MergeOperationRecordV0);

#[cfg(test)]
impl ArchivedMergeRecord {
    pub(in crate::workspace_ops) fn view(&self) -> super::status::MergeStatusRecordView<'_> {
        super::status::MergeStatusRecordView::from_archived(&self.0)
    }
}

#[cfg(test)]
pub(crate) fn read_archived_record(
    root: &Path,
    merge_id: &str,
) -> ModelResult<ArchivedMergeRecord> {
    validate_merge_id(merge_id)?;
    let path = root
        .join(".gwz/merge/done")
        .join(format!("{merge_id}.yaml"));
    let bytes = fs::read(&path).map_err(|error| {
        ModelError::new(
            ErrorCode::OperationNotFound,
            format!(
                "archived merge record '{merge_id}' was not found at '{}': {error}",
                path.display()
            ),
        )
    })?;
    let (_, _, record) = super::record_wire::decode_archived_common(&bytes).map_err(|_| {
        ModelError::new(
            ErrorCode::ArchivedRecordUnreadable,
            format!("archived merge record '{merge_id}' is unreadable"),
        )
    })?;
    Ok(ArchivedMergeRecord(record))
}
