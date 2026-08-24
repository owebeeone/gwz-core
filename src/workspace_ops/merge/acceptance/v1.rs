use std::collections::{BTreeMap, BTreeSet};

use crate::artifact::{LockArtifact, ManifestArtifact};
use crate::model::ModelResult;
use crate::workspace_ops::merge::model::v1::{
    AcceptedAttachedCheckoutV1, AcceptedIntegrationRefV1, AcceptedLockMemberV1, AcceptedLockV1,
    AcceptedMetadataBaseV1, AcceptedMetadataSourceV1, AcceptedRootBaseV1, AcceptedWorkspaceV1,
    MemberAcceptanceV1, MergeOperationRecordV1, RootArtifactHashesV1, RootPublicationInputV1,
};
use crate::workspace_ops::merge::{
    MergeBaseline, MergeOperationRecord, MergeParticipantRecord, MergeTargetKind,
};

mod support;

use support::*;

#[derive(Clone, Copy)]
pub(in crate::workspace_ops::merge) enum V1AcceptanceRecord<'a> {
    V0(&'a MergeOperationRecord),
    V1(&'a MergeOperationRecordV1),
}

pub(in crate::workspace_ops::merge) enum V1AcceptanceMetadata<'a> {
    OperationBaseline,
    SelectedRootResult {
        commit: &'a str,
        manifest_exact_yaml: &'a str,
        lock_exact_yaml: &'a str,
    },
}

pub(in crate::workspace_ops::merge) struct BuiltV1Acceptance {
    accepted_workspace: AcceptedWorkspaceV1,
    publication_required: bool,
}

impl BuiltV1Acceptance {
    #[allow(
        dead_code,
        reason = "A1 activation: reached only by this tree's own suites; the compile gate's blanket `dead_code` allowance expired with the activation, so the residue is named item by item."
    )]
    pub(in crate::workspace_ops::merge) fn accepted_workspace(&self) -> &AcceptedWorkspaceV1 {
        &self.accepted_workspace
    }

    pub(in crate::workspace_ops::merge) fn into_accepted_workspace(self) -> AcceptedWorkspaceV1 {
        self.accepted_workspace
    }

    pub(in crate::workspace_ops::merge) fn publication_required(&self) -> bool {
        self.publication_required
    }
}

