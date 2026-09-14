use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_yaml::Value;
use sha2::{Digest, Sha256};

use crate::artifact::{ArtifactSourceKind, LockArtifact, ManifestArtifact, ResolvedMemberArtifact};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::MergeParticipantRecord;
use crate::workspace_ops::merge::model::v1::AcceptedLockMemberV1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MemberIdentity {
    path: String,
    source_id: String,
    source_kind: ArtifactSourceKind,
}

impl MemberIdentity {
    pub(super) fn to_lock_member(&self) -> ResolvedMemberArtifact {
        ResolvedMemberArtifact {
            path: self.path.clone(),
            source_id: Some(self.source_id.clone()),
            source_kind: self.source_kind,
            commit: None,
            branch: None,
            detached: None,
            upstream: None,
            dirty: None,
            materialized: None,
        }
    }
}

pub(super) fn selected_identity(
    member_id: &str,
    participant: &MergeParticipantRecord,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    baseline_manifest: &ManifestArtifact,
    baseline_lock: &LockArtifact,
    merge_id: &str,
) -> ModelResult<MemberIdentity> {
    let baseline = manifest_identity(baseline_manifest, member_id)
        .or_else(|| lock_identity(baseline_lock, member_id))
        .ok_or_else(|| input_error(merge_id, "selected member has no frozen identity"))?;
    let identities = [
        Some(baseline.clone()),
        manifest_identity(manifest, member_id),
        lock_identity(lock, member_id),
        manifest_identity(baseline_manifest, member_id),
        lock_identity(baseline_lock, member_id),
    ];
    if identities
        .iter()
        .flatten()
        .any(|identity| identity != &baseline || identity.path != participant.path)
    {
        // v0 raised this from `construct_complete_lock`
        // (`git show 57502e4:src/workspace_ops/merge/acceptance/workspace.rs`,
        // lines 70-77) as `ManifestInvalid` carrying the member's own id and
        // path. Both halves matter and neither is decoration: the code is an
        // artifact-validity code (wire 6), so `GwzM5-8I2ProtocolContract.md`
        // §1's "Codes 46-65 have absent member/detail/target fields" does not
        // reach it, and the operator needs the member named to know which
        // `path:` in the merged root's manifest to put back.
        return Err(ModelError::new(
            ErrorCode::ManifestInvalid,
            format!("merge record '{merge_id}' selected member identity changed before acceptance"),
        )
        .with_member(member_id, &participant.path));
    }
    Ok(baseline)
}

/// Parse the merged root's own manifest, keeping the artifact layer's typed
/// code instead of laundering it into `AcceptanceInputDrift`.
///
/// `AcceptanceInputDrift` is wire code 50 and
/// `dev-docs/GwzM5-8I2ProtocolContract.md` §1 (as amended, accepted GO/GO)
/// says "Codes 46-65 have absent member/detail/target fields" and restricts
/// their message bodies to the reasons registered in
/// `GwzM5-8I2CompatibilityPredicates.json`. An unsupported schema or an
/// escaping member path in the merged root's manifest is neither: it is an
/// artifact-validity defect whose typed codes (`ManifestInvalid` 6,
/// `SchemaUnsupported` 7, `PathEscape` 10) sit outside that range, are free to
/// carry `@root` / `.`, and tell the operator what to fix. This is what v0
/// raised through `root::candidate_metadata`.
pub(super) fn parse_root_manifest(yaml: &str) -> ModelResult<ManifestArtifact> {
    ManifestArtifact::from_yaml(yaml).map_err(root_metadata)
}

/// The lock half of [`parse_root_manifest`], with the same reasoning.
pub(super) fn parse_root_lock(yaml: &str) -> ModelResult<LockArtifact> {
    LockArtifact::from_yaml(yaml).map_err(root_metadata)
}

pub(super) fn root_metadata(error: ModelError) -> ModelError {
    if error.member_id.is_none() {
        error.with_member("@root", ".")
    } else {
        error
    }
}

fn manifest_identity(manifest: &ManifestArtifact, member_id: &str) -> Option<MemberIdentity> {
    manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .map(|member| MemberIdentity {
            path: member.path.clone(),
            source_id: member.source_id.clone(),
            source_kind: member.source_kind,
        })
}

