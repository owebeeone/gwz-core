//! The frozen metadata files: index, clone pointer, allocation marker.
//!
//! Every key is a `gwz_family_model::fields` constant and every enum value
//! is that model's frozen spelling, so a rename here is a format change and
//! not a refactor (design §3). Decoding is strict in both directions that
//! matter: an unknown field and a `schema:` that is not a format version
//! this store reads refuse as malformed rather than being accepted or
//! upgraded.
//!
//! **Index format 2 (GwzLaneCleanFixes R20).** The index gained exactly one
//! key, the optional per-row `owner` token. This store *reads* format 1 and
//! format 2 ([`gwz_family_model::INDEX_SCHEMAS_READ`]) and *writes* format 2
//! always, so the first write of any kind — a create, a dispose, a `--keep`,
//! a family merge — carries a v1 index forward to v2. Going the other way is
//! a refusal, not a downgrade: an older gwz reading a v2 index refuses the
//! whole file, and the refusal names the minimum gwz version that reads what
//! it found ([`wrong_schema`]), because `deny_unknown_fields` means the
//! `owner` key could never have been added silently. The schema is therefore
//! checked *before* the strict field decode, so the answer to a newer index
//! is the version it needs and not an unknown-field complaint about one of
//! its keys.

use std::collections::BTreeMap;

use gwz_family_model::{
    AllocationId, CloneMode, FamilyId, FamilyView, INDEX_SCHEMA, INDEX_SCHEMAS_READ, MemberKind,
    MemberName, MemberRow, MemberState, OwnerToken, POINTER_SCHEMA, index_schema_min_gwz_version,
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

/// The refusal for a `schema:` this decoder does not read. `accepted` is
/// what it *does* read, newest last. R20: when the value found is a schema
/// gwz knows but this decoder is too old for, the refusal names the minimum
/// gwz version that reads it, so an operator is told what to install rather
/// than left with a version string to interpret.
fn wrong_schema_any(found: &str, accepted: &[&str]) -> FormatError {
    let expected = accepted
        .iter()
        .map(|schema| format!("`{schema}`"))
        .collect::<Vec<_>>()
        .join(" or ");
    let mut detail = format!(
        "`schema: {found}` is not {expected}; this file is not in a format this store reads"
    );
    if let Some(minimum) = index_schema_min_gwz_version(found)
        && !accepted.contains(&found)
    {
        detail.push_str(&format!(
            "; `{found}` is read by gwz {minimum} and later, so upgrade gwz to at least {minimum}"
        ));
    }
    FormatError::new(detail)
}

fn wrong_schema(found: &str, expected: &str) -> FormatError {
    wrong_schema_any(found, &[expected])
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
    /// Format 2 only (R20). Absent on every format-1 row and on any row
    /// reserved without `--owner`; never rewritten once the row exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
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
                        owner: row.owner.as_ref().map(|owner| owner.as_str().to_owned()),
                    },
                )
            })
            .collect(),
    };
    encode(&file)
}

/// Just enough of any index to read its `schema:`. Unknown keys are ignored
/// here on purpose: the schema decides whether the strict decode below is
/// even the right one to run, so it must be readable in a file whose other
/// keys this build has never heard of.
#[derive(Debug, Deserialize)]
struct IndexSchemaProbe {
    schema: String,
}

pub(crate) fn decode_index(bytes: &[u8]) -> Result<FamilyView, FormatError> {
    decode_index_accepting(bytes, INDEX_SCHEMAS_READ)
}