pub(in crate::workspace_ops::merge) fn build_v1_acceptance(
    source: V1AcceptanceRecord<'_>,
    metadata: V1AcceptanceMetadata<'_>,
) -> ModelResult<BuiltV1Acceptance> {
    let record = RecordView::from(source);
    let baseline_manifest_yaml = required(
        record.baseline.manifest_yaml.as_deref(),
        record.merge_id,
        "operation baseline manifest bytes are missing",
    )?;
    let baseline_lock_yaml = required(
        record.baseline.lock_yaml.as_deref(),
        record.merge_id,
        "operation baseline lock bytes are missing",
    )?;
    require_digest(
        baseline_manifest_yaml,
        &record.baseline.manifest_sha256,
        record.merge_id,
    )?;
    require_digest(
        baseline_lock_yaml,
        &record.baseline.lock_sha256,
        record.merge_id,
    )?;
    let baseline_manifest = parse_manifest(record.merge_id, baseline_manifest_yaml)?;
    let baseline_lock = parse_lock(record.merge_id, baseline_lock_yaml)?;
    require_workspace(
        record.workspace_id,
        &baseline_manifest,
        &baseline_lock,
        record.merge_id,
    )?;

    let selected_root = selected_root(&record)?;
    let (metadata_source, manifest_yaml, lock_yaml) = match (metadata, selected_root) {
        (V1AcceptanceMetadata::OperationBaseline, None) => (
            AcceptedMetadataSourceV1::OperationBaseline,
            baseline_manifest_yaml,
            baseline_lock_yaml,
        ),
        (
            V1AcceptanceMetadata::SelectedRootResult {
                commit,
                manifest_exact_yaml,
                lock_exact_yaml,
            },
            Some(root),
        ) if root.resulting_commit.as_deref() == Some(commit) => (
            AcceptedMetadataSourceV1::SelectedRootResult {
                commit: commit.to_owned(),
            },
            manifest_exact_yaml,
            lock_exact_yaml,
        ),
        _ => {
            return Err(input_error(
                record.merge_id,
                "metadata source does not match selected-root participation",
            ));
        }
    };
    let manifest = parse_manifest(record.merge_id, manifest_yaml)?;
    let metadata_lock = parse_lock(record.merge_id, lock_yaml)?;
    require_workspace(
        record.workspace_id,
        &manifest,
        &metadata_lock,
        record.merge_id,
    )?;

    let completed_lock = complete_lock(
        &record,
        &manifest,
        metadata_lock,
        &baseline_manifest,
        &baseline_lock,
    )?;
    let selected_members = record
        .selected_targets
        .iter()
        .filter(|target| target.as_str() != "@root")
        .cloned()
        .collect::<BTreeSet<_>>();
    let canonical_lock_yaml = render_complete_lock(
        record.merge_id,
        lock_yaml,
        baseline_lock_yaml,
        &completed_lock,
        &selected_members,
    )?;
    let accepted_lock_yaml = if let Some(candidate) = record.candidate_lock_yaml {
        if parse_lock(record.merge_id, candidate)? != completed_lock {
            return Err(input_error(
                record.merge_id,
                "persisted candidate lock differs from the complete accepted lock",
            ));
        }
        candidate.to_owned()
    } else {
        canonical_lock_yaml
    };
    let accepted_rows = parse_lock_rows(record.merge_id, &accepted_lock_yaml)?;
    let member_audit = member_audit(&record, &manifest, &accepted_rows)?;
    let root = root_input(&record, selected_root)?;
    let publication_required = record.participants.values().any(|participant| {
        participant
            .resulting_commit
            .as_deref()
            .is_some_and(|result| result != participant.before_commit)
    });
    if publication_required && root.publication_branch.is_none() {
        return Err(input_error(
            record.merge_id,
            "publication-required acceptance has no attached root branch",
        ));
    }
    let accepted_workspace = AcceptedWorkspaceV1 {
        operation_baseline_lock_sha256: record.baseline.lock_sha256.clone(),
        metadata_base: AcceptedMetadataBaseV1 {
            source: metadata_source,
            manifest_exact_yaml: manifest_yaml.to_owned(),
            manifest_sha256: digest(manifest_yaml),
            lock_exact_yaml: lock_yaml.to_owned(),
            lock_sha256: digest(lock_yaml),
        },
        lock: AcceptedLockV1 {
            exact_yaml: accepted_lock_yaml.clone(),
            sha256: digest(&accepted_lock_yaml),
        },
        member_audit,
        root,
    };
    Ok(BuiltV1Acceptance {
        accepted_workspace,
        publication_required,
    })
}

pub(in crate::workspace_ops::merge) fn classify_frozen_v1_publication(
    record: &MergeOperationRecordV1,
) -> ModelResult<bool> {
    let accepted = record
        .accepted_workspace
        .as_ref()
        .ok_or_else(|| input_error(&record.merge_id, "accepted workspace is missing"))?;
    let member_change = accepted.member_audit.values().any(|audit| {
        matches!(
            audit,
            MemberAcceptanceV1::Selected { integration, .. }
                if integration.resulting_commit != integration.before_commit
        )
    });
    let root_change = match &accepted.metadata_base.source {
        AcceptedMetadataSourceV1::OperationBaseline => {
            if record
                .selected_targets
                .iter()
                .any(|target| target == "@root")
            {
                return Err(input_error(
                    &record.merge_id,
                    "frozen acceptance lost its selected-root metadata source",
                ));
            }
            false
        }
        AcceptedMetadataSourceV1::SelectedRootResult { commit } => {
            let root = record
                .participants
                .get("@root")
                .ok_or_else(|| input_error(&record.merge_id, "selected root is missing"))?;
            if !record
                .selected_targets
                .iter()
                .any(|target| target == "@root")
                || root.resulting_commit.as_deref() != Some(commit.as_str())
            {
                return Err(input_error(
                    &record.merge_id,
                    "frozen selected-root result no longer matches acceptance",
                ));
            }
            commit != &root.before_commit
        }
    };
    let required = member_change || root_change;
    if required && accepted.root.publication_branch.is_none() {
        return Err(input_error(
            &record.merge_id,
            "frozen publication input has no attached root branch",
        ));
    }
    Ok(required)
}

