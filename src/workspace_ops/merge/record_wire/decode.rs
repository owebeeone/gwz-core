use serde_yaml::Value;

use super::super::MergeOperationRecord;
use super::header::{
    HeaderClassificationError, InstalledMergeRecordVersions, MergeRecordDispatch,
    MergeRecordHeader, classify_merge_record_header, read_merge_record_header,
};
use super::raw_yaml::{StrictYamlError, parse_strict_yaml};

#[cfg(test)]
use super::super::model::v1::{CanonicalMergeRecord, MergeOperationRecordV1, validate_v1_record};
#[cfg(test)]
use super::unknown_fields::{UnknownFieldManifest, UnknownFieldManifestError};
#[cfg(test)]
use crate::model::ModelError;

#[derive(Debug)]
pub(crate) struct DecodedV0Record {
    raw: Value,
    header: MergeRecordHeader,
    record: MergeOperationRecord,
    #[cfg(test)]
    unknown_fields: UnknownFieldManifest,
}

impl DecodedV0Record {
    pub(crate) fn into_production_parts(self) -> (Value, MergeRecordHeader, MergeOperationRecord) {
        (self.raw, self.header, self.record)
    }

    #[cfg(test)]
    pub(crate) fn record(&self) -> &MergeOperationRecord {
        &self.record
    }

    #[cfg(test)]
    pub(crate) fn unknown_fields(&self) -> &UnknownFieldManifest {
        &self.unknown_fields
    }

    #[cfg(test)]
    pub(crate) fn raw(&self) -> &Value {
        &self.raw
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct DecodedV1Record {
    pub(crate) raw: Value,
    pub(crate) header: MergeRecordHeader,
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
    #[cfg(test)]
    Validation {
        header: MergeRecordHeader,
        error: ModelError,
    },
    #[cfg(test)]
    UnknownFields {
        header: MergeRecordHeader,
        error: UnknownFieldManifestError,
    },
}

pub(crate) fn decode_production_v0(bytes: &[u8]) -> Result<DecodedV0Record, RecordDecodeError> {
    let document = parse_strict_yaml(bytes).map_err(RecordDecodeError::Raw)?;
    let header = read_merge_record_header(&document).map_err(|reason| {
        RecordDecodeError::Header(HeaderClassificationError::Malformed(reason))
    })?;
    match classify_merge_record_header(&header, InstalledMergeRecordVersions::PRODUCTION_R3)
        .map_err(RecordDecodeError::Header)?
    {
        MergeRecordDispatch::V0 => {}
        MergeRecordDispatch::V1 => unreachable!("the R3 production decoder does not install v1"),
    }
    let raw = document.into_root();
    let record = serde_yaml::from_value(raw.clone()).map_err(|error| RecordDecodeError::Body {
        header: header.clone(),
        detail: error.to_string(),
    })?;
    #[cfg(test)]
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
        #[cfg(test)]
        unknown_fields,
    })
}

#[cfg(test)]
pub(crate) fn decode_v1_for_r3_tests(bytes: &[u8]) -> Result<DecodedV1Record, RecordDecodeError> {
    let document = parse_strict_yaml(bytes).map_err(RecordDecodeError::Raw)?;
    let header = read_merge_record_header(&document).map_err(|reason| {
        RecordDecodeError::Header(HeaderClassificationError::Malformed(reason))
    })?;
    match classify_merge_record_header(
        &header,
        InstalledMergeRecordVersions::V0_AND_V1_FOR_R3_TESTS,
    )
    .map_err(RecordDecodeError::Header)?
    {
        MergeRecordDispatch::V1 => {}
        MergeRecordDispatch::V0 => {
            return Err(RecordDecodeError::Body {
                header,
                detail: "test v1 decoder received a v0 record".to_owned(),
            });
        }
    }
    let raw = document.into_root();
    let record: MergeOperationRecordV1 =
        serde_yaml::from_value(raw.clone()).map_err(|error| RecordDecodeError::Body {
            header: header.clone(),
            detail: error.to_string(),
        })?;
    let validated = validate_v1_record(record).map_err(|error| RecordDecodeError::Validation {
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
        canonical: CanonicalMergeRecord::from(validated),
        unknown_fields,
    })
}