fn lock_identity(lock: &LockArtifact, member_id: &str) -> Option<MemberIdentity> {
    let member = lock.members.get(member_id)?;
    Some(MemberIdentity {
        path: member.path.clone(),
        source_id: member.source_id.clone()?,
        source_kind: member.source_kind,
    })
}

#[derive(Deserialize)]
struct AcceptedLockRows {
    members: BTreeMap<String, AcceptedLockMemberV1>,
}

pub(super) fn parse_lock_rows(
    merge_id: &str,
    yaml: &str,
) -> ModelResult<BTreeMap<String, AcceptedLockMemberV1>> {
    serde_yaml::from_str::<AcceptedLockRows>(yaml)
        .map(|lock| lock.members)
        .map_err(|_| input_error(merge_id, "accepted lock rows are invalid"))
}

pub(super) fn parse_manifest(merge_id: &str, yaml: &str) -> ModelResult<ManifestArtifact> {
    ManifestArtifact::from_yaml(yaml)
        .map_err(|_| input_error(merge_id, "accepted manifest bytes are invalid"))
}

pub(super) fn parse_lock(merge_id: &str, yaml: &str) -> ModelResult<LockArtifact> {
    LockArtifact::from_yaml(yaml)
        .map_err(|_| input_error(merge_id, "accepted lock bytes are invalid"))
}

/// The lock row fields this gwz knows, in `ResolvedMemberArtifact` declaration
/// order: the order `LockArtifact::to_yaml`, the `gwz commit` writer, emits.
const LOCK_ROW_FIELDS: [&str; 9] = [
    "path",
    "source_id",
    "source_kind",
    "commit",
    "branch",
    "detached",
    "upstream",
    "dirty",
    "materialized",
];

/// Only selected rows are rewritten. Unselected rows and the top level pass
/// through as parsed, in the metadata lock's order, so a merge that selects no
/// member re-emits a gwz-written lock byte for byte.
pub(super) fn render_complete_lock(
    merge_id: &str,
    metadata_yaml: &str,
    baseline_yaml: &str,
    complete: &LockArtifact,
    selected: &BTreeSet<String>,
) -> ModelResult<String> {
    let mut raw: Value = serde_yaml::from_str(metadata_yaml)
        .map_err(|_| input_error(merge_id, "accepted lock YAML is invalid"))?;
    let baseline: Value = serde_yaml::from_str(baseline_yaml)
        .map_err(|_| input_error(merge_id, "baseline lock YAML is invalid"))?;
    for member_id in selected {
        let typed = complete.members.get(member_id).ok_or_else(|| {
            input_error(merge_id, "selected member is absent from the complete lock")
        })?;
        let members = mapping_field_mut(&mut raw, "members", merge_id)?;
        let key = Value::String(member_id.clone());
        if !members.contains_key(&key) {
            let baseline_row = baseline
                .get("members")
                .and_then(|members| members.get(member_id))
                .cloned()
                .unwrap_or(serde_yaml::to_value(typed).map_err(|_| {
                    input_error(merge_id, "selected lock row cannot be serialized")
                })?);
            insert_in_member_order(members, key.clone(), baseline_row);
        }
        let row = members
            .get_mut(&key)
            .and_then(Value::as_mapping_mut)
            .ok_or_else(|| input_error(merge_id, "selected lock row is not a mapping"))?;
        let replacement = serde_yaml::to_value(typed)
            .map_err(|_| input_error(merge_id, "selected lock row cannot be serialized"))?;
        let Value::Mapping(mut replacement) = replacement else {
            return Err(input_error(merge_id, "selected lock row is not a mapping"));
        };
        // Rebuild the row instead of editing it in place: `Mapping::remove` is
        // `swap_remove`, so removing and re-inserting each field rotated the
        // row on every merge. The typed row supplies the known fields in the
        // `gwz commit` order. Fields this gwz does not know follow them, in the
        // order the row held them: that layout is a fixed point of this
        // rebuild, and it is where a newer gwz that appends a field after
        // `materialized` already writes it.
        for (field, value) in std::mem::take(row) {
            if !field
                .as_str()
                .is_some_and(|name| LOCK_ROW_FIELDS.contains(&name))
            {
                replacement.insert(field, value);
            }
        }
        *row = replacement;
    }
    let rendered = serde_yaml::to_string(&raw)
        .map_err(|_| input_error(merge_id, "complete lock cannot be serialized"))?;
    if parse_lock(merge_id, &rendered)? != *complete {
        return Err(input_error(
            merge_id,
            "complete lock YAML differs from its typed model",
        ));
    }
    Ok(rendered)
}

