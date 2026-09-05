//! The frozen format-1 files: index, clone pointer, allocation marker.
//!
//! Every key is a `gwz_family_model::fields` constant and every enum value
//! is that model's frozen spelling, so a rename here is a format change and
//! not a refactor (design §3). Decoding is strict in both directions that
//! matter: an unknown field and a `schema:` that is not this crate's format
//! version refuse as malformed rather than being accepted or upgraded.

use std::collections::BTreeMap;

use gwz_family_model::{
    AllocationId, CloneMode, FamilyId, FamilyView, INDEX_SCHEMA, MemberKind, MemberName, MemberRow,
    MemberState, POINTER_SCHEMA,
};
use serde::{Deserialize, Serialize};

/// `schema:` value of a clone's allocation marker, format 1. The model
/// freezes the index and pointer schemas; the marker is the store's own
/// ordinary mix-up check, so its schema is owned here and carries the same
/// `/v1` version.
pub(crate) const MARKER_SCHEMA: &str = "gwz.local-clone-allocation/v1";

/// A file that could not be decoded. The caller adds the path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FormatError(String);

impl FormatError {
    fn new(detail: impl Into<String>) -> Self {
        Self(detail.into())
    }

    pub(crate) fn detail(&self) -> &str {
        &self.0
    }
}

