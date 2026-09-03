use std::collections::{BTreeMap, BTreeSet};

use crate::artifact::{MarkerMergeArtifact, MarkerMergeParticipantArtifact, MarkerMergeTargetKind};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::{MergeTargetKind, OperationState};

use super::model::v1::{
    AcceptedMetadataSourceV1, AcceptedRootBaseV1, MemberAcceptanceV1, MergeOperationRecordV1,
};

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


struct MarkerMergeRecordView<'a> {
    state: OperationState,
    has_operation_drift: bool,
    merge_id: &'a str,
    operation_id: &'a str,
    source_ref: &'a str,
    selected_targets: &'a [String],
    participants: &'a BTreeMap<String, super::MergeParticipantRecord>,
}

fn marker_merge_from_view(
    record: MarkerMergeRecordView<'_>,
    verified: &[VerifiedMergeParticipant],
) -> ModelResult<MarkerMergeArtifact> {
    if record.state != OperationState::Finalizing {
        return Err(recovery(format!(
            "merge '{}' must be finalizing before marker conversion",
            record.merge_id
        )));
    }
    if record.has_operation_drift {
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
    for target_id in record.selected_targets {
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
        if !super::participant_semantics::result::is_successful_result(durable.state) {
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
        merge_id: record.merge_id.to_owned(),
        operation_id: record.operation_id.to_owned(),
        source_ref: record.source_ref.to_owned(),
        selected_targets: record.selected_targets.to_vec(),
        participants,
        root_merge_commit,
    };
    artifact.validate()?;
    Ok(artifact)
}

pub(in crate::workspace_ops::merge) fn marker_merge_from_v1_acceptance(
    record: &MergeOperationRecordV1,
) -> ModelResult<MarkerMergeArtifact> {
    let accepted = record
        .accepted_workspace
        .as_ref()
        .ok_or_else(|| unreadable("accepted workspace is missing"))?;
    let mut verified = Vec::with_capacity(record.selected_targets.len());
    for target_id in &record.selected_targets {
        let durable = record
            .participants
            .get(target_id)
            .ok_or_else(|| unreadable(format!("participant '{target_id}' is missing")))?;
        if target_id == "@root" {
            let AcceptedMetadataSourceV1::SelectedRootResult { commit } =
                &accepted.metadata_base.source
            else {
                return Err(unreadable(
                    "selected root acceptance metadata source is inconsistent",
                ));
            };
            let AcceptedRootBaseV1::BornAttached {
                commit: accepted_commit,
                symbolic_branch,
            } = &accepted.root.base
            else {
                return Err(unreadable("selected root acceptance base is inconsistent"));
            };
            if commit != accepted_commit
                || durable.target_branch != *symbolic_branch
                || accepted.root.publication_branch.as_deref() != Some(symbolic_branch.as_str())
                || durable.resulting_commit.as_deref() != Some(commit.as_str())
            {
                return Err(drift("accepted root differs from its durable result")
                    .with_member(target_id, &durable.path));
            }
            verified.push(VerifiedMergeParticipant {
                target_id: target_id.clone(),
                target_branch: symbolic_branch.clone(),
                resulting_commit: commit.clone(),
            });
            continue;
        }
        let MemberAcceptanceV1::Selected {
            integration,
            final_checkout,
            ..
        } = accepted
            .member_audit
            .get(target_id)
            .ok_or_else(|| unreadable(format!("acceptance row '{target_id}' is missing")))?
        else {
            return Err(unreadable(format!(
                "acceptance row '{target_id}' is not selected"
            )));
        };
        if integration.branch != durable.target_branch
            || integration.before_commit != durable.before_commit
            || Some(integration.resulting_commit.as_str()) != durable.resulting_commit.as_deref()
            || final_checkout.branch != integration.branch
            || final_checkout.commit != integration.resulting_commit
        {
            return Err(
                drift("accepted participant differs from its durable result")
                    .with_member(target_id, &durable.path),
            );
        }
        verified.push(VerifiedMergeParticipant {
            target_id: target_id.clone(),
            target_branch: integration.branch.clone(),
            resulting_commit: integration.resulting_commit.clone(),
        });
    }
    marker_merge_from_view(
        MarkerMergeRecordView {
            // Candidate creation checks the Finalizing predecessor. Frozen
            // marker semantics remain valid after later lifecycle advances.
            state: OperationState::Finalizing,
            has_operation_drift: false,
            merge_id: &record.merge_id,
            operation_id: &record.operation_id,
            source_ref: &record.source_ref,
            selected_targets: &record.selected_targets,
            participants: &record.participants,
        },
        &verified,
    )
}

pub(in crate::workspace_ops::merge) fn selected_v1_result_changed(
    record: &MergeOperationRecordV1,
    target_id: &str,
) -> ModelResult<bool> {
    if target_id == "@root" {
        let record_root = record
            .participants
            .get(target_id)
            .ok_or_else(|| unreadable("selected root participant is missing"))?;
        let AcceptedMetadataSourceV1::SelectedRootResult { commit } = &record
            .accepted_workspace
            .as_ref()
            .ok_or_else(|| unreadable("accepted workspace is missing"))?
            .metadata_base
            .source
        else {
            return Err(unreadable(
                "selected root acceptance metadata source is inconsistent",
            ));
        };
        if record_root.resulting_commit.as_deref() != Some(commit.as_str()) {
            return Err(unreadable(
                "selected root acceptance differs from its durable result",
            ));
        }
        return Ok(commit != &record_root.before_commit);
    }
    let MemberAcceptanceV1::Selected { integration, .. } = record
        .accepted_workspace
        .as_ref()
        .and_then(|accepted| accepted.member_audit.get(target_id))
        .ok_or_else(|| unreadable(format!("selected acceptance row '{target_id}' is missing")))?
    else {
        return Err(unreadable(format!(
            "acceptance row '{target_id}' is not selected"
        )));
    };
    Ok(integration.resulting_commit != integration.before_commit)
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

    const RECORD: &str = r#"{schema: gwz.merge-operation/v1, record_schema_version: 1, writer_version: test, workspace_id: ws_test, merge_id: merge_1, operation_id: op_1, state: finalizing, source_ref: feature/x, created_at: now, baseline: {lock_sha256: lock, manifest_sha256: manifest}, selected_targets: [mem_b, '@root', mem_a], participants: {mem_a: {path: a, target_kind: member, target_branch: main, before_commit: a0, source_commit: as, commit_message: m, state: merged, resulting_commit: a1}, mem_b: {path: b, target_kind: member, target_branch: release, before_commit: b0, source_commit: bs, commit_message: m, state: up_to_date, resulting_commit: b0}, '@root': {path: '.', target_kind: root, target_branch: main, before_commit: r0, source_commit: rs, commit_message: m, state: fast_forwarded, resulting_commit: r1}}}"#;

    fn record() -> MergeOperationRecordV1 {
        serde_yaml::from_str(RECORD).unwrap()
    }

    fn changed(old: &str, new: &str) -> MergeOperationRecordV1 {
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

    /// The v0 engine's `marker_merge_from_verified` entry left with it; the
    /// property it pinned belongs to the view, which both the v1 marker path
    /// and these cases share.
    fn from_record(
        record: &MergeOperationRecordV1,
        verified: &[VerifiedMergeParticipant],
    ) -> ModelResult<MarkerMergeArtifact> {
        marker_merge_from_view(
            MarkerMergeRecordView {
                state: record.state,
                has_operation_drift: !record.operation_drift.is_empty(),
                merge_id: &record.merge_id,
                operation_id: &record.operation_id,
                source_ref: &record.source_ref,
                selected_targets: &record.selected_targets,
                participants: &record.participants,
            },
            verified,
        )
    }

    fn rejected(record: &MergeOperationRecordV1, verified: &[VerifiedMergeParticipant]) -> ErrorCode {
        from_record(record, verified)
            .unwrap_err()
            .code
    }

    #[test]
    fn conversion_preserves_order_and_exact_member_and_root_evidence() {
        let marker = from_record(&record(), &verified()).unwrap();
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