struct RecordView<'a> {
    workspace_id: &'a str,
    merge_id: &'a str,
    baseline: &'a MergeBaseline,
    selected_targets: &'a [String],
    participants: &'a BTreeMap<String, MergeParticipantRecord>,
    candidate_lock_yaml: Option<&'a str>,
}

impl<'a> From<V1AcceptanceRecord<'a>> for RecordView<'a> {
    fn from(value: V1AcceptanceRecord<'a>) -> Self {
        match value {
            V1AcceptanceRecord::V0(record) => Self::new(
                &record.workspace_id,
                &record.merge_id,
                &record.baseline,
                &record.selected_targets,
                &record.participants,
                record
                    .publication
                    .as_ref()
                    .and_then(|progress| progress.candidate.as_ref())
                    .map(|candidate| candidate.lock_yaml.as_str()),
            ),
            V1AcceptanceRecord::V1(record) => Self::new(
                &record.workspace_id,
                &record.merge_id,
                &record.baseline,
                &record.selected_targets,
                &record.participants,
                record
                    .publication
                    .as_ref()
                    .and_then(|progress| progress.candidate.as_ref())
                    .map(|candidate| candidate.lock_yaml.as_str()),
            ),
        }
    }
}

impl<'a> RecordView<'a> {
    fn new(
        workspace_id: &'a str,
        merge_id: &'a str,
        baseline: &'a MergeBaseline,
        selected_targets: &'a [String],
        participants: &'a BTreeMap<String, MergeParticipantRecord>,
        candidate_lock_yaml: Option<&'a str>,
    ) -> Self {
        Self {
            workspace_id,
            merge_id,
            baseline,
            selected_targets,
            participants,
            candidate_lock_yaml,
        }
    }
}

fn complete_lock(
    record: &RecordView<'_>,
    manifest: &ManifestArtifact,
    mut lock: LockArtifact,
    baseline_manifest: &ManifestArtifact,
    baseline_lock: &LockArtifact,
) -> ModelResult<LockArtifact> {
    for target_id in record
        .selected_targets
        .iter()
        .filter(|target| target.as_str() != "@root")
    {
        let participant = record
            .participants
            .get(target_id)
            .ok_or_else(|| input_error(record.merge_id, "selected participant is missing"))?;
        let result = participant.resulting_commit.as_ref().ok_or_else(|| {
            input_error(record.merge_id, "selected participant result is missing")
        })?;
        if participant.target_kind != MergeTargetKind::Member
            || !super::super::participant_semantics::result::is_successful_result(participant.state)
        {
            return Err(input_error(
                record.merge_id,
                "selected participant result is not acceptance-ready",
            ));
        }
        let identity = selected_identity(
            target_id,
            participant,
            manifest,
            &lock,
            baseline_manifest,
            baseline_lock,
            record.merge_id,
        )?;
        let mut row = lock
            .members
            .remove(target_id)
            .or_else(|| baseline_lock.members.get(target_id).cloned())
            .unwrap_or_else(|| identity.to_lock_member());
        row.commit = Some(result.clone());
        row.branch = Some(participant.target_branch.clone());
        row.detached = Some(false);
        row.dirty = Some(false);
        row.materialized = Some(true);
        lock.members.insert(target_id.clone(), row);
    }
    Ok(lock)
}