/// Insert a row the metadata lock lacks where `gwz commit` would put it.
/// `LockArtifact::members` is a `BTreeMap`, so rows are written in member-id
/// order: the row goes before the first row whose id sorts after it, and the
/// rows already present keep their order.
fn insert_in_member_order(members: &mut serde_yaml::Mapping, key: Value, row: Value) {
    let mut rows = std::mem::take(members).into_iter().collect::<Vec<_>>();
    let at = rows
        .iter()
        .position(|(existing, _)| existing.as_str() > key.as_str())
        .unwrap_or(rows.len());
    rows.insert(at, (key, row));
    *members = rows.into_iter().collect();
}

fn mapping_field_mut<'a>(
    value: &'a mut Value,
    field: &str,
    merge_id: &str,
) -> ModelResult<&'a mut serde_yaml::Mapping> {
    value
        .get_mut(field)
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| input_error(merge_id, "accepted lock members are not a mapping"))
}

pub(super) fn require_workspace(
    workspace_id: &str,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    merge_id: &str,
) -> ModelResult<()> {
    if manifest.workspace.id == workspace_id && lock.workspace_id == workspace_id {
        Ok(())
    } else {
        Err(input_error(
            merge_id,
            "accepted metadata workspace identity changed",
        ))
    }
}

pub(super) fn required<'a>(
    value: Option<&'a str>,
    merge_id: &str,
    detail: &str,
) -> ModelResult<&'a str> {
    value.ok_or_else(|| input_error(merge_id, detail))
}

pub(super) fn require_digest(value: &str, expected: &str, merge_id: &str) -> ModelResult<()> {
    if digest(value) == expected {
        Ok(())
    } else {
        Err(input_error(
            merge_id,
            "operation baseline exact bytes do not match their digest",
        ))
    }
}

