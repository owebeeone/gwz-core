use super::MergeOperationRecord;

pub(crate) fn project_open_v0(_record: &MergeOperationRecord) -> crate::MergeRecordProjection {
    crate::MergeRecordProjection {
        source_version: crate::MergeRecordVersion::V0,
        archived: false,
        terminal_outcome: None,
        acceptance: None,
        recovery: None,
    }
}

pub(crate) fn project_open_v1(
    record: &super::v1::MergeOperationRecordV1,
) -> crate::MergeRecordProjection {
    crate::MergeRecordProjection {
        source_version: crate::MergeRecordVersion::V1,
        archived: false,
        terminal_outcome: None,
        acceptance: record.accepted_workspace.as_ref().map(|workspace| {
            crate::MergeAcceptanceProjection {
                kind: crate::MergeAcceptanceKind::SupportedPersisted,
                supported_persisted: Some(crate::MergeInstalledAcceptedWorkspaceProjection {
                    kind: crate::MergeInstalledAcceptedWorkspaceKind::V1,
                    v1: Some(project_accepted_v1(workspace)),
                }),
                legacy_complete: None,
                legacy_source: None,
                legacy_evidence: None,
                missing_gaps: Vec::new(),
            }
        }),
        recovery: record
            .recovery_context
            .as_ref()
            .map(|context| project_recovery(record, context.origin_state)),
    }
}

pub(crate) fn project_archived(
    archived: &super::archive_projection::ArchivedMergeProjection,
) -> crate::MergeRecordProjection {
    crate::MergeRecordProjection {
        source_version: match archived.source_version {
            super::archive_projection::ArchiveSourceVersion::V0 => crate::MergeRecordVersion::V0,
            super::archive_projection::ArchiveSourceVersion::V1 => crate::MergeRecordVersion::V1,
        },
        archived: true,
        terminal_outcome: Some(match archived.terminal_outcome {
            super::archive_projection::ArchivedTerminalOutcome::Completed => {
                crate::MergeTerminalOutcome::Completed
            }
            super::archive_projection::ArchivedTerminalOutcome::Aborted => {
                crate::MergeTerminalOutcome::Aborted
            }
        }),
        acceptance: Some(project_archived_acceptance(&archived.acceptance)),
        recovery: None,
    }
}

fn project_archived_acceptance(
    acceptance: &super::archive_projection::ArchivedAcceptanceProjection,
) -> crate::MergeAcceptanceProjection {
    use super::archive_projection::ArchivedAcceptanceProjection as Acceptance;
    match acceptance {
        Acceptance::SupportedPersisted { workspace } => crate::MergeAcceptanceProjection {
            kind: crate::MergeAcceptanceKind::SupportedPersisted,
            supported_persisted: Some(project_installed(workspace)),
            legacy_complete: None,
            legacy_source: None,
            legacy_evidence: None,
            missing_gaps: Vec::new(),
        },
        Acceptance::LegacyComplete { workspace, source } => crate::MergeAcceptanceProjection {
            kind: crate::MergeAcceptanceKind::LegacyComplete,
            supported_persisted: None,
            legacy_complete: Some(project_legacy_workspace(workspace)),
            legacy_source: Some(project_legacy_source(*source)),
            legacy_evidence: None,
            missing_gaps: Vec::new(),
        },
        Acceptance::LegacyUnavailable { available, missing } => crate::MergeAcceptanceProjection {
            kind: crate::MergeAcceptanceKind::LegacyUnavailable,
            supported_persisted: None,
            legacy_complete: None,
            legacy_source: None,
            legacy_evidence: Some(project_legacy_evidence(available)),
            missing_gaps: missing.iter().copied().map(project_legacy_gap).collect(),
        },
        Acceptance::NotAccepted => crate::MergeAcceptanceProjection {
            kind: crate::MergeAcceptanceKind::NotAccepted,
            supported_persisted: None,
            legacy_complete: None,
            legacy_source: None,
            legacy_evidence: None,
            missing_gaps: Vec::new(),
        },
    }
}

fn project_installed(
    workspace: &super::archive_projection::InstalledAcceptedWorkspaceProjection,
) -> crate::MergeInstalledAcceptedWorkspaceProjection {
    match workspace {
        super::archive_projection::InstalledAcceptedWorkspaceProjection::V1(value) => {
            crate::MergeInstalledAcceptedWorkspaceProjection {
                kind: crate::MergeInstalledAcceptedWorkspaceKind::V1,
                v1: Some(project_archived_v1(value)),
            }
        }
    }
}