/// Decode an index, accepting only the schemas in `accepted`. Production
/// passes [`INDEX_SCHEMAS_READ`]; a narrower list is how a test stands in
/// for an older gwz reading a newer index (R22).
pub(crate) fn decode_index_accepting(
    bytes: &[u8],
    accepted: &[&str],
) -> Result<FamilyView, FormatError> {
    let probe: IndexSchemaProbe =
        serde_yaml::from_slice(bytes).map_err(|error| FormatError::new(error.to_string()))?;
    if !accepted.contains(&probe.schema.as_str()) {
        return Err(wrong_schema_any(&probe.schema, accepted));
    }
    let file: IndexFile =
        serde_yaml::from_slice(bytes).map_err(|error| FormatError::new(error.to_string()))?;
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
        owner: row
            .owner
            .map(OwnerToken::parse)
            .transpose()
            .map_err(|error| FormatError::new(format!("row `{name}`: `owner`: {error}")))?,
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
                    owner: Some(OwnerToken::parse("claude-code:lane_1").unwrap()),
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
            fields::OWNER,
        ] {
            assert!(
                text.contains(&format!("{key}:")),
                "{key} missing from {text}"
            );
        }
        assert!(text.contains(INDEX_SCHEMA), "{text}");
        assert_eq!(decode_index(&encoded).unwrap(), view);
    }

    /// R20: a format-1 index still decodes, every row reporting no owner,
    /// and the very next encode of that same view writes format 2.
    #[test]
    fn a_format_1_index_reads_unchanged_and_re_encodes_as_format_2() {
        let v1 = b"schema: gwz.local-family/v1\n\
                   family_id: fam_format\n\
                   root:\n  allocation_id: alloc_root\n\
                   members:\n\
                   \x20 A:\n\
                   \x20   path: ../ws-A\n\
                   \x20   kind: checkout\n\
                   \x20   state: ready\n\
                   \x20   allocation_id: alloc_a\n\
                   \x20   source_path: .\n\
                   \x20   mode: verbatim\n";
        let view = decode_index(v1).expect("a format-1 index still reads");
        let (_, row) = view.member("A").expect("the row decodes");
        assert_eq!(row.owner, None, "a format-1 row records no owner");
        let text = String::from_utf8(encode_index(&view).unwrap()).unwrap();
        assert!(
            text.contains(INDEX_SCHEMA),
            "the first write is format 2: {text}"
        );
        assert!(
            !text.contains(&format!("{}:", fields::OWNER)),
            "a row with no owner writes no `owner` key: {text}"
        );
    }

    /// R22, second test: a v1-only decoder handed a v2 index refuses the
    /// whole file and the refusal names the minimum gwz version that reads
    /// it -- not an unknown-field complaint about `owner`.
    #[test]
    fn a_v1_only_decoder_refuses_a_v2_index_naming_the_minimum_gwz_version() {
        let encoded = encode_index(&view()).unwrap();
        let text = String::from_utf8(encoded.clone()).unwrap();
        assert!(
            text.contains("owner:"),
            "the fixture exercises `owner`: {text}"
        );
        let error = decode_index_accepting(&encoded, &[gwz_family_model::INDEX_SCHEMA_V1])
            .expect_err("a v1-only decoder refuses a v2 index");
        let detail = error.detail();
        assert!(detail.contains(INDEX_SCHEMA), "{detail}");
        assert!(
            detail.contains(gwz_family_model::INDEX_MIN_GWZ_VERSION),
            "the refusal must name the minimum gwz version: {detail}"
        );
        assert!(
            !detail.contains("unknown field"),
            "the schema decides before the strict field decode: {detail}"
        );
        // The gate is the schema and nothing else: the same decoder reads
        // the file it is for.
        let v1 = text.replace(INDEX_SCHEMA, gwz_family_model::INDEX_SCHEMA_V1);
        assert!(
            decode_index_accepting(v1.as_bytes(), &[gwz_family_model::INDEX_SCHEMA_V1]).is_ok(),
            "a v1-only decoder still reads a v1 index"
        );
    }

    /// R20: an owner token that is not the model's shape refuses the index
    /// rather than being carried as an opaque string.
    #[test]
    fn a_malformed_owner_token_refuses_the_index() {
        let encoded = encode_index(&view()).unwrap();
        let text = String::from_utf8(encoded).unwrap();
        let bad = text.replace("claude-code:lane_1", "\"lane one\"");
        let error = decode_index(bad.as_bytes()).expect_err("an invalid owner refuses");
        assert!(error.detail().contains("owner"), "{}", error.detail());
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

        let future = b"schema: gwz.local-family/v9\nfamily_id: f\nroot:\n  allocation_id: a\n";
        let error = decode_index(future).unwrap_err();
        assert!(error.detail().contains("gwz.local-family/v9"), "{error:?}");
        assert!(error.detail().contains(INDEX_SCHEMA), "{error:?}");
        assert!(
            !error.detail().contains("upgrade gwz"),
            "a schema no gwz claims to read names no version: {error:?}"
        );

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
