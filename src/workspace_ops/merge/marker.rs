use std::collections::{BTreeMap, BTreeSet};

use crate::artifact::{MarkerMergeArtifact, MarkerMergeParticipantArtifact, MarkerMergeTargetKind};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::{MergeOperationRecord, MergeTargetKind, OperationState, ParticipantState};

/// Live result already re-observed and accepted by finalization.
///
/// Marker conversion must compare these values with the durable participant
/// record. It does not perform Git I/O or silently adopt a different result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedMergeParticipant {
    pub target_id: String,
    pub target_branch: String,
    pub resulting_commit: String,
}

/// Convert one complete, verified participant set into additive marker
/// evidence. The result never contains the composition commit that will contain
/// the marker.
pub(crate) fn marker_merge_from_verified(
    record: &MergeOperationRecord,
    verified: &[VerifiedMergeParticipant],
) -> ModelResult<MarkerMergeArtifact> {
    if record.state != OperationState::Finalizing {
        return Err(recovery(format!(
            "merge '{}' must be finalizing before marker conversion",
            record.merge_id
        )));
    }
    if !record.operation_drift.is_empty() {
        return Err(drift("merge operation has unresolved drift"));
    }
    let selected: BTreeSet<_> = record.selected_targets.iter().map(String::as_str).collect();
    if selected.is_empty() {
        return Err(unreadable("selected targets are empty"));
    }
    if selected.len() != record.selected_targets.len() {
        return Err(unreadable("selected targets contain duplicates"));
    }
    if record.participants.len() != selected.len()
        || record
            .participants
            .keys()
            .any(|target| !selected.contains(target.as_str()))
    {
        return Err(unreadable(
            "participant records do not exactly match selected targets",
        ));
    }
    let mut observed = BTreeMap::new();
    for participant in verified {
        if observed
            .insert(participant.target_id.as_str(), participant)
            .is_some()
        {
            return Err(recovery(format!(
                "verified participant '{}' is duplicated",
                participant.target_id
            )));
        }
        if !selected.contains(participant.target_id.as_str()) {
            return Err(recovery(format!(
                "verified participant '{}' was not selected",
                participant.target_id
            )));
        }
    }

    let mut participants = BTreeMap::new();
    let mut root_merge_commit = None;
    for target_id in &record.selected_targets {
        let durable = record
            .participants
            .get(target_id)
            .ok_or_else(|| unreadable(format!("participant '{target_id}' is missing")))?;
        let live = observed.get(target_id.as_str()).ok_or_else(|| {
            recovery(format!("verified participant '{target_id}' is missing"))
                .with_member(target_id, &durable.path)
        })?;
        if durable.pending_action.is_some()
            || !durable.drift.is_empty()
            || durable.error.is_some()
            || !durable.conflict_paths.is_empty()
            || durable.expected_merge_head.is_some()
        {
            return Err(drift("participant has unresolved merge state")
                .with_member(target_id, &durable.path));
        }
        if !matches!(
            durable.state,
            ParticipantState::UpToDate
                | ParticipantState::FastForwarded
                | ParticipantState::Merged
                | ParticipantState::Continued
        ) {
            return Err(recovery(format!(
                "participant is in non-success state {:?}",
                durable.state
            ))
            .with_member(target_id, &durable.path));
        }
        let result = durable.resulting_commit.as_deref().ok_or_else(|| {
            unreadable(format!("participant '{target_id}' has no resulting commit"))
                .with_member(target_id, &durable.path)
        })?;
        if live.target_branch != durable.target_branch || live.resulting_commit != result {
            return Err(drift(
                "verified branch or resulting commit differs from the durable result",
            )
            .with_member(target_id, &durable.path));
        }
        let target_kind = match durable.target_kind {
            MergeTargetKind::Member if target_id != "@root" => MarkerMergeTargetKind::Member,
            MergeTargetKind::Root if target_id == "@root" => {
                root_merge_commit = Some(result.to_owned());
                MarkerMergeTargetKind::Root
            }
            _ => {
                return Err(unreadable(format!(
                    "participant '{target_id}' has an inconsistent target kind"
                )));
            }
        };
        participants.insert(
            target_id.clone(),
            MarkerMergeParticipantArtifact {
                target_kind,
                target_branch: durable.target_branch.clone(),
                before_commit: durable.before_commit.clone(),
                source_commit: durable.source_commit.clone(),
                resulting_commit: result.to_owned(),
            },
        );
    }
    if observed.len() != selected.len() {
        return Err(recovery("verified participant set is incomplete"));
    }
    let artifact = MarkerMergeArtifact {
        merge_id: record.merge_id.clone(),
        operation_id: record.operation_id.clone(),
        source_ref: record.source_ref.clone(),
        selected_targets: record.selected_targets.clone(),
        participants,
        root_merge_commit,
    };
    artifact.validate()?;
    Ok(artifact)
}

fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}

fn recovery(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}