fn project_archived_v1(
    value: &super::archive_projection::AcceptedWorkspaceV1Projection,
) -> crate::MergeAcceptedWorkspaceV1Projection {
    crate::MergeAcceptedWorkspaceV1Projection {
        operation_baseline_lock_sha256: value.operation_baseline_lock_sha256.clone(),
        metadata_base: project_metadata(&value.metadata_base),
        lock_yaml: value.lock_yaml.clone(),
        lock_sha256: value.lock_sha256.clone(),
        members: value.members.iter().map(project_member).collect(),
        root: project_root(&value.root),
    }
}

fn project_legacy_workspace(
    value: &super::archive_projection::ArchivedAcceptedWorkspace,
) -> crate::MergeLegacyAcceptedWorkspace {
    crate::MergeLegacyAcceptedWorkspace {
        baseline_lock_sha256: value.baseline_lock_sha256.clone(),
        lock_yaml: value.lock_yaml.clone(),
        lock_sha256: value.lock_sha256.clone(),
        members: value.members.iter().map(project_member).collect(),
        root: project_root(&value.root),
    }
}

fn project_legacy_evidence(
    value: &super::archive_projection::LegacyAcceptanceEvidence,
) -> crate::MergeLegacyAcceptanceEvidence {
    crate::MergeLegacyAcceptanceEvidence {
        lock_yaml: value.lock_yaml.clone(),
        lock_sha256: value.lock_sha256.clone(),
        members: value
            .members
            .iter()
            .map(|member| crate::MergeLegacyMemberEvidence {
                member_id: member.member_id.clone(),
                selected: member.selected,
                state: member
                    .state
                    .map(super::super::participant_semantics::result::wire_state),
                integration: member.integration.as_ref().map(project_integration),
                lock_member: member.lock_member.as_ref().map(project_lock_member),
            })
            .collect(),
        root: value.root.as_ref().map(project_root),
        composition_commit: value.composition_commit.clone(),
        composition_tree: value.composition_tree.clone(),
        candidate_hashes: value
            .candidate_hashes
            .iter()
            .map(|hash| crate::MergeAcceptedCandidateHashProjection {
                path: hash.path.clone(),
                sha256: hash.sha256.clone(),
            })
            .collect(),
    }
}

fn project_accepted_v1(
    value: &super::v1::AcceptedWorkspaceV1,
) -> crate::MergeAcceptedWorkspaceV1Projection {
    use super::v1::{AcceptedMetadataSourceV1, MemberAcceptanceV1};
    let (source, source_commit) = match &value.metadata_base.source {
        AcceptedMetadataSourceV1::OperationBaseline => {
            (crate::MergeAcceptedMetadataSource::OperationBaseline, None)
        }
        AcceptedMetadataSourceV1::SelectedRootResult { commit } => (
            crate::MergeAcceptedMetadataSource::SelectedRootResult,
            Some(commit.clone()),
        ),
    };
    let members = value
        .member_audit
        .iter()
        .map(|(member_id, member)| {
            let (kind, integration, final_checkout, lock_member) = match member {
                MemberAcceptanceV1::Selected {
                    integration,
                    final_checkout,
                    lock_member,
                } => (
                    crate::MergeAcceptedMemberKind::Selected,
                    Some(crate::MergeAcceptedIntegrationProjection {
                        branch: integration.branch.clone(),
                        before_commit: integration.before_commit.clone(),
                        resulting_commit: integration.resulting_commit.clone(),
                    }),
                    Some(crate::MergeAcceptedCheckoutProjection {
                        branch: final_checkout.branch.clone(),
                        commit: final_checkout.commit.clone(),
                    }),
                    Some(project_v1_lock_member(lock_member)),
                ),
                MemberAcceptanceV1::UnselectedPresent { lock_member } => (
                    crate::MergeAcceptedMemberKind::UnselectedPresent,
                    None,
                    None,
                    Some(project_v1_lock_member(lock_member)),
                ),
                MemberAcceptanceV1::Absent => {
                    (crate::MergeAcceptedMemberKind::Absent, None, None, None)
                }
            };
            crate::MergeAcceptedMemberV1Projection {
                member_id: member_id.clone(),
                kind,
                integration,
                final_checkout,
                lock_member,
            }
        })
        .collect();
    crate::MergeAcceptedWorkspaceV1Projection {
        operation_baseline_lock_sha256: value.operation_baseline_lock_sha256.clone(),
        metadata_base: crate::MergeAcceptedMetadataBaseProjection {
            source,
            source_commit,
            manifest_yaml: value.metadata_base.manifest_exact_yaml.clone(),
            manifest_sha256: value.metadata_base.manifest_sha256.clone(),
            lock_yaml: value.metadata_base.lock_exact_yaml.clone(),
            lock_sha256: value.metadata_base.lock_sha256.clone(),
        },
        lock_yaml: value.lock.exact_yaml.clone(),
        lock_sha256: value.lock.sha256.clone(),
        members,
        root: project_v1_root(&value.root),
    }
}