pub(super) fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(super) fn input_error(merge_id: &str, detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::AcceptanceInputDrift,
        format!("merge record '{merge_id}' acceptance input is incomplete: {detail}"),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_yaml::Value;

    use super::{LOCK_ROW_FIELDS, parse_lock_rows, render_complete_lock};
    use crate::artifact::{
        ArtifactSourceKind, LOCK_SCHEMA, LockArtifact, ResolvedMemberArtifact, WORKSPACE_SCHEMA,
    };

    const MERGE_ID: &str = "merge_lock_order";

    fn row(path: &str, commit: char) -> ResolvedMemberArtifact {
        ResolvedMemberArtifact {
            path: path.to_owned(),
            source_id: Some(format!("src_{path}")),
            source_kind: ArtifactSourceKind::Git,
            commit: Some(commit.to_string().repeat(40)),
            branch: Some("main".to_owned()),
            detached: Some(false),
            upstream: Some("origin/main".to_owned()),
            dirty: Some(false),
            materialized: Some(true),
        }
    }

    fn lock(rows: Vec<(&str, ResolvedMemberArtifact)>) -> LockArtifact {
        LockArtifact {
            schema: LOCK_SCHEMA.to_owned(),
            workspace_id: "ws_lock_order".to_owned(),
            manifest_schema: WORKSPACE_SCHEMA.to_owned(),
            members: rows
                .into_iter()
                .map(|(member_id, row)| (member_id.to_owned(), row))
                .collect(),
        }
    }

    fn selected(member_ids: &[&str]) -> BTreeSet<String> {
        member_ids
            .iter()
            .map(|member_id| (*member_id).to_owned())
            .collect()
    }

    /// `LOCK_ROW_FIELDS` decides which row fields are unknown, so it must name
    /// every field the commit writer emits, in that writer's order.
    #[test]
    fn known_row_fields_are_the_commit_writer_fields() {
        let serialized = serde_yaml::to_value(row("core", 'c')).unwrap();
        let fields = serialized
            .as_mapping()
            .unwrap()
            .keys()
            .map(|field| field.as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(fields, LOCK_ROW_FIELDS);
    }

    /// Consecutive merges of an unchanged lock, each reading the bytes the
    /// previous one wrote. A rewrite that reorders fields can come back to
    /// its starting order (the `swap_remove` rewrite cycled every seven
    /// renders), so every one of eight renders is compared.
    #[test]
    fn repeated_renders_of_one_lock_are_byte_identical() {
        let mut evidence = row("evidence", 'e');
        evidence.upstream = None;
        let complete = lock(vec![
            ("mem_cli", row("cli", 'b')),
            ("mem_core", row("core", 'c')),
            ("mem_evidence", evidence),
        ]);
        let targets = selected(&["mem_cli", "mem_core", "mem_evidence"]);
        let baseline = complete.to_yaml().unwrap();
        let mut current = baseline.clone();
        let mut renders = Vec::new();
        for _ in 0..8 {
            current =
                render_complete_lock(MERGE_ID, &current, &baseline, &complete, &targets).unwrap();
            renders.push(current.clone());
        }
        for (index, render) in renders.iter().enumerate() {
            assert_eq!(
                render,
                &renders[0],
                "render {} differs from render 1",
                index + 1
            );
        }
    }

    /// Without unknown fields a merge writes the bytes `gwz commit` writes for
    /// the same typed lock: rows in member-id order, fields in declaration
    /// order. `mem_a_added` is in neither lock and sorts before every metadata
    /// row; `mem_c_restored` comes back from the baseline lock between two.
    #[test]
    fn rendered_lock_is_the_commit_writer_lock_when_rows_are_added() {
        let metadata = lock(vec![
            ("mem_b_kept", row("kept", '1')),
            ("mem_d_changed", row("changed", '2')),
        ]);
        let baseline = lock(vec![
            ("mem_b_kept", row("kept", '1')),
            ("mem_c_restored", row("restored", '3')),
            ("mem_d_changed", row("changed", '2')),
        ]);
        let complete = lock(vec![
            ("mem_a_added", row("added", '4')),
            ("mem_b_kept", row("kept", '1')),
            ("mem_c_restored", row("restored", '5')),
            ("mem_d_changed", row("changed", '6')),
        ]);

        let rendered = render_complete_lock(
            MERGE_ID,
            &metadata.to_yaml().unwrap(),
            &baseline.to_yaml().unwrap(),
            &complete,
            &selected(&["mem_a_added", "mem_c_restored", "mem_d_changed"]),
        )
        .unwrap();

        assert_eq!(rendered, complete.to_yaml().unwrap());
    }

    /// An unknown row field stays in its row, after the known fields. The
    /// known fields come out in declaration order even from a row that holds
    /// them in the rotated order earlier merges wrote.
    #[test]
    fn unknown_row_fields_follow_the_known_fields() {
        let complete = lock(vec![("mem_core", row("core", 'd'))]);
        let metadata = format!(
            "\
schema: gwz.lock/v0
workspace_id: ws_lock_order
manifest_schema: gwz.workspace/v0
members:
  mem_core:
    dirty: false
    path: core
    future_field: kept
    source_id: src_core
    source_kind: git
    commit: {commit}
    branch: main
    detached: false
    upstream: origin/main
    materialized: true
",
            commit = "c".repeat(40)
        );
        let targets = selected(&["mem_core"]);

        let rendered =
            render_complete_lock(MERGE_ID, &metadata, &metadata, &complete, &targets).unwrap();

        let expected = complete.to_yaml().unwrap().replace(
            "    materialized: true\n",
            "    materialized: true\n    future_field: kept\n",
        );
        assert_eq!(rendered, expected);
        assert_eq!(
            parse_lock_rows(MERGE_ID, &rendered).unwrap()["mem_core"]
                .extensions
                .get("future_field"),
            Some(&Value::String("kept".to_owned()))
        );
        assert_eq!(
            render_complete_lock(MERGE_ID, &rendered, &metadata, &complete, &targets).unwrap(),
            rendered
        );
    }
}
