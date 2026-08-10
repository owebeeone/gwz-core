use super::super::super::OperationState;
use super::super::super::model::archive_projection::*;
use super::super::super::model::v1::*;

pub(super) fn project(record: &MergeOperationRecordV1) -> Result<ArchivedMergeProjection, ()> {
    let terminal_outcome = match record.state {
        OperationState::Completed => ArchivedTerminalOutcome::Completed,
        OperationState::Aborted => ArchivedTerminalOutcome::Aborted,
        _ => return Err(()),
    };
    let acceptance = match record.accepted_workspace.as_ref() {
        Some(accepted) => ArchivedAcceptanceProjection::SupportedPersisted {
            workspace: InstalledAcceptedWorkspaceProjection::V1(project_workspace(accepted)),
        },
        None if terminal_outcome == ArchivedTerminalOutcome::Aborted => {
            ArchivedAcceptanceProjection::NotAccepted
        }
        None => return Err(()),
    };
    if let Some(publication) = record
        .publication
        .as_ref()
        .filter(|publication| publication.candidate.is_some())
    {
        let marker = crate::artifact::MarkerArtifact::from_yaml(
            &publication.candidate.as_ref().ok_or(())?.marker_yaml,
        )
        .map_err(|_| ())?;
        let candidate_lock = crate::artifact::LockArtifact::from_yaml(
            &publication.candidate.as_ref().ok_or(())?.lock_yaml,
        )
        .map_err(|_| ())?;
        super::v0_evidence::validate_marker_merge_v1(
            record,
            publication,
            &marker,
            &candidate_lock,
        )?;
    }
    Ok(ArchivedMergeProjection {
        source_version: ArchiveSourceVersion::V1,
        terminal_outcome,
        acceptance,
    })
}

fn project_workspace(accepted: &AcceptedWorkspaceV1) -> AcceptedWorkspaceV1Projection {
    let (metadata_source, source_commit) = match &accepted.metadata_base.source {
        AcceptedMetadataSourceV1::OperationBaseline => {
            (AcceptedMetadataSource::OperationBaseline, None)
        }
        AcceptedMetadataSourceV1::SelectedRootResult { commit } => (
            AcceptedMetadataSource::SelectedRootResult,
            Some(commit.clone()),
        ),
    };
    AcceptedWorkspaceV1Projection {
        operation_baseline_lock_sha256: accepted.operation_baseline_lock_sha256.clone(),
        metadata_base: AcceptedMetadataBaseProjection {
            source: metadata_source,
            source_commit,
            manifest_yaml: accepted.metadata_base.manifest_exact_yaml.clone(),
            manifest_sha256: accepted.metadata_base.manifest_sha256.clone(),
            lock_yaml: accepted.metadata_base.lock_exact_yaml.clone(),
            lock_sha256: accepted.metadata_base.lock_sha256.clone(),
        },
        lock_yaml: accepted.lock.exact_yaml.clone(),
        lock_sha256: accepted.lock.sha256.clone(),
        members: accepted
            .member_audit
            .iter()
            .map(|(member_id, member)| project_member(member_id, member))
            .collect(),
        root: project_root(&accepted.root),
    }
}

fn project_member(member_id: &str, member: &MemberAcceptanceV1) -> AcceptedMemberV1Projection {
    match member {
        MemberAcceptanceV1::Selected {
            integration,
            final_checkout,
            lock_member,
        } => AcceptedMemberV1Projection {
            member_id: member_id.to_owned(),
            kind: AcceptedMemberKind::Selected,
            integration: Some(AcceptedIntegrationProjection {
                branch: integration.branch.clone(),
                before_commit: integration.before_commit.clone(),
                resulting_commit: integration.resulting_commit.clone(),
            }),
            final_checkout: Some(AcceptedCheckoutProjection {
                branch: final_checkout.branch.clone(),
                commit: final_checkout.commit.clone(),
            }),
            lock_member: Some(project_lock_member(lock_member)),
        },
        MemberAcceptanceV1::UnselectedPresent { lock_member } => AcceptedMemberV1Projection {
            member_id: member_id.to_owned(),
            kind: AcceptedMemberKind::UnselectedPresent,
            integration: None,
            final_checkout: None,
            lock_member: Some(project_lock_member(lock_member)),
        },
        MemberAcceptanceV1::Absent => AcceptedMemberV1Projection {
            member_id: member_id.to_owned(),
            kind: AcceptedMemberKind::Absent,
            integration: None,
            final_checkout: None,
            lock_member: None,
        },
    }
}

fn project_lock_member(member: &AcceptedLockMemberV1) -> AcceptedLockMemberProjection {
    AcceptedLockMemberProjection {
        path: member.path.clone(),
        source_id: member.source_id.clone(),
        source_kind: member.source_kind,
        commit: member.commit.clone(),
        branch: member.branch.clone(),
        detached: member.detached,
        upstream: member.upstream.clone(),
        dirty: member.dirty,
        materialized: member.materialized,
    }
}

fn project_root(root: &RootPublicationInputV1) -> AcceptedRootProjection {
    let (kind, commit, symbolic_branch) = match &root.base {
        AcceptedRootBaseV1::BornAttached {
            commit,
            symbolic_branch,
        } => (
            AcceptedRootKind::BornAttached,
            Some(commit.clone()),
            Some(symbolic_branch.clone()),
        ),
        AcceptedRootBaseV1::BornDetached { commit } => {
            (AcceptedRootKind::BornDetached, Some(commit.clone()), None)
        }
        AcceptedRootBaseV1::UnbornAttached { symbolic_branch } => (
            AcceptedRootKind::UnbornAttached,
            None,
            Some(symbolic_branch.clone()),
        ),
    };
    AcceptedRootProjection {
        kind,
        commit,
        symbolic_branch,
        publication_branch: root.publication_branch.clone(),
        lock_worktree_sha256: root.baseline_artifact_hashes.lock_worktree_sha256.clone(),
        manifest_worktree_sha256: root
            .baseline_artifact_hashes
            .manifest_worktree_sha256
            .clone(),
        lock_commit_sha256: root.baseline_artifact_hashes.lock_commit_sha256.clone(),
        manifest_commit_sha256: root.baseline_artifact_hashes.manifest_commit_sha256.clone(),
    }
}