fn project_recovery(
    record: &super::v1::MergeOperationRecordV1,
    origin: super::v1::RecoveryOriginStateV1,
) -> crate::MergeRecoveryProjection {
    use super::v1::RecoveryOriginStateV1 as Origin;
    let origin_state = match origin {
        Origin::Executing => crate::MergeRecoveryOriginState::Executing,
        Origin::AwaitingResolution => crate::MergeRecoveryOriginState::AwaitingResolution,
        Origin::Halted => crate::MergeRecoveryOriginState::Halted,
        Origin::Finalizing => crate::MergeRecoveryOriginState::Finalizing,
        Origin::Preserving => crate::MergeRecoveryOriginState::Preserving,
        Origin::RollingBack => crate::MergeRecoveryOriginState::RollingBack,
    };
    let resume_action = match origin {
        Origin::Executing
            if record
                .participants
                .values()
                .any(|row| row.pending_action.is_some()) =>
        {
            crate::MergeCompatibilityNextAction::ReconcilePendingParticipant
        }
        Origin::Executing | Origin::Halted => {
            crate::MergeCompatibilityNextAction::ExecuteNextParticipant
        }
        Origin::AwaitingResolution => crate::MergeCompatibilityNextAction::AwaitResolution,
        Origin::Finalizing => finalization_resume(record),
        Origin::Preserving => crate::MergeCompatibilityNextAction::ResumePreservation,
        Origin::RollingBack => crate::MergeCompatibilityNextAction::ResumeRollback,
    };
    crate::MergeRecoveryProjection {
        origin_state,
        base_phase: compatibility_base_phase(record),
        next_action: crate::MergeCompatibilityNextAction::ReportRecoveryRequired,
        resume_action,
    }
}

fn compatibility_base_phase(
    record: &super::v1::MergeOperationRecordV1,
) -> crate::MergeCompatibilityBasePhase {
    use super::PublicationStep as Step;
    let Some(publication) = record.publication.as_ref() else {
        return if record.accepted_workspace.is_some() {
            crate::MergeCompatibilityBasePhase::PreCandidate
        } else {
            crate::MergeCompatibilityBasePhase::PreAcceptance
        };
    };
    if publication.candidate.is_none() {
        return if publication.step == Step::Complete {
            crate::MergeCompatibilityBasePhase::NoPublicationComplete
        } else {
            crate::MergeCompatibilityBasePhase::PreCandidate
        };
    }
    if publication.composition_commit.is_none() {
        return if publication.step == Step::CommittingEvidence {
            crate::MergeCompatibilityBasePhase::EvidenceUnrecorded
        } else {
            crate::MergeCompatibilityBasePhase::CandidatePersisted
        };
    }
    match publication.step {
        Step::CommittingEvidence => crate::MergeCompatibilityBasePhase::EvidenceRecorded,
        Step::PublishingCandidate => crate::MergeCompatibilityBasePhase::PublishingPrefix,
        Step::VerifyingPublication | Step::Complete => {
            crate::MergeCompatibilityBasePhase::Published
        }
        _ => crate::MergeCompatibilityBasePhase::EvidenceRecorded,
    }
}

