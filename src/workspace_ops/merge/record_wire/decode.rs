use serde_yaml::Value;

use super::super::MergeOperationRecord;
use super::header::{
    HeaderClassificationError, InstalledMergeRecordVersions, MergeRecordDispatch,
    MergeRecordHeader, classify_merge_record_header, read_merge_record_header,
};
use super::raw_yaml::{StrictYamlError, parse_strict_yaml};

use super::super::model::v1::{CanonicalMergeRecord, MergeOperationRecordV1, validate_v1_record};
use super::unknown_fields::{UnknownFieldManifest, UnknownFieldManifestError};
use crate::model::ModelError;

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "A1 activation: reached only by this tree's own suites; the compile gate's blanket `dead_code` allowance expired with the activation, so the residue is named item by item."
)]
pub(crate) struct DecodedV0Record {
    raw: Value,
    header: MergeRecordHeader,
    record: MergeOperationRecord,
    unknown_fields: UnknownFieldManifest,
}

impl DecodedV0Record {
    pub(crate) fn into_production_parts(self) -> (Value, MergeRecordHeader, MergeOperationRecord) {
        (self.raw, self.header, self.record)
    }

    pub(crate) fn record(&self) -> &MergeOperationRecord {
        &self.record
    }

    pub(crate) fn unknown_fields(&self) -> &UnknownFieldManifest {
        &self.unknown_fields
    }

    #[cfg(test)]
    pub(crate) fn raw(&self) -> &Value {
        &self.raw
    }
}

#[allow(
    dead_code,
    reason = "A1 activation: reached only by this tree's own suites; the compile gate's blanket `dead_code` allowance expired with the activation, so the residue is named item by item."
)]
#[derive(Debug)]
pub(crate) struct DecodedV1Record {
    pub(crate) raw: Value,
    pub(crate) header: MergeRecordHeader,
    pub(crate) record: MergeOperationRecordV1,
    pub(crate) canonical: CanonicalMergeRecord,
    pub(crate) unknown_fields: UnknownFieldManifest,
}

#[derive(Debug)]
pub(crate) enum RecordDecodeError {
    Raw(StrictYamlError),
    Header(HeaderClassificationError),
    Body {
        header: MergeRecordHeader,
        detail: String,
    },
    Validation {
        header: MergeRecordHeader,
        error: ModelError,
    },
    UnknownFields {
        header: MergeRecordHeader,
        error: UnknownFieldManifestError,
    },
}

/// The A1 envelope registry dispatch (compatibility contract §1).
///
/// One header classification decides the body decoder: v0 → the v0 model,
/// v1 → the v1 canonical decode, and every other allocated-but-uninstalled or
/// unknown pair keeps its frozen typed projection from
/// `classify_merge_record_header`.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "A1 activation: reached only by this tree's own suites; the compile gate's blanket `dead_code` allowance expired with the activation, so the residue is named item by item."
)]
pub(crate) enum DecodedRecord {
    V0(Box<DecodedV0Record>),
    V1(Box<DecodedV1Record>),
}

pub(crate) fn decode_production(bytes: &[u8]) -> Result<DecodedRecord, RecordDecodeError> {
    let document = parse_strict_yaml(bytes).map_err(RecordDecodeError::Raw)?;
    let header = read_merge_record_header(&document).map_err(|reason| {
        RecordDecodeError::Header(HeaderClassificationError::Malformed(reason))
    })?;
    let dispatch = classify_merge_record_header(&header, InstalledMergeRecordVersions::PRODUCTION)
        .map_err(RecordDecodeError::Header)?;
    let raw = document.into_root();
    match dispatch {
        MergeRecordDispatch::V0 => {
            decode_v0_body(raw, header).map(|decoded| DecodedRecord::V0(Box::new(decoded)))
        }
        MergeRecordDispatch::V1 => {
            decode_v1_body(raw, header).map(|decoded| DecodedRecord::V1(Box::new(decoded)))
        }
    }
}

