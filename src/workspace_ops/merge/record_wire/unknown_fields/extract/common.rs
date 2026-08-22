use std::collections::BTreeSet;

use serde_yaml::Mapping;

use super::super::identity;
use super::super::support::{
    Path, child, collect_unknown, field, identity_child, map_child, mapping, require_unique,
    sequence,
};
use super::super::{SemanticIdentity, UnknownFieldManifest, UnknownFieldManifestError};

/// The preservation evidence row's known-key set forks by record version.
///
/// `GwzM5-8DurableCursorAmendment.md` §2.3: manifest membership is governed by
/// the extractor's known-key set, not the typed serde parse. The v1 set adopts
/// `noop_commit`/`reset_commit` — without which the first marker write fails
/// the overlay's unauthorized-unknown-field check. The v0 set must NOT adopt
/// them, or the v0 collision rule loses its trigger.
#[derive(Clone, Copy)]
pub(super) enum EvidenceKeys {
    V0,
    V1,
}

/// The four inherited v0 field names, unchanged.
const EVIDENCE_KEYS_V0: &[&str] = &["backup_ref", "backup_commit", "stash_id", "stash_object_id"];

/// The v0 names plus this amendment's two markers.
const EVIDENCE_KEYS_V1: &[&str] = &[
    "backup_ref",
    "backup_commit",
    "stash_id",
    "stash_object_id",
    "noop_commit",
    "reset_commit",
];

impl EvidenceKeys {
    fn names(self) -> &'static [&'static str] {
        match self {
            Self::V0 => EVIDENCE_KEYS_V0,
            Self::V1 => EVIDENCE_KEYS_V1,
        }
    }
}

pub(super) fn extract(
    root: &Mapping,
    path: &Path,
    evidence_keys: EvidenceKeys,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    extract_baseline(root, path, manifest)?;
    extract_participants(root, path, evidence_keys, manifest)?;
    extract_publication(root, path, evidence_keys, manifest)?;
    extract_operation_drift(root, path, manifest)
}

fn extract_baseline(
    root: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(root, "baseline") else {
        return Ok(());
    };
    collect_unknown(
        mapping(value, "baseline")?,
        &[
            "lock_sha256",
            "manifest_sha256",
            "lock_yaml",
            "manifest_yaml",
            "lock_commit_sha256",
            "manifest_commit_sha256",
            "root_head",
            "root_branch",
        ],
        &child(path, "baseline"),
        manifest,
    )
}

fn extract_participants(
    root: &Mapping,
    path: &Path,
    evidence_keys: EvidenceKeys,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(root, "participants") else {
        return Ok(());
    };
    let participants_path = child(path, "participants");
    for (member_id, value) in mapping(value, "participants")? {
        let Some(member_id) = member_id.as_str() else {
            return Err(super::super::error("participant key is not a string"));
        };
        extract_participant(
            member_id,
            mapping(value, "participant")?,
            &map_child(&participants_path, member_id),
            evidence_keys,
            manifest,
        )?;
    }
    Ok(())
}

fn extract_participant(
    member_id: &str,
    participant: &Mapping,
    path: &Path,
    evidence_keys: EvidenceKeys,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    collect_unknown(
        participant,
        &[
            "path",
            "target_kind",
            "target_branch",
            "before_commit",
            "source_commit",
            "commit_message",
            "state",
            "resulting_commit",
            "expected_merge_head",
            "conflict_paths",
            "conflict_snapshot",
            "error",
            "pending_action",
            "preservation",
            "drift",
        ],
        path,
        manifest,
    )?;
    extract_conflicts(participant, path, manifest)?;
    extract_error(participant, path, manifest)?;
    extract_pending_action(participant, path, manifest)?;
    extract_preservation(
        participant,
        path,
        &format!("participant:{member_id}"),
        evidence_keys,
        manifest,
    )?;
    extract_participant_drift(participant, path, manifest)
}

fn extract_conflicts(
    participant: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(participant, "conflict_snapshot") else {
        return Ok(());
    };
    let sequence_path = child(path, "conflict_snapshot");
    let mut seen = BTreeSet::new();
    for value in sequence(value, "conflict_snapshot")? {
        let row = mapping(value, "conflict evidence")?;
        let identity = identity::conflict(row, 0)?;
        require_unique(&mut seen, &identity, "conflict evidence")?;
        collect_unknown(
            row,
            &["path", "sha256"],
            &identity_child(&sequence_path, identity),
            manifest,
        )?;
    }
    Ok(())
}

fn extract_error(
    participant: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(participant, "error").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let row = mapping(value, "participant error")?;
    collect_unknown(
        row,
        &["code", "message", "detail"],
        &identity_child(
            &child(
                &identity_child(path, identity::participant_error_scope(participant)?),
                "error",
            ),
            identity::participant_error(row)?,
        ),
        manifest,
    )
}