fn finalization_resume(
    record: &super::v1::MergeOperationRecordV1,
) -> crate::MergeCompatibilityNextAction {
    use super::PublicationStep as Step;
    let Some(publication) = record.publication.as_ref() else {
        return if record.accepted_workspace.is_none() {
            crate::MergeCompatibilityNextAction::PersistAcceptance
        } else {
            crate::MergeCompatibilityNextAction::ValidateResults
        };
    };
    match publication.step {
        Step::NotStarted | Step::ValidatingResults => {
            crate::MergeCompatibilityNextAction::ValidateResults
        }
        Step::PreparingCandidate if publication.candidate.is_none() => {
            crate::MergeCompatibilityNextAction::PrepareCandidate
        }
        Step::PreparingCandidate | Step::CommittingEvidence
            if publication.composition_commit.is_none() =>
        {
            crate::MergeCompatibilityNextAction::CreateOrAdoptEvidence
        }
        Step::CommittingEvidence | Step::PublishingCandidate => {
            crate::MergeCompatibilityNextAction::PublishCandidate
        }
        Step::VerifyingPublication => crate::MergeCompatibilityNextAction::VerifyPublication,
        Step::Complete if publication.candidate.is_none() => {
            crate::MergeCompatibilityNextAction::CompleteNoPublication
        }
        Step::Complete => crate::MergeCompatibilityNextAction::VerifyPublication,
        Step::PreparingCandidate => crate::MergeCompatibilityNextAction::CreateOrAdoptEvidence,
    }
}

fn project_metadata(
    value: &super::archive_projection::AcceptedMetadataBaseProjection,
) -> crate::MergeAcceptedMetadataBaseProjection {
    crate::MergeAcceptedMetadataBaseProjection {
        source: match value.source {
            super::archive_projection::AcceptedMetadataSource::OperationBaseline => {
                crate::MergeAcceptedMetadataSource::OperationBaseline
            }
            super::archive_projection::AcceptedMetadataSource::SelectedRootResult => {
                crate::MergeAcceptedMetadataSource::SelectedRootResult
            }
        },
        source_commit: value.source_commit.clone(),
        manifest_yaml: value.manifest_yaml.clone(),
        manifest_sha256: value.manifest_sha256.clone(),
        lock_yaml: value.lock_yaml.clone(),
        lock_sha256: value.lock_sha256.clone(),
    }
}

fn project_member(
    value: &super::archive_projection::AcceptedMemberV1Projection,
) -> crate::MergeAcceptedMemberV1Projection {
    crate::MergeAcceptedMemberV1Projection {
        member_id: value.member_id.clone(),
        kind: match value.kind {
            super::archive_projection::AcceptedMemberKind::Selected => {
                crate::MergeAcceptedMemberKind::Selected
            }
            super::archive_projection::AcceptedMemberKind::UnselectedPresent => {
                crate::MergeAcceptedMemberKind::UnselectedPresent
            }
            super::archive_projection::AcceptedMemberKind::Absent => {
                crate::MergeAcceptedMemberKind::Absent
            }
        },
        integration: value.integration.as_ref().map(project_integration),
        final_checkout: value.final_checkout.as_ref().map(|checkout| {
            crate::MergeAcceptedCheckoutProjection {
                branch: checkout.branch.clone(),
                commit: checkout.commit.clone(),
            }
        }),
        lock_member: value.lock_member.as_ref().map(project_lock_member),
    }
}

fn project_integration(
    value: &super::archive_projection::AcceptedIntegrationProjection,
) -> crate::MergeAcceptedIntegrationProjection {
    crate::MergeAcceptedIntegrationProjection {
        branch: value.branch.clone(),
        before_commit: value.before_commit.clone(),
        resulting_commit: value.resulting_commit.clone(),
    }
}

fn project_lock_member(
    value: &super::archive_projection::AcceptedLockMemberProjection,
) -> crate::MergeAcceptedLockMemberProjection {
    crate::MergeAcceptedLockMemberProjection {
        path: value.path.clone(),
        source_id: value.source_id.clone(),
        source_kind: value.source_kind.into(),
        commit: value.commit.clone(),
        branch: value.branch.clone(),
        detached: value.detached,
        upstream: value.upstream.clone(),
        dirty: value.dirty,
        materialized: value.materialized,
    }
}