/// The ARCHIVED leaf's decoder, over both installed envelopes, projected onto
/// the common record half retention reads.
///
/// Retention read the archive with `decode_production_v0`, whose `V0_ONLY`
/// installed set classifies a v1 envelope `Unsupported` below — the live
/// refusal, not the unreachable mirror arm after it — so no v1 archive could be
/// collected. The registry dispatch (contract §1) picks the body decoder
/// instead, running the v1 decode and `validate_v1_record` as `decode_archived`
/// does. Retention reads only the half the two envelopes hold IDENTICALLY,
/// field for field and type for type; the v1-only halves are its
/// archive-PROJECTION inputs. Envelope fields are carried verbatim.
pub(crate) fn decode_archived_common(
    bytes: &[u8],
) -> Result<(Value, MergeRecordHeader, MergeOperationRecord), RecordDecodeError> {
    Ok(match decode_production(bytes)? {
        DecodedRecord::V0(decoded) => decoded.into_production_parts(),
        DecodedRecord::V1(decoded) => {
            let DecodedV1Record {
                raw,
                header,
                record,
                ..
            } = *decoded;
            (
                raw,
                header,
                MergeOperationRecord {
                    schema: record.schema,
                    record_schema_version: record.record_schema_version,
                    writer_version: record.writer_version,
                    workspace_id: record.workspace_id,
                    merge_id: record.merge_id,
                    operation_id: record.operation_id,
                    state: record.state,
                    source_ref: record.source_ref,
                    mode: record.mode,
                    created_at: record.created_at,
                    baseline: record.baseline,
                    selected_targets: record.selected_targets,
                    participants: record.participants,
                    publication: record.publication,
                    operation_drift: record.operation_drift,
                    extensions: record.extensions,
                },
            )
        }
    })
}

/// The v0 record store's decoder. The store owns only v0 bodies, so it
/// installs v0 alone; a v1 envelope classifies `UnsupportedRecordVersion`
/// here and the dispatch routes that record to the v1 lifecycle before this
/// decoder is ever reached in production.
pub(crate) fn decode_production_v0(bytes: &[u8]) -> Result<DecodedV0Record, RecordDecodeError> {
    let document = parse_strict_yaml(bytes).map_err(RecordDecodeError::Raw)?;
    let header = read_merge_record_header(&document).map_err(|reason| {
        RecordDecodeError::Header(HeaderClassificationError::Malformed(reason))
    })?;
    match classify_merge_record_header(&header, InstalledMergeRecordVersions::V0_ONLY)
        .map_err(RecordDecodeError::Header)?
    {
        MergeRecordDispatch::V0 => {}
        // L13 / [P3-7]: the typed twin of `decode_production_v1`'s mirror
        // arm. The v0-only installed set cannot classify a v1 envelope as
        // `V1`, so this is unreachable today — but the panic audit condemned
        // exactly this shape, and a typed refusal costs nothing and keeps the
        // decoder total.
        MergeRecordDispatch::V1 => {
            return Err(RecordDecodeError::Body {
                header,
                detail: "the v0 decoder received a v1 record".to_owned(),
            });
        }
    }
    decode_v0_body(document.into_root(), header)
}

fn decode_v0_body(
    raw: Value,
    header: MergeRecordHeader,
) -> Result<DecodedV0Record, RecordDecodeError> {
    let record = serde_yaml::from_value(raw.clone()).map_err(|error| RecordDecodeError::Body {
        header: header.clone(),
        detail: error.to_string(),
    })?;
    let unknown_fields = UnknownFieldManifest::extract_v0(&raw).map_err(|error| {
        RecordDecodeError::UnknownFields {
            header: header.clone(),
            error,
        }
    })?;
    Ok(DecodedV0Record {
        raw,
        header,
        record,
        unknown_fields,
    })
}

pub(crate) fn decode_production_v1(bytes: &[u8]) -> Result<DecodedV1Record, RecordDecodeError> {
    let document = parse_strict_yaml(bytes).map_err(RecordDecodeError::Raw)?;
    let header = read_merge_record_header(&document).map_err(|reason| {
        RecordDecodeError::Header(HeaderClassificationError::Malformed(reason))
    })?;
    match classify_merge_record_header(&header, InstalledMergeRecordVersions::PRODUCTION)
        .map_err(RecordDecodeError::Header)?
    {
        MergeRecordDispatch::V1 => {}
        MergeRecordDispatch::V0 => {
            return Err(RecordDecodeError::Body {
                header,
                detail: "the v1 decoder received a v0 record".to_owned(),
            });
        }
    }
    decode_v1_body(document.into_root(), header)
}

fn decode_v1_body(
    raw: Value,
    header: MergeRecordHeader,
) -> Result<DecodedV1Record, RecordDecodeError> {
    let record: MergeOperationRecordV1 =
        serde_yaml::from_value(raw.clone()).map_err(|error| RecordDecodeError::Body {
            header: header.clone(),
            detail: error.to_string(),
        })?;
    let validated =
        validate_v1_record(record.clone()).map_err(|error| RecordDecodeError::Validation {
            header: header.clone(),
            error,
        })?;
    let unknown_fields = UnknownFieldManifest::extract_v1(&raw).map_err(|error| {
        RecordDecodeError::UnknownFields {
            header: header.clone(),
            error,
        }
    })?;
    Ok(DecodedV1Record {
        raw,
        header,
        record,
        canonical: CanonicalMergeRecord::from(validated),
        unknown_fields,
    })
}
