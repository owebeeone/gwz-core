use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::super::super::model::archive_projection::*;
#[cfg(test)]
use super::super::super::model::v1::MergeOperationRecordV1;
use super::super::super::{
    MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeOperationRecord, MergeTargetKind,
    OperationState, ParticipantState, PreservationEvidence, PublicationProgress,
};
use crate::artifact::{LOCK_PATH, LockArtifact, ManifestArtifact, MarkerArtifact};
use crate::workspace::MemberPath;

pub(super) struct BaselineEvidence {
    pub(super) manifest: Option<ManifestArtifact>,
    pub(super) lock: Option<LockArtifact>,
}

pub(super) struct CandidateEvidence {
    pub(super) lock: LockArtifact,
    pub(super) metadata_base_lock: LockArtifact,
    pub(super) exact_lock: bool,
    pub(super) composition_complete: bool,
    pub(super) publication_complete: bool,
}

pub(super) fn validate_common(record: &MergeOperationRecord) -> Result<(), ()> {
    validate_common_v0(record)?;
    if record.state.is_open() {
        return Err(());
    }
    if record.baseline.lock_commit_sha256.is_some()
        != record.baseline.manifest_commit_sha256.is_some()
    {
        return Err(());
    }
    for participant in record.participants.values() {
        if participant.pending_action.is_some() {
            return Err(());
        }
        let shape_ok = match (record.state, participant.state) {
            (OperationState::Completed, ParticipantState::UpToDate) => participant
                .resulting_commit
                .as_deref()
                .is_none_or(|result| result == participant.before_commit),
            (
                OperationState::Completed,
                ParticipantState::FastForwarded
                | ParticipantState::Merged
                | ParticipantState::Continued,
            ) => participant
                .resulting_commit
                .as_deref()
                .is_none_or(|result| result != participant.before_commit),
            (OperationState::Aborted, ParticipantState::Aborted) => participant
                .resulting_commit
                .as_deref()
                .is_none_or(|result| result == participant.before_commit),
            (OperationState::Aborted, ParticipantState::RolledBack) => participant
                .resulting_commit
                .as_deref()
                .is_none_or(|result| result != participant.before_commit),
            _ => false,
        };
        if !shape_ok {
            return Err(());
        }
        if participant.error.is_some() && !matches!(participant.state, ParticipantState::Aborted) {
            return Err(());
        }
        let conflict_ok = match participant.state {
            ParticipantState::Aborted => {
                participant.expected_merge_head.as_deref()
                    == Some(participant.source_commit.as_str())
                    || participant.expected_merge_head.is_none()
                        && participant.conflict_paths.is_empty()
                        && participant.conflict_snapshot.is_empty()
            }
            _ => {
                participant.expected_merge_head.is_none()
                    && participant.conflict_paths.is_empty()
                    && participant.conflict_snapshot.is_empty()
            }
        };
        if !conflict_ok {
            return Err(());
        }
    }
    Ok(())
}

pub(super) fn validate_baseline(record: &MergeOperationRecord) -> Result<BaselineEvidence, ()> {
    let manifest = record
        .baseline
        .manifest_yaml
        .as_deref()
        .map(|yaml| {
            if digest(yaml) != record.baseline.manifest_sha256 {
                return Err(());
            }
            let manifest = ManifestArtifact::from_yaml(yaml).map_err(|_| ())?;
            (manifest.workspace.id == record.workspace_id)
                .then_some(manifest)
                .ok_or(())
        })
        .transpose()?;
    let lock = record
        .baseline
        .lock_yaml
        .as_deref()
        .map(|yaml| {
            if digest(yaml) != record.baseline.lock_sha256 {
                return Err(());
            }
            let lock = LockArtifact::from_yaml(yaml).map_err(|_| ())?;
            (lock.workspace_id == record.workspace_id)
                .then_some(lock)
                .ok_or(())
        })
        .transpose()?;
    Ok(BaselineEvidence { manifest, lock })
}