fn member_audit(
    record: &RecordView<'_>,
    manifest: &ManifestArtifact,
    accepted_rows: &BTreeMap<String, AcceptedLockMemberV1>,
) -> ModelResult<BTreeMap<String, MemberAcceptanceV1>> {
    let selected = record
        .selected_targets
        .iter()
        .filter(|target| target.as_str() != "@root")
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut domain = accepted_rows.keys().cloned().collect::<BTreeSet<_>>();
    domain.extend(
        manifest
            .members
            .iter()
            .filter(|member| member.active)
            .map(|member| member.id.clone()),
    );
    domain.extend(selected.iter().cloned());
    domain
        .into_iter()
        .map(|member_id| {
            let audit = if selected.contains(&member_id) {
                let participant = record.participants.get(&member_id).ok_or_else(|| {
                    input_error(record.merge_id, "selected participant is missing")
                })?;
                let result = participant.resulting_commit.as_ref().ok_or_else(|| {
                    input_error(record.merge_id, "selected participant result is missing")
                })?;
                let lock_member = accepted_rows.get(&member_id).cloned().ok_or_else(|| {
                    input_error(record.merge_id, "selected accepted lock row is missing")
                })?;
                MemberAcceptanceV1::Selected {
                    integration: AcceptedIntegrationRefV1 {
                        branch: participant.target_branch.clone(),
                        before_commit: participant.before_commit.clone(),
                        resulting_commit: result.clone(),
                    },
                    final_checkout: AcceptedAttachedCheckoutV1 {
                        branch: participant.target_branch.clone(),
                        commit: result.clone(),
                    },
                    lock_member,
                }
            } else if let Some(lock_member) = accepted_rows.get(&member_id) {
                MemberAcceptanceV1::UnselectedPresent {
                    lock_member: lock_member.clone(),
                }
            } else {
                MemberAcceptanceV1::Absent
            };
            Ok((member_id, audit))
        })
        .collect()
}

fn selected_root<'a>(
    record: &'a RecordView<'_>,
) -> ModelResult<Option<&'a MergeParticipantRecord>> {
    let selected = record
        .selected_targets
        .iter()
        .any(|target| target == "@root");
    match (selected, record.participants.get("@root")) {
        (false, None) => Ok(None),
        (true, Some(root))
            if root.target_kind == MergeTargetKind::Root
                && root.path == "."
                && super::super::participant_semantics::result::is_successful_result(
                    root.state,
                ) =>
        {
            Ok(Some(root))
        }
        _ => Err(input_error(
            record.merge_id,
            "selected root identity or result is inconsistent",
        )),
    }
}

fn root_input(
    record: &RecordView<'_>,
    selected_root: Option<&MergeParticipantRecord>,
) -> ModelResult<RootPublicationInputV1> {
    let base = if let Some(root) = selected_root {
        if record.baseline.lock_commit_sha256.is_none()
            || record.baseline.manifest_commit_sha256.is_none()
        {
            return Err(input_error(
                record.merge_id,
                "selected root has no committed baseline artifact hashes",
            ));
        }
        AcceptedRootBaseV1::BornAttached {
            commit: root
                .resulting_commit
                .clone()
                .ok_or_else(|| input_error(record.merge_id, "root result is missing"))?,
            symbolic_branch: root.target_branch.clone(),
        }
    } else {
        match (&record.baseline.root_head, &record.baseline.root_branch) {
            (Some(commit), Some(branch)) => AcceptedRootBaseV1::BornAttached {
                commit: commit.clone(),
                symbolic_branch: branch.clone(),
            },
            (Some(commit), None) => AcceptedRootBaseV1::BornDetached {
                commit: commit.clone(),
            },
            (None, Some(branch)) => AcceptedRootBaseV1::UnbornAttached {
                symbolic_branch: branch.clone(),
            },
            (None, None) => {
                return Err(input_error(
                    record.merge_id,
                    "operation baseline has no accepted root checkout",
                ));
            }
        }
    };
    let publication_branch = match &base {
        AcceptedRootBaseV1::BornAttached {
            symbolic_branch, ..
        }
        | AcceptedRootBaseV1::UnbornAttached { symbolic_branch } => Some(symbolic_branch.clone()),
        AcceptedRootBaseV1::BornDetached { .. } => None,
    };
    Ok(RootPublicationInputV1 {
        base,
        publication_branch,
        baseline_artifact_hashes: RootArtifactHashesV1 {
            lock_worktree_sha256: record.baseline.lock_sha256.clone(),
            manifest_worktree_sha256: record.baseline.manifest_sha256.clone(),
            lock_commit_sha256: record.baseline.lock_commit_sha256.clone(),
            manifest_commit_sha256: record.baseline.manifest_commit_sha256.clone(),
        },
    })
}