fn wrong_schema(found: &str, expected: &str) -> FormatError {
    FormatError::new(format!(
        "`schema: {found}` is not `{expected}` (format version \
         {version}); this file is not in a format this store reads",
        version = gwz_family_model::INDEX_FORMAT_VERSION,
    ))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IndexFile {
    schema: String,
    family_id: String,
    root: RootFile,
    #[serde(default)]
    members: BTreeMap<String, RowFile>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RootFile {
    allocation_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RowFile {
    path: String,
    kind: String,
    state: String,
    allocation_id: String,
    source_path: String,
    mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

/// A clone's pointer to its registering root: the family it belongs to and
/// where that family's index lives (design §3).
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PointerFile {
    schema: String,
    pub(crate) family_id: String,
    pub(crate) root_path: String,
}

/// A clone's ordinary allocation marker: which family allocated this
/// destination, and under which allocation id.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarkerFile {
    schema: String,
    pub(crate) family_id: String,
    pub(crate) allocation_id: String,
}

pub(crate) fn encode_index(view: &FamilyView) -> Result<Vec<u8>, FormatError> {
    let file = IndexFile {
        schema: INDEX_SCHEMA.to_owned(),
        family_id: view.family_id.as_str().to_owned(),
        root: RootFile {
            allocation_id: view.root.allocation_id.as_str().to_owned(),
        },
        members: view
            .members
            .iter()
            .map(|(name, row)| {
                (
                    name.as_str().to_owned(),
                    RowFile {
                        path: row.path.clone(),
                        kind: row.kind.as_str().to_owned(),
                        state: row.state.as_str().to_owned(),
                        allocation_id: row.allocation_id.as_str().to_owned(),
                        source_path: row.source_path.clone(),
                        mode: row.mode.as_str().to_owned(),
                        last_error: row.last_error.clone(),
                    },
                )
            })
            .collect(),
    };
    encode(&file)
}

pub(crate) fn decode_index(bytes: &[u8]) -> Result<FamilyView, FormatError> {
    let file: IndexFile =
        serde_yaml::from_slice(bytes).map_err(|error| FormatError::new(error.to_string()))?;
    if file.schema != INDEX_SCHEMA {
        return Err(wrong_schema(&file.schema, INDEX_SCHEMA));
    }
    let family_id = FamilyId::new(file.family_id)
        .map_err(|error| FormatError::new(format!("`family_id`: {error}")))?;
    let root_allocation = AllocationId::new(file.root.allocation_id)
        .map_err(|error| FormatError::new(format!("`root.allocation_id`: {error}")))?;
    let mut view = FamilyView::founded(family_id, root_allocation);
    for (name, row) in file.members {
        let name = MemberName::parse(&name)
            .map_err(|error| FormatError::new(format!("member key `{name}`: {error}")))?;
        let row = decode_row(&name, row)?;
        view.members.insert(name, row);
    }
    Ok(view)
}

fn decode_row(name: &MemberName, row: RowFile) -> Result<MemberRow, FormatError> {
    let field = |field: &str, value: &str| {
        FormatError::new(format!(
            "row `{name}`: `{field}: {value}` is not a known value"
        ))
    };
    Ok(MemberRow {
        path: row.path,
        kind: MemberKind::parse(&row.kind).ok_or_else(|| field("kind", &row.kind))?,
        state: MemberState::parse(&row.state).ok_or_else(|| field("state", &row.state))?,
        allocation_id: AllocationId::new(row.allocation_id)
            .map_err(|error| FormatError::new(format!("row `{name}`: `allocation_id`: {error}")))?,
        source_path: row.source_path,
        mode: CloneMode::parse(&row.mode).ok_or_else(|| field("mode", &row.mode))?,
        last_error: row.last_error,
    })
}

pub(crate) fn encode_pointer(
    family_id: &FamilyId,
    root_path: &str,
) -> Result<Vec<u8>, FormatError> {
    encode(&PointerFile {
        schema: POINTER_SCHEMA.to_owned(),
        family_id: family_id.as_str().to_owned(),
        root_path: root_path.to_owned(),
    })
}

pub(crate) fn decode_pointer(bytes: &[u8]) -> Result<PointerFile, FormatError> {
    let file: PointerFile =
        serde_yaml::from_slice(bytes).map_err(|error| FormatError::new(error.to_string()))?;
    if file.schema != POINTER_SCHEMA {
        return Err(wrong_schema(&file.schema, POINTER_SCHEMA));
    }
    Ok(file)
}

pub(crate) fn encode_marker(
    family_id: &FamilyId,
    allocation_id: &AllocationId,
) -> Result<Vec<u8>, FormatError> {
    encode(&MarkerFile {
        schema: MARKER_SCHEMA.to_owned(),
        family_id: family_id.as_str().to_owned(),
        allocation_id: allocation_id.as_str().to_owned(),
    })
}

pub(crate) fn decode_marker(bytes: &[u8]) -> Result<MarkerFile, FormatError> {
    let file: MarkerFile =
        serde_yaml::from_slice(bytes).map_err(|error| FormatError::new(error.to_string()))?;
    if file.schema != MARKER_SCHEMA {
        return Err(wrong_schema(&file.schema, MARKER_SCHEMA));
    }
    Ok(file)
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, FormatError> {
    serde_yaml::to_string(value)
        .map(String::into_bytes)
        .map_err(|error| FormatError::new(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_family_model::{FamilyChange, fields, validate_transition};

    fn view() -> FamilyView {
        let mut view = FamilyView::founded(
            FamilyId::new("fam_format").unwrap(),
            AllocationId::new("alloc_root").unwrap(),
        );
        let validated = validate_transition(
            &view,
            &FamilyChange::Allocate {
                name: MemberName::parse("A").unwrap(),
                row: MemberRow {
                    path: "../ws-A".to_owned(),
                    kind: MemberKind::Bare,
                    state: MemberState::Creating,
                    allocation_id: AllocationId::new("alloc_a").unwrap(),
                    source_path: ".".to_owned(),
                    mode: CloneMode::Bare,
                    last_error: Some("interrupted".to_owned()),
                },
            },
        )
        .unwrap();
        view = validated.next;
        view
    }

    #[test]
    fn the_index_round_trips_through_the_frozen_keys() {
        let view = view();
        let encoded = encode_index(&view).unwrap();
        let text = String::from_utf8(encoded.clone()).unwrap();
        for key in [
            fields::SCHEMA,
            fields::FAMILY_ID,
            fields::ROOT,
            fields::MEMBERS,
            fields::PATH,
            fields::KIND,
            fields::STATE,
            fields::ALLOCATION_ID,
            fields::SOURCE_PATH,
            fields::MODE,
            fields::LAST_ERROR,
        ] {
            assert!(
                text.contains(&format!("{key}:")),
                "{key} missing from {text}"
            );
        }
        assert!(text.contains(INDEX_SCHEMA), "{text}");
        assert_eq!(decode_index(&encoded).unwrap(), view);
    }

    #[test]
    fn an_absent_last_error_is_omitted_and_decodes_as_none() {
        let mut view = view();
        view.members
            .get_mut(&MemberName::parse("A").unwrap())
            .unwrap()
            .last_error = None;
        let encoded = encode_index(&view).unwrap();
        assert!(
            !String::from_utf8(encoded.clone())
                .unwrap()
                .contains("last_error"),
            "an absent diagnostic is not written"
        );
        assert_eq!(decode_index(&encoded).unwrap(), view);
    }

    #[test]
    fn an_unknown_field_and_a_wrong_version_refuse_instead_of_being_accepted() {
        let unknown =
            b"schema: gwz.local-family/v1\nfamily_id: f\nroot:\n  allocation_id: a\nsurprise: 1\n";
        let error = decode_index(unknown).unwrap_err();
        assert!(error.detail().contains("surprise"), "{error:?}");

        let future = b"schema: gwz.local-family/v2\nfamily_id: f\nroot:\n  allocation_id: a\n";
        let error = decode_index(future).unwrap_err();
        assert!(error.detail().contains("gwz.local-family/v2"), "{error:?}");
        assert!(error.detail().contains(INDEX_SCHEMA), "{error:?}");

        let unknown_row = b"schema: gwz.local-family/v1\nfamily_id: f\nroot:\n  allocation_id: a\nmembers:\n  A:\n    path: ../ws-A\n    kind: checkout\n    state: elsewhere\n    allocation_id: x\n    source_path: .\n    mode: verbatim\n";
        let error = decode_index(unknown_row).unwrap_err();
        assert!(error.detail().contains("state: elsewhere"), "{error:?}");
    }

    #[test]
    fn the_pointer_and_marker_round_trip_and_refuse_another_format() {
        let family = FamilyId::new("fam_format").unwrap();
        let encoded = encode_pointer(&family, "/roots/one").unwrap();
        let decoded = decode_pointer(&encoded).unwrap();
        assert_eq!(decoded.family_id, "fam_format");
        assert_eq!(decoded.root_path, "/roots/one");
        assert!(
            decode_pointer(b"schema: gwz.family-root/v9\nfamily_id: f\nroot_path: /r\n").is_err()
        );
        assert!(
            decode_pointer(b"schema: gwz.family-root/v1\nfamily_id: f\nroot_path: /r\nextra: 1\n")
                .is_err(),
            "an unknown pointer field refuses"
        );

        let allocation = AllocationId::new("alloc_a").unwrap();
        let encoded = encode_marker(&family, &allocation).unwrap();
        let decoded = decode_marker(&encoded).unwrap();
        assert_eq!(decoded.allocation_id, "alloc_a");
        assert_eq!(decoded.family_id, "fam_format");
        assert!(String::from_utf8(encoded).unwrap().contains(MARKER_SCHEMA));
        assert!(decode_marker(b"schema: other\nfamily_id: f\nallocation_id: a\n").is_err());
    }

    #[test]
    fn a_malformed_document_reports_the_decoder_detail() {
        let error = decode_index(b"schema: [unterminated\n").unwrap_err();
        assert!(!error.detail().is_empty());
    }
}