pub(super) fn validate_candidate(
    record: &MergeOperationRecord,
    publication: &PublicationProgress,
) -> Result<CandidateEvidence, ()> {
    let candidate = publication.candidate.as_ref().ok_or(())?;
    let lock = LockArtifact::from_yaml(&candidate.lock_yaml).map_err(|_| ())?;
    let baseline_lock = LockArtifact::from_yaml(&candidate.baseline_lock_yaml).map_err(|_| ())?;
    let marker = MarkerArtifact::from_yaml(&candidate.marker_yaml).map_err(|_| ())?;
    if lock.workspace_id != record.workspace_id
        || baseline_lock.workspace_id != record.workspace_id
        || marker.workspace_id != record.workspace_id
        || marker.gwz_commit_id != candidate.marker_id
        || marker.created_by.actor_id != candidate.actor_id
        || marker.created_at != record.created_at
        || marker.root.branch.as_deref() != Some(candidate.root_branch.as_str())
        || marker.selected_targets != record.selected_targets
        || marker.members != lock.members
        || candidate.baseline_boundary_sha256 != digest(&candidate.baseline_boundary_text)
        || candidate.marker_sha256 != digest(&candidate.marker_yaml)
        || candidate.boundary_sha256 != digest(&candidate.boundary_text)
        || publication
            .preservation_prefix
            .as_deref()
            .is_some_and(|prefix| !matches!(prefix, "baseline" | "marker" | "lock" | "boundary"))
    {
        return Err(());
    }
    validate_marker_merge(record, publication, &marker, &lock)?;
    if publication.root_merge_commit.is_none()
        && digest(&candidate.baseline_lock_yaml) != record.baseline.lock_sha256
    {
        return Err(());
    }
    let expected_marker_path = format!(
        "{}/{}.yaml",
        crate::artifact::MARKER_DIR,
        candidate.marker_id
    );
    if publication
        .candidate_marker_path
        .as_deref()
        .is_some_and(|path| path != expected_marker_path)
        || publication
            .candidate_lock_sha256
            .as_deref()
            .is_some_and(|hash| hash != digest(&candidate.lock_yaml))
    {
        return Err(());
    }
    let selected_root = record.participants.get("@root");
    match (selected_root, publication.root_merge_commit.as_deref()) {
        (Some(root), Some(commit))
            if root
                .resulting_commit
                .as_deref()
                .is_none_or(|result| result == commit) => {}
        (Some(_), None) | (None, None) => {}
        _ => return Err(()),
    }
    let expected_hashes = BTreeMap::from([
        (LOCK_PATH.to_owned(), digest(&candidate.lock_yaml)),
        (expected_marker_path, digest(&candidate.marker_yaml)),
    ]);
    if publication
        .candidate_hashes
        .windows(2)
        .any(|rows| rows[0].path >= rows[1].path)
    {
        return Err(());
    }
    let mut seen = BTreeSet::new();
    for row in &publication.candidate_hashes {
        if !seen.insert(row.path.as_str()) || expected_hashes.get(&row.path) != Some(&row.sha256) {
            return Err(());
        }
    }
    for value in [
        publication.composition_commit.as_deref(),
        publication.composition_tree.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !is_oid(value) {
            return Err(());
        }
    }
    let composition_absent = publication.composition_commit.is_none()
        && publication.composition_tree.is_none()
        && publication.candidate_hashes.is_empty();
    let composition_complete = publication.composition_commit.is_some()
        && publication.composition_tree.is_some()
        && publication.candidate_hashes.len() == expected_hashes.len()
        && expected_hashes
            .keys()
            .eq(publication.candidate_hashes.iter().map(|row| &row.path));
    if !composition_absent && !composition_complete {
        return Err(());
    }
    if publication.evidence_rolled_back && !composition_complete {
        return Err(());
    }
    Ok(CandidateEvidence {
        lock,
        metadata_base_lock: baseline_lock,
        exact_lock: publication.candidate_lock_sha256.is_some(),
        composition_complete,
        publication_complete: composition_complete
            && publication.candidate_marker_path.is_some()
            && selected_root.is_none() == publication.root_merge_commit.is_none(),
    })
}