fn project_v1_lock_member(
    value: &super::v1::AcceptedLockMemberV1,
) -> crate::MergeAcceptedLockMemberProjection {
    crate::MergeAcceptedLockMemberProjection {
        path: value.path.clone(),
        source_id: value.source_id.clone(),
        source_kind: value.source_kind.into(),
        commit: value.commit.clone(),
        branch: value.branch.clone(),
        detached: value.detached,
        upstream: value.upstream.clone(),
        dirty: value.dirty,
        materialized: value.materialized,
    }
}

fn project_root(
    value: &super::archive_projection::AcceptedRootProjection,
) -> crate::MergeAcceptedRootProjection {
    crate::MergeAcceptedRootProjection {
        kind: match value.kind {
            super::archive_projection::AcceptedRootKind::BornAttached => {
                crate::MergeAcceptedRootKind::BornAttached
            }
            super::archive_projection::AcceptedRootKind::BornDetached => {
                crate::MergeAcceptedRootKind::BornDetached
            }
            super::archive_projection::AcceptedRootKind::UnbornAttached => {
                crate::MergeAcceptedRootKind::UnbornAttached
            }
        },
        commit: value.commit.clone(),
        symbolic_branch: value.symbolic_branch.clone(),
        publication_branch: value.publication_branch.clone(),
        lock_worktree_sha256: value.lock_worktree_sha256.clone(),
        manifest_worktree_sha256: value.manifest_worktree_sha256.clone(),
        lock_commit_sha256: value.lock_commit_sha256.clone(),
        manifest_commit_sha256: value.manifest_commit_sha256.clone(),
    }
}

fn project_v1_root(
    value: &super::v1::RootPublicationInputV1,
) -> crate::MergeAcceptedRootProjection {
    use super::v1::AcceptedRootBaseV1;
    let (kind, commit, symbolic_branch) = match &value.base {
        AcceptedRootBaseV1::BornAttached {
            commit,
            symbolic_branch,
        } => (
            crate::MergeAcceptedRootKind::BornAttached,
            Some(commit.clone()),
            Some(symbolic_branch.clone()),
        ),
        AcceptedRootBaseV1::BornDetached { commit } => (
            crate::MergeAcceptedRootKind::BornDetached,
            Some(commit.clone()),
            None,
        ),
        AcceptedRootBaseV1::UnbornAttached { symbolic_branch } => (
            crate::MergeAcceptedRootKind::UnbornAttached,
            None,
            Some(symbolic_branch.clone()),
        ),
    };
    crate::MergeAcceptedRootProjection {
        kind,
        commit,
        symbolic_branch,
        publication_branch: value.publication_branch.clone(),
        lock_worktree_sha256: value.baseline_artifact_hashes.lock_worktree_sha256.clone(),
        manifest_worktree_sha256: value
            .baseline_artifact_hashes
            .manifest_worktree_sha256
            .clone(),
        lock_commit_sha256: value.baseline_artifact_hashes.lock_commit_sha256.clone(),
        manifest_commit_sha256: value
            .baseline_artifact_hashes
            .manifest_commit_sha256
            .clone(),
    }
}

fn project_legacy_source(
    value: super::archive_projection::LegacyAcceptanceSource,
) -> crate::MergeLegacyAcceptanceSource {
    match value {
        super::archive_projection::LegacyAcceptanceSource::Candidate => {
            crate::MergeLegacyAcceptanceSource::Candidate
        }
        super::archive_projection::LegacyAcceptanceSource::BaselineNoPublication => {
            crate::MergeLegacyAcceptanceSource::BaselineNoPublication
        }
    }
}

fn project_legacy_gap(
    value: super::archive_projection::LegacyAcceptanceGap,
) -> crate::MergeLegacyAcceptanceGap {
    match value {
        super::archive_projection::LegacyAcceptanceGap::ExactLockBytes => {
            crate::MergeLegacyAcceptanceGap::ExactLockBytes
        }
        super::archive_projection::LegacyAcceptanceGap::CompleteMemberAudit => {
            crate::MergeLegacyAcceptanceGap::CompleteMemberAudit
        }
        super::archive_projection::LegacyAcceptanceGap::AcceptedRootInput => {
            crate::MergeLegacyAcceptanceGap::AcceptedRootInput
        }
        super::archive_projection::LegacyAcceptanceGap::PublicationEvidence => {
            crate::MergeLegacyAcceptanceGap::PublicationEvidence
        }
    }
}
