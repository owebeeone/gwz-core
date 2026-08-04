use serde_yaml::Value;

use super::super::MergeOperationRecord;
use super::header::{
    HeaderClassificationError, InstalledMergeRecordVersions, MergeRecordDispatch,
    MergeRecordHeader, classify_merge_record_header, read_merge_record_header,
};
use super::raw_yaml::{StrictYamlError, parse_strict_yaml};

#[derive(Debug)]
pub(crate) struct DecodedV0Record {
    pub(crate) raw: Value,
    pub(crate) header: MergeRecordHeader,
    pub(crate) record: MergeOperationRecord,
}

#[derive(Debug)]
pub(crate) enum RecordDecodeError {
    Raw(StrictYamlError),
    Header(HeaderClassificationError),
    Body {
        header: MergeRecordHeader,
        detail: String,
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
    Ok(DecodedV0Record {
        raw,
        header,
        record,
    })
}