pub(super) fn validate_marker_merge(
    record: &MergeOperationRecord,
    publication: &PublicationProgress,
    marker: &MarkerArtifact,
    candidate_lock: &LockArtifact,
) -> Result<(), ()> {
    validate_marker_merge_view(marker_view_v0(record), publication, marker, candidate_lock)
}

#[cfg(test)]
pub(super) fn validate_marker_merge_v1(
    record: &MergeOperationRecordV1,
    publication: &PublicationProgress,
    marker: &MarkerArtifact,
    candidate_lock: &LockArtifact,
) -> Result<(), ()> {
    validate_marker_merge_view(marker_view_v1(record), publication, marker, candidate_lock)
}

struct MarkerMergeView<'a> {
    created_at: &'a str,
    merge_id: &'a str,
    operation_id: &'a str,
    source_ref: &'a str,
    selected_targets: &'a [String],
    participants: &'a BTreeMap<String, super::super::super::MergeParticipantRecord>,
    baseline: &'a super::super::super::MergeBaseline,
}

fn marker_view_v0(record: &MergeOperationRecord) -> MarkerMergeView<'_> {
    MarkerMergeView {
        created_at: &record.created_at,
        merge_id: &record.merge_id,
        operation_id: &record.operation_id,
        source_ref: &record.source_ref,
        selected_targets: &record.selected_targets,
        participants: &record.participants,
        baseline: &record.baseline,
    }
}

#[cfg(test)]
fn marker_view_v1(record: &MergeOperationRecordV1) -> MarkerMergeView<'_> {
    MarkerMergeView {
        created_at: &record.created_at,
        merge_id: &record.merge_id,
        operation_id: &record.operation_id,
        source_ref: &record.source_ref,
        selected_targets: &record.selected_targets,
        participants: &record.participants,
        baseline: &record.baseline,
    }
}

fn validate_marker_merge_view(
    record: MarkerMergeView<'_>,
    publication: &PublicationProgress,
    marker: &MarkerArtifact,
    candidate_lock: &LockArtifact,
) -> Result<(), ()> {
    let merge = marker.merge.as_ref().ok_or(())?;
    if marker.created_at != record.created_at
        || merge.merge_id != record.merge_id
        || merge.operation_id != record.operation_id
        || merge.source_ref != record.source_ref
        || merge.selected_targets != record.selected_targets
        || merge.participants.len() != record.participants.len()
        || merge.root_merge_commit != publication.root_merge_commit
    {
        return Err(());
    }
    for (target_id, participant) in record.participants {
        let row = merge.participants.get(target_id).ok_or(())?;
        let expected_kind = match participant.target_kind {
            super::super::super::MergeTargetKind::Member => {
                crate::artifact::MarkerMergeTargetKind::Member
            }
            super::super::super::MergeTargetKind::Root => {
                crate::artifact::MarkerMergeTargetKind::Root
            }
        };
        if row.target_kind != expected_kind
            || row.target_branch != participant.target_branch
            || row.before_commit != participant.before_commit
            || row.source_commit != participant.source_commit
            || participant
                .resulting_commit
                .as_deref()
                .is_some_and(|result| result != row.resulting_commit)
            || candidate_lock
                .members
                .get(target_id)
                .and_then(|lock_row| lock_row.commit.as_deref())
                .is_some_and(|result| result != row.resulting_commit)
            || !marker_result_matches_state(participant, &row.resulting_commit)
            || target_id == "@root"
                && publication.root_merge_commit.as_deref() != Some(row.resulting_commit.as_str())
        {
            return Err(());
        }
    }
    let selected_parent = merge
        .participants
        .get("@root")
        .map(|root| &root.resulting_commit);
    if let Some(expected_parent) = selected_parent.or(record.baseline.root_head.as_ref()) {
        if marker.root.before_commit.as_ref() != Some(expected_parent) {
            return Err(());
        }
    } else if record.baseline.root_branch.is_some() && marker.root.before_commit.is_some() {
        return Err(());
    }
    let mut committed = record
        .selected_targets
        .iter()
        .filter(|target_id| {
            merge
                .participants
                .get(*target_id)
                .zip(record.participants.get(*target_id))
                .is_some_and(|(row, participant)| row.resulting_commit != participant.before_commit)
        })
        .cloned()
        .collect::<Vec<_>>();
    if !committed.iter().any(|target| target == "@root") {
        committed.push("@root".to_owned());
    }
    (marker.committed_targets == committed)
        .then_some(())
        .ok_or(())
}