fn drift(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeDrift, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORD: &str = r#"{schema: gwz.merge-operation/v0, record_schema_version: 0, writer_version: test, workspace_id: ws_test, merge_id: merge_1, operation_id: op_1, state: finalizing, source_ref: feature/x, created_at: now, baseline: {lock_sha256: lock, manifest_sha256: manifest}, selected_targets: [mem_b, '@root', mem_a], participants: {mem_a: {path: a, target_kind: member, target_branch: main, before_commit: a0, source_commit: as, commit_message: m, state: merged, resulting_commit: a1}, mem_b: {path: b, target_kind: member, target_branch: release, before_commit: b0, source_commit: bs, commit_message: m, state: up_to_date, resulting_commit: b0}, '@root': {path: '.', target_kind: root, target_branch: main, before_commit: r0, source_commit: rs, commit_message: m, state: fast_forwarded, resulting_commit: r1}}}"#;

    fn record() -> MergeOperationRecord {
        serde_yaml::from_str(RECORD).unwrap()
    }

    fn changed(old: &str, new: &str) -> MergeOperationRecord {
        serde_yaml::from_str(&RECORD.replacen(old, new, 1)).unwrap()
    }

    fn row(id: &str, branch: &str, commit: &str) -> VerifiedMergeParticipant {
        VerifiedMergeParticipant {
            target_id: id.into(),
            target_branch: branch.into(),
            resulting_commit: commit.into(),
        }
    }

    fn verified() -> Vec<VerifiedMergeParticipant> {
        [
            ("mem_a", "main", "a1"),
            ("mem_b", "release", "b0"),
            ("@root", "main", "r1"),
        ]
        .map(|(id, branch, commit)| row(id, branch, commit))
        .into()
    }

    fn rejected(record: &MergeOperationRecord, verified: &[VerifiedMergeParticipant]) -> ErrorCode {
        marker_merge_from_verified(record, verified)
            .unwrap_err()
            .code
    }

    #[test]
    fn conversion_preserves_order_and_exact_member_and_root_evidence() {
        let marker = marker_merge_from_verified(&record(), &verified()).unwrap();
        assert_eq!(marker.selected_targets, ["mem_b", "@root", "mem_a"]);
        assert_eq!(marker.root_merge_commit.as_deref(), Some("r1"));
        let member = &marker.participants["mem_a"];
        assert_eq!(
            (
                member.target_kind,
                member.target_branch.as_str(),
                member.before_commit.as_str(),
                member.source_commit.as_str(),
                member.resulting_commit.as_str(),
            ),
            (MarkerMergeTargetKind::Member, "main", "a0", "as", "a1")
        );
        assert_eq!(
            marker.participants["@root"].target_kind,
            MarkerMergeTargetKind::Root
        );
    }

    #[test]
    fn conversion_rejects_incomplete_duplicate_extra_and_mismatched_observations() {
        let durable = record();
        let mut values = verified();
        values.pop();
        assert_eq!(
            rejected(&durable, &values),
            ErrorCode::MergeRecoveryRequired
        );
        let mut values = verified();
        values.push(values[0].clone());
        assert_eq!(
            rejected(&durable, &values),
            ErrorCode::MergeRecoveryRequired
        );
        let mut values = verified();
        values.push(row("mem_extra", "main", "x"));
        assert_eq!(
            rejected(&durable, &values),
            ErrorCode::MergeRecoveryRequired
        );
        let mut values = verified();
        values[0].target_branch = "other".into();
        assert_eq!(rejected(&durable, &values), ErrorCode::MergeDrift);
        let mut values = verified();
        values[0].resulting_commit = "wrong".into();
        assert_eq!(rejected(&durable, &values), ErrorCode::MergeDrift);
    }

    #[test]
    fn conversion_rejects_non_success_missing_result_pending_action_and_drift() {
        assert_eq!(
            rejected(
                &changed("state: finalizing", "state: executing"),
                &verified()
            ),
            ErrorCode::MergeRecoveryRequired
        );
        assert_eq!(
            rejected(
                &changed(
                    "selected_targets: [mem_b, '@root', mem_a]",
                    "selected_targets: []"
                ),
                &[]
            ),
            ErrorCode::MergeRecordUnreadable
        );
        assert_eq!(
            rejected(&changed("state: merged", "state: failed"), &verified()),
            ErrorCode::MergeRecoveryRequired
        );
        assert_eq!(
            rejected(&changed(", resulting_commit: a1", ""), &verified()),
            ErrorCode::MergeRecordUnreadable
        );
        assert_eq!(
            rejected(
                &changed(
                    "state: merged",
                    "state: merged, pending_action: {kind: true_merge, target_branch: main, before_commit: a0, source_commit: as, commit_message: m}"
                ),
                &verified()
            ),
            ErrorCode::MergeDrift
        );
        assert_eq!(
            rejected(
                &changed(
                    "state: merged",
                    "state: merged, drift: [{kind: head_advanced, message: drift, expected_head: a1, live_head: a2}]"
                ),
                &verified()
            ),
            ErrorCode::MergeDrift
        );
    }
}