fn extract_pending_action(
    participant: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(participant, "pending_action").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let row = mapping(value, "pending action")?;
    let action_path = identity_child(
        &child(path, "pending_action"),
        identity::pending_action(row)?,
    );
    collect_unknown(
        row,
        &[
            "kind",
            "target_branch",
            "before_commit",
            "source_commit",
            "commit_message",
            "expected_result",
            "commit_spec",
        ],
        &action_path,
        manifest,
    )?;
    let Some(spec) = field(row, "commit_spec").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let spec = mapping(spec, "commit spec")?;
    let spec_path = child(&action_path, "commit_spec");
    collect_unknown(
        spec,
        &["tree_oid", "author", "committer"],
        &spec_path,
        manifest,
    )?;
    for role in ["author", "committer"] {
        let Some(signature) = field(spec, role) else {
            continue;
        };
        collect_unknown(
            mapping(signature, role)?,
            &["name", "email", "time_seconds", "timezone_offset_minutes"],
            &child(&spec_path, role),
            manifest,
        )?;
    }
    Ok(())
}

fn extract_preservation(
    parent: &Mapping,
    path: &Path,
    owner: &str,
    evidence_keys: EvidenceKeys,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(parent, "preservation") else {
        return Ok(());
    };
    extract_preservation_rows(
        value,
        &child(path, "preservation"),
        owner,
        evidence_keys,
        manifest,
    )
}

fn extract_preservation_rows(
    value: &serde_yaml::Value,
    sequence_path: &Path,
    owner: &str,
    evidence_keys: EvidenceKeys,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let rows = sequence(value, "preservation")?;
    if rows.len() > 1 {
        return Err(super::super::error(format!(
            "preservation evidence owner '{owner}' is duplicated"
        )));
    }
    for value in rows {
        collect_unknown(
            mapping(value, "preservation evidence")?,
            evidence_keys.names(),
            &identity_child(sequence_path, identity::preservation_owner(owner)),
            manifest,
        )?;
    }
    Ok(())
}

fn extract_participant_drift(
    participant: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(participant, "drift") else {
        return Ok(());
    };
    let sequence_path = child(path, "drift");
    let mut prior: Vec<SemanticIdentity> = Vec::new();
    for value in sequence(value, "participant drift")? {
        let row = mapping(value, "participant drift")?;
        let mut identity = identity::participant_drift(row, 0)?;
        identity.occurrence = identity::occurrence_for(&prior, &identity);
        prior.push(identity.clone());
        collect_unknown(
            row,
            &[
                "kind",
                "message",
                "expected_branch",
                "live_branch",
                "expected_head",
                "live_head",
                "expected_merge_head",
                "live_merge_head",
            ],
            &identity_child(&sequence_path, identity),
            manifest,
        )?;
    }
    Ok(())
}

fn extract_publication(
    root: &Mapping,
    path: &Path,
    evidence_keys: EvidenceKeys,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(root, "publication").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let publication = mapping(value, "publication")?;
    let publication_path = child(path, "publication");
    collect_unknown(
        publication,
        &[
            "step",
            "candidate_lock_sha256",
            "candidate_marker_path",
            "root_merge_commit",
            "composition_commit",
            "composition_tree",
            "candidate_hashes",
            "candidate",
            "evidence_rolled_back",
            "root_preservation",
            "preservation_prefix",
        ],
        &publication_path,
        manifest,
    )?;
    extract_candidate_hashes(publication, &publication_path, manifest)?;
    extract_candidate(publication, &publication_path, manifest)?;
    if let Some(value) = field(publication, "root_preservation") {
        extract_preservation_rows(
            value,
            &child(&publication_path, "root_preservation"),
            "publication-root",
            evidence_keys,
            manifest,
        )?;
    }
    Ok(())
}

fn extract_candidate_hashes(
    publication: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(publication, "candidate_hashes") else {
        return Ok(());
    };
    let sequence_path = child(path, "candidate_hashes");
    let mut seen = BTreeSet::new();
    for value in sequence(value, "candidate hashes")? {
        let row = mapping(value, "candidate hash")?;
        let identity = identity::candidate_hash(row, 0)?;
        require_unique(&mut seen, &identity, "candidate hash")?;
        collect_unknown(
            row,
            &["path", "sha256"],
            &identity_child(&sequence_path, identity),
            manifest,
        )?;
    }
    Ok(())
}

fn extract_candidate(
    publication: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(publication, "candidate").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    collect_unknown(
        mapping(value, "publication candidate")?,
        &[
            "marker_id",
            "root_branch",
            "actor_id",
            "baseline_lock_yaml",
            "lock_yaml",
            "marker_yaml",
            "baseline_boundary_text",
            "boundary_text",
            "baseline_boundary_sha256",
            "marker_sha256",
            "boundary_sha256",
        ],
        &child(path, "candidate"),
        manifest,
    )
}

fn extract_operation_drift(
    root: &Mapping,
    path: &Path,
    manifest: &mut UnknownFieldManifest,
) -> Result<(), UnknownFieldManifestError> {
    let Some(value) = field(root, "operation_drift") else {
        return Ok(());
    };
    let sequence_path = child(path, "operation_drift");
    let mut seen = BTreeSet::new();
    for value in sequence(value, "operation drift")? {
        let row = mapping(value, "operation drift")?;
        let identity = identity::operation_drift(row, 0)?;
        require_unique(&mut seen, &identity, "operation drift")?;
        collect_unknown(
            row,
            &["kind", "message"],
            &identity_child(&sequence_path, identity),
            manifest,
        )?;
    }
    Ok(())
}