fn marker_result_matches_state(
    participant: &super::super::super::MergeParticipantRecord,
    result: &str,
) -> bool {
    match participant.state {
        ParticipantState::UpToDate | ParticipantState::Aborted => {
            result == participant.before_commit
        }
        ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued
        | ParticipantState::RolledBack => result != participant.before_commit,
        _ => false,
    }
}

pub(super) fn project_root(
    record: &MergeOperationRecord,
    publication_branch: Option<&str>,
) -> Result<Option<AcceptedRootProjection>, ()> {
    let (kind, commit, symbolic_branch, branch) =
        if let Some(root) = record.participants.get("@root") {
            if record.baseline.root_head.as_deref() != Some(root.before_commit.as_str())
                || record.baseline.root_branch.as_deref() != Some(root.target_branch.as_str())
                || record.baseline.lock_commit_sha256.is_none()
                || record.baseline.manifest_commit_sha256.is_none()
            {
                return Err(());
            }
            let Some(result) = root.resulting_commit.clone() else {
                return Ok(None);
            };
            (
                AcceptedRootKind::BornAttached,
                Some(result),
                Some(root.target_branch.clone()),
                Some(root.target_branch.clone()),
            )
        } else {
            match (&record.baseline.root_head, &record.baseline.root_branch) {
                (Some(commit), Some(branch)) => (
                    AcceptedRootKind::BornAttached,
                    Some(commit.clone()),
                    Some(branch.clone()),
                    Some(branch.clone()),
                ),
                (Some(commit), None) => (
                    AcceptedRootKind::BornDetached,
                    Some(commit.clone()),
                    None,
                    None,
                ),
                (None, Some(branch)) => (
                    AcceptedRootKind::UnbornAttached,
                    None,
                    Some(branch.clone()),
                    Some(branch.clone()),
                ),
                (None, None) => return Ok(None),
            }
        };
    if publication_branch.is_some() && publication_branch != branch.as_deref() {
        return Err(());
    }
    Ok(Some(AcceptedRootProjection {
        kind,
        commit,
        symbolic_branch,
        publication_branch: branch,
        lock_worktree_sha256: record.baseline.lock_sha256.clone(),
        manifest_worktree_sha256: record.baseline.manifest_sha256.clone(),
        lock_commit_sha256: record.baseline.lock_commit_sha256.clone(),
        manifest_commit_sha256: record.baseline.manifest_commit_sha256.clone(),
    }))
}

fn validate_common_v0(record: &MergeOperationRecord) -> Result<(), ()> {
    if record.schema != MERGE_RECORD_SCHEMA
        || record.record_schema_version != MERGE_RECORD_SCHEMA_VERSION
        || !portable_id(&record.workspace_id, "ws_")
        || !portable_id(&record.operation_id, "op_")
        || !slug(&record.merge_id)
        || !text(&record.writer_version)
        || !text(&record.source_ref)
        || !text(&record.created_at)
        || !sha256_hex(&record.baseline.lock_sha256)
        || !sha256_hex(&record.baseline.manifest_sha256)
        || record
            .baseline
            .lock_commit_sha256
            .as_deref()
            .is_some_and(|value| !sha256_hex(value))
        || record
            .baseline
            .manifest_commit_sha256
            .as_deref()
            .is_some_and(|value| !sha256_hex(value))
        || record
            .baseline
            .root_head
            .as_deref()
            .is_some_and(|value| !is_oid(value))
        || record
            .baseline
            .root_branch
            .as_deref()
            .is_some_and(|value| !short_branch(value))
    {
        return Err(());
    }
    let mut selected = BTreeSet::new();
    for target in &record.selected_targets {
        if !target_id(target)
            || !selected.insert(target.as_str())
            || !record.participants.contains_key(target)
        {
            return Err(());
        }
    }
    if selected.is_empty()
        || record
            .selected_targets
            .iter()
            .position(|target| target == "@root")
            .is_some_and(|position| position + 1 != record.selected_targets.len())
        || record.participants.len() != selected.len()
    {
        return Err(());
    }
    for (target, participant) in &record.participants {
        let identity = match participant.target_kind {
            MergeTargetKind::Root => target == "@root" && participant.path == ".",
            MergeTargetKind::Member => {
                target != "@root" && MemberPath::parse(&participant.path).is_ok()
            }
        };
        if !target_id(target)
            || !selected.contains(target.as_str())
            || !identity
            || !short_branch(&participant.target_branch)
            || !is_oid(&participant.before_commit)
            || !is_oid(&participant.source_commit)
            || !commit_message(record, &participant.commit_message)
            || participant
                .resulting_commit
                .as_deref()
                .is_some_and(|value| !is_oid(value))
            || participant
                .expected_merge_head
                .as_deref()
                .is_some_and(|value| !is_oid(value))
            || participant
                .conflict_snapshot
                .iter()
                .any(|row| !text(&row.path) || !sha256_hex(&row.sha256))
            || !preservation_rows(&participant.preservation)
        {
            return Err(());
        }
    }
    if record
        .publication
        .as_ref()
        .is_some_and(|publication| !preservation_rows(&publication.root_preservation))
    {
        return Err(());
    }
    Ok(())
}

fn preservation_rows(rows: &[PreservationEvidence]) -> bool {
    rows.iter().all(|row| {
        row.backup_ref.as_deref().is_none_or(text)
            && row.backup_commit.as_deref().is_none_or(is_oid)
            && row.stash_id.as_deref().is_none_or(text)
            && row.stash_object_id.as_deref().is_none_or(is_oid)
    })
}

fn commit_message(record: &MergeOperationRecord, message: &str) -> bool {
    let trailer = format!(
        "\n\nGWZ-Merge-ID: {}\nGWZ-Operation-ID: {}",
        record.merge_id, record.operation_id
    );
    let body = message.strip_suffix(&trailer).unwrap_or_default();
    !body.trim().is_empty() && !body.contains(['\0', '\r']) && !body.ends_with('\n')
}

fn target_id(value: &str) -> bool {
    value == "@root" || portable_id(value, "mem_")
}

fn portable_id(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && value.len() > prefix.len()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn slug(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn text(value: &str) -> bool {
    !value.trim().is_empty()
}

fn sha256_hex(value: &str) -> bool {
    value.len() == 64 && lower_hex(value)
}

fn short_branch(branch: &str) -> bool {
    let invalid_byte = branch.bytes().any(|byte| {
        byte <= b' '
            || byte == 0x7f
            || matches!(byte, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
    });
    let invalid_component = branch
        .split('/')
        .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"));
    !(branch.is_empty()
        || branch.starts_with("refs/")
        || branch.starts_with('-')
        || branch.ends_with('/')
        || branch.ends_with('.')
        || branch.contains("..")
        || branch.contains("@{")
        || invalid_byte
        || invalid_component)
}

fn lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn is_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
