#![forbid(clippy::disallowed_methods)]

use super::*;

pub(super) fn plan_participant<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<PreservationPlan> {
    if participant.pending_action.is_some() {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "completed participant still has a pending merge action",
        )
        .with_member(target_id, &participant.path));
    }
    let path = super::super::status::validated_participant_path(root, target_id, participant)?;
    let head = backend
        .head(&path)
        .map_err(|error| attach_member(error, target_id, &participant.path))?;
    if head.is_detached || head.branch.as_deref() != Some(participant.target_branch.as_str()) {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "preserve-abort requires HEAD attached to the recorded target branch",
        )
        .with_member(target_id, &participant.path));
    }
    let live_commit = head.commit.ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeDrift,
            "preserve-abort requires a live branch commit",
        )
        .with_member(target_id, &participant.path)
    })?;
    let target_ref = backend
        .read_ref(&path, &format!("refs/heads/{}", participant.target_branch))
        .map_err(|error| attach_member(error, target_id, &participant.path))?;
    if target_ref.as_deref() != Some(live_commit.as_str()) {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "recorded target branch does not point to live HEAD",
        )
        .with_member(target_id, &participant.path));
    }
    if backend
        .repository_state(&path)
        .map_err(|error| attach_member(error, target_id, &participant.path))?
        != GitRepositoryState::Clean
    {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "preserve-abort does not accept an active or foreign Git integration state",
        )
        .with_member(target_id, &participant.path));
    }
    let status = if participant.target_kind == MergeTargetKind::Root
        && record
            .publication
            .as_ref()
            .is_some_and(|publication| publication.candidate.is_some())
    {
        root_user_status(backend, root, record)?
    } else {
        backend
            .status(&path)
            .map_err(|error| attach_member(error, target_id, &participant.path))?
    };
    if status.unresolved > 0 {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "preserve-abort does not accept unresolved index entries",
        )
        .with_member(target_id, &participant.path));
    }
    let anchor = preservation_anchor(record, target_id, participant)?;
    if live_commit != anchor
        && !backend
            .is_ancestor(&path, &anchor, &live_commit)
            .map_err(|error| attach_member(error, target_id, &participant.path))?
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "live HEAD was rewound or diverged from the recorded merge result",
        )
        .with_member(target_id, &participant.path));
    }
    let key = if participant.target_kind == MergeTargetKind::Root {
        "root"
    } else {
        target_id
    };
    let preserve_commit = live_commit != anchor;
    let preserve_worktree = status.staged > 0 || status.unstaged > 0 || status.untracked > 0;
    if participant.target_kind == MergeTargetKind::Root
        && (preserve_commit || preserve_worktree)
        && record.publication.as_ref().is_some_and(|publication| {
            publication.candidate.is_some() && publication.composition_commit.is_none()
        })
    {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "root preservation requires a recorded composition commit; preserve this pre-evidence root work manually before aborting",
        )
        .with_member(target_id, &participant.path));
    }
    Ok(PreservationPlan {
        target_id: target_id.to_owned(),
        path,
        relative_path: participant.path.clone(),
        target_branch: participant.target_branch.clone(),
        anchor: anchor.clone(),
        live_commit: live_commit.clone(),
        backup_ref: format!("refs/gwz/merge/{}/{key}/head", record.merge_id),
        status: status.clone(),
        preserve_commit,
        preserve_worktree,
        root_publication_prefix: if participant.target_kind == MergeTargetKind::Root
            && record
                .publication
                .as_ref()
                .and_then(|publication| publication.candidate.as_ref())
                .is_some()
        {
            super::super::publication::classify_candidate_publication(root, record)?
        } else {
            None
        },
        evidence_owner: EvidenceOwner::Participant,
    })
}

pub(super) fn plan_publication_root<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<PreservationPlan> {
    let publication = record.publication.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root composition evidence has no publication record",
        )
        .with_member("@root", ".")
    })?;
    if publication.root_preservation.len() > 1 {
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root publication has multiple preservation evidence rows",
        )
        .with_member("@root", "."));
    }
    let candidate = publication.candidate.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root composition evidence has no publication candidate",
        )
        .with_member("@root", ".")
    })?;
    let anchor = publication.composition_commit.clone().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root publication has no composition commit",
        )
        .with_member("@root", ".")
    })?;
    let head = backend
        .head(root)
        .map_err(|error| attach_member(error, "@root", "."))?;
    if head.is_detached || head.branch.as_deref() != Some(candidate.root_branch.as_str()) {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "preserve-abort requires root HEAD attached to the recorded publication branch",
        )
        .with_member("@root", "."));
    }
    let live_commit = head.commit.ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeDrift,
            "preserve-abort requires a live root branch commit",
        )
        .with_member("@root", ".")
    })?;
    let target_ref = backend
        .read_ref(root, &format!("refs/heads/{}", candidate.root_branch))
        .map_err(|error| attach_member(error, "@root", "."))?;
    if target_ref.as_deref() != Some(live_commit.as_str()) {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "recorded root publication branch does not point to live HEAD",
        )
        .with_member("@root", "."));
    }
    if backend
        .repository_state(root)
        .map_err(|error| attach_member(error, "@root", "."))?
        != GitRepositoryState::Clean
    {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "preserve-abort does not accept an active or foreign root Git integration state",
        )
        .with_member("@root", "."));
    }
    let status = root_user_status(backend, root, record)?;
    if status.unresolved > 0 {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "preserve-abort does not accept unresolved root index entries",
        )
        .with_member("@root", "."));
    }
    if live_commit != anchor
        && !backend
            .is_ancestor(root, &anchor, &live_commit)
            .map_err(|error| attach_member(error, "@root", "."))?
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "root HEAD was rewound or diverged from the recorded composition commit",
        )
        .with_member("@root", "."));
    }
    let preserve_worktree = status.staged > 0 || status.unstaged > 0 || status.untracked > 0;
    if preserve_worktree && record.baseline.root_head.is_none() {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "automatic root worktree preservation is unavailable when the merge composition created the root's first commit; preserve root work manually before aborting",
        )
        .with_member("@root", "."));
    }
    Ok(PreservationPlan {
        target_id: "@root".to_owned(),
        path: root.to_path_buf(),
        relative_path: ".".to_owned(),
        target_branch: candidate.root_branch.clone(),
        anchor: anchor.clone(),
        live_commit: live_commit.clone(),
        backup_ref: format!("refs/gwz/merge/{}/root/head", record.merge_id),
        preserve_commit: live_commit != anchor,
        preserve_worktree,
        status,
        root_publication_prefix: super::super::publication::classify_candidate_publication(
            root, record,
        )?,
        evidence_owner: EvidenceOwner::PublicationRoot,
    })
}

fn root_user_status<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<GitStatus> {
    let mut status = backend
        .status(root)
        .map_err(|error| attach_member(error, "@root", "."))?;
    let excluded = super::super::publication::candidate_files(record)?
        .into_iter()
        .map(|file| file.path)
        .collect::<std::collections::BTreeSet<_>>();
    status.files.retain(|file| !excluded.contains(&file.path));
    status.staged = status
        .files
        .iter()
        .filter(|file| file.index_status != " " && file.index_status != "?")
        .count();
    status.unstaged = status
        .files
        .iter()
        .filter(|file| file.worktree_status != " " && file.worktree_status != "?")
        .count();
    status.untracked = status
        .files
        .iter()
        .filter(|file| file.index_status == "?" || file.worktree_status == "?")
        .count();
    status.unresolved = status
        .files
        .iter()
        .filter(|file| file.index_status == "U" || file.worktree_status == "U")
        .count();
    status.is_dirty =
        status.staged > 0 || status.unstaged > 0 || status.untracked > 0 || status.unresolved > 0;
    Ok(status)
}

fn preservation_anchor(
    record: &MergeOperationRecord,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<String> {
    if participant.target_kind == MergeTargetKind::Root
        && let Some(commit) = record
            .publication
            .as_ref()
            .and_then(|publication| publication.composition_commit.as_ref())
    {
        return Ok(commit.clone());
    }
    participant.resulting_commit.clone().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "successful participant has no resulting commit",
        )
        .with_member(target_id, &participant.path)
    })
}

pub(super) fn preflight_artifacts<B: GitBackend>(
    backend: &B,
    record: &MergeOperationRecord,
    plans: &mut [PreservationPlan],
) -> ModelResult<()> {
    let stash_id = format!("stash_{}", record.merge_id);
    let stash_prefix = format!("gwz:{stash_id}:");
    for plan in plans {
        let evidence = existing_evidence(record, plan)?;
        if let Some(evidence) = evidence {
            validate_evidence_shape(plan, evidence, &stash_id)?;
        }
        let observed_ref = member_result(backend.read_ref(&plan.path, &plan.backup_ref), plan)?;
        match (
            evidence.and_then(|value| value.backup_commit.as_deref()),
            observed_ref.as_deref(),
        ) {
            (Some(expected), Some(actual)) if expected == actual => plan.preserve_commit = true,
            (Some(_), Some(_)) => return Err(drift(plan, "preservation ref target changed")),
            (Some(_), None) => return Err(drift(plan, "recorded preservation ref is missing")),
            (None, Some(actual))
                if record.state == OperationState::Preserving && actual == plan.live_commit =>
            {
                plan.preserve_commit = true;
            }
            (None, Some(_)) => {
                return Err(drift(
                    plan,
                    "unrecorded preservation ref collides with this merge",
                ));
            }
            (None, None) => {}
        }
        let matching = member_result(backend.stash_list(&plan.path), plan)?
            .into_iter()
            .filter(|entry| entry.message.contains(&stash_prefix))
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(drift(
                plan,
                "multiple native stashes use this merge preservation id",
            ));
        }
        match (
            evidence.and_then(|value| value.stash_object_id.as_deref()),
            matching.first(),
        ) {
            (Some(expected), Some(actual)) if expected == actual.object_id => {
                plan.preserve_worktree = true;
            }
            (Some(_), Some(_)) => return Err(drift(plan, "preservation stash identity changed")),
            (Some(_), None) => return Err(drift(plan, "recorded preservation stash is missing")),
            (None, Some(_))
                if record.state == OperationState::Preserving && !plan.status.is_dirty =>
            {
                plan.preserve_worktree = true;
            }
            (None, Some(_)) => {
                return Err(drift(
                    plan,
                    "repository contains new work after an unrecorded preservation stash",
                ));
            }
            (None, None) => {}
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(in crate::workspace_ops::merge) struct V1PreservationOwnerPlan {
    pub(in crate::workspace_ops::merge) owner:
        crate::workspace_ops::merge::model::v1::PreservationOwnerV1,
    pub(in crate::workspace_ops::merge) target_id: String,
    pub(in crate::workspace_ops::merge) path: PathBuf,
    pub(in crate::workspace_ops::merge) relative_path: String,
    pub(in crate::workspace_ops::merge) branch: String,
    pub(in crate::workspace_ops::merge) anchor: String,
    pub(in crate::workspace_ops::merge) live_commit: String,
    pub(in crate::workspace_ops::merge) protected_commit: String,
    pub(in crate::workspace_ops::merge) backup_ref: String,
    pub(in crate::workspace_ops::merge) root_handoff:
        Option<crate::workspace_ops::merge::model::v1::PreservationPublicationCandidateV1>,
}

pub(in crate::workspace_ops::merge) fn v1_preservation_owners<
    B: crate::git::MergeAuthorityBackend,
>(
    backend: &B,
    root: &Path,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
) -> ModelResult<Vec<V1PreservationOwnerPlan>> {
    use crate::git::GitDirectRefObservation;
    use crate::workspace_ops::merge::model::v1::{
        PendingPreservationActionV1, PreservationOwnerV1, PreservationStashPhaseV1 as S,
    };

    let mut owners = Vec::new();
    for target_id in &record.selected_targets {
        let participant = record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        if !super::super::participant_semantics::result::is_integrated_result(participant.state) {
            continue;
        }
        let owner = PreservationOwnerV1::Participant {
            member_id: target_id.clone(),
        };
        let anchor = if target_id == "@root" {
            record
                .publication
                .as_ref()
                .and_then(|publication| publication.composition_commit.clone())
                .or_else(|| participant.resulting_commit.clone())
        } else {
            participant.resulting_commit.clone()
        }
        .ok_or_else(|| {
            owner_error(
                &owner,
                participant.path.as_str(),
                "owner has no merge result",
            )
        })?;
        owners.push(v1_owner_plan(
            backend,
            root,
            record,
            owner,
            participant.path.clone(),
            participant.target_branch.clone(),
            anchor,
        )?);
    }

    let publication_root = record
        .publication
        .as_ref()
        .and_then(|publication| publication.composition_commit.as_ref())
        .is_some()
        && !record.participants.contains_key("@root");
    if publication_root {
        let Some(publication) = record.publication.as_ref() else {
            return Err(owner_error(
                &PreservationOwnerV1::PublicationRoot,
                ".",
                "root composition owner has no publication progress",
            ));
        };
        let Some(composition_commit) = publication.composition_commit.clone() else {
            return Err(owner_error(
                &PreservationOwnerV1::PublicationRoot,
                ".",
                "root composition owner has no composition commit",
            ));
        };
        let candidate = publication.candidate.as_ref().ok_or_else(|| {
            owner_error(
                &PreservationOwnerV1::PublicationRoot,
                ".",
                "root composition evidence has no publication candidate",
            )
        })?;
        owners.push(v1_owner_plan(
            backend,
            root,
            record,
            PreservationOwnerV1::PublicationRoot,
            ".".into(),
            candidate.root_branch.clone(),
            composition_commit,
        )?);
    }

    for plan in &owners {
        let evidence = v1_owner_evidence(record, &plan.owner)?;
        let observed = backend
            .observe_direct_ref(&plan.path, &plan.backup_ref)
            .map_err(|error| attach_v1(error, plan))?;
        let pending_target = match record.pending_preservation.as_ref() {
            Some(PendingPreservationActionV1::BackupRef {
                owner,
                name,
                target_commit,
            }) if owner == &plan.owner && name == &plan.backup_ref => Some(target_commit.as_str()),
            _ => None,
        };
        match (
            evidence.and_then(|row| row.backup_commit.as_deref()),
            pending_target,
            observed,
        ) {
            (Some(expected), _, GitDirectRefObservation::Direct { target })
                if target == expected => {}
            (None, Some(_), GitDirectRefObservation::Absent) => {}
            (None, Some(expected), GitDirectRefObservation::Direct { target })
                if target == expected => {}
            (None, None, GitDirectRefObservation::Absent) => {}
            _ => {
                return Err(owner_error(
                    &plan.owner,
                    &plan.relative_path,
                    "preservation backup ref is missing, foreign, or attached to the wrong commit",
                ));
            }
        }

        let stashes =
            crate::git::observe_preservation_stashes_read_only(&plan.path, &record.merge_id)
                .map_err(|error| attach_v1(error, plan))?;
        let expected_stash = evidence.and_then(|row| row.stash_object_id.as_deref());
        let pending_stash = match record.pending_preservation.as_ref() {
            Some(action @ PendingPreservationActionV1::Stash { owner, .. })
                if owner == &plan.owner =>
            {
                Some(action)
            }
            _ => None,
        };
        match (expected_stash, stashes.as_slice(), pending_stash) {
            (Some(expected), [actual], pending)
                if actual.object_id == expected
                    && pending.is_none_or(|action| v1_stash_matches_action(actual, action)) => {}
            (None, [], None) => {}
            (
                None,
                [],
                Some(PendingPreservationActionV1::Stash {
                    phase:
                        S::NormalizeParent
                        | S::NormalizeMarker
                        | S::NormalizeLock
                        | S::NormalizeIndex
                        | S::CreateStash,
                    ..
                }),
            ) => {}
            (
                None,
                [actual],
                Some(
                    action @ PendingPreservationActionV1::Stash {
                        phase: S::CreateStash,
                        ..
                    },
                ),
            ) if v1_stash_matches_action(actual, action) => {}
            _ => {
                return Err(owner_error(
                    &plan.owner,
                    &plan.relative_path,
                    "native preservation stash is missing, duplicated, or has a foreign identity",
                ));
            }
        }
    }
    Ok(owners)
}

fn v1_stash_matches_action(
    actual: &crate::git::GitPreservationStashEvidence,
    action: &crate::workspace_ops::merge::model::v1::PendingPreservationActionV1,
) -> bool {
    let crate::workspace_ops::merge::model::v1::PendingPreservationActionV1::Stash {
        stash_object_id,
        message,
        head_commit,
        preimage_sha256,
        ..
    } = action
    else {
        return false;
    };
    stash_object_id
        .as_ref()
        .is_none_or(|expected| expected.digest_hex == actual.object_id)
        && actual.message == *message
        && actual.head_commit == *head_commit
        && actual.image.preimage_sha256 == *preimage_sha256
}

fn v1_owner_plan<B: crate::git::MergeAuthorityBackend>(
    backend: &B,
    root: &Path,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    owner: crate::workspace_ops::merge::model::v1::PreservationOwnerV1,
    relative_path: String,
    branch: String,
    anchor: String,
) -> ModelResult<V1PreservationOwnerPlan> {
    let target_id = v1_owner_id(&owner).to_owned();
    let path = if target_id == "@root" {
        root.to_path_buf()
    } else {
        let participant = record.participants.get(&target_id).ok_or_else(|| {
            owner_error(
                &owner,
                &relative_path,
                "preservation participant disappeared",
            )
        })?;
        super::super::status::validated_participant_path(root, &target_id, participant)?
    };
    if backend
        .repository_state(&path)
        .map_err(|error| attach_v1_parts(error, &target_id, &relative_path))?
        != GitRepositoryState::Clean
    {
        return Err(owner_error(
            &owner,
            &relative_path,
            "preservation requires a clean native Git integration state",
        ));
    }
    let head = backend
        .head(&path)
        .map_err(|error| attach_v1_parts(error, &target_id, &relative_path))?;
    let live_commit = head.commit.ok_or_else(|| {
        owner_error(
            &owner,
            &relative_path,
            "preservation owner has no attached commit",
        )
    })?;
    if head.is_detached || head.branch.as_deref() != Some(branch.as_str()) {
        return Err(owner_error(
            &owner,
            &relative_path,
            "preservation owner is not attached to its recorded branch",
        ));
    }
    if backend
        .read_ref(&path, &format!("refs/heads/{branch}"))
        .map_err(|error| attach_v1_parts(error, &target_id, &relative_path))?
        .as_deref()
        != Some(live_commit.as_str())
    {
        return Err(owner_error(
            &owner,
            &relative_path,
            "preservation branch does not point to live HEAD",
        ));
    }
    let evidence = v1_owner_evidence(record, &owner)?;
    let protected_commit = evidence
        .and_then(|row| row.backup_commit.clone())
        .unwrap_or_else(|| live_commit.clone());
    if live_commit != anchor && live_commit != protected_commit {
        return Err(owner_error(
            &owner,
            &relative_path,
            "preservation owner is neither at its protected commit nor its rollback anchor",
        ));
    }
    if protected_commit != anchor
        && !backend
            .is_ancestor(&path, &anchor, &protected_commit)
            .map_err(|error| attach_v1_parts(error, &target_id, &relative_path))?
    {
        return Err(owner_error(
            &owner,
            &relative_path,
            "preservation owner was rewound or diverged from its immutable anchor",
        ));
    }
    let root_handoff = matches!(owner, crate::workspace_ops::merge::model::v1::PreservationOwnerV1::PublicationRoot)
        .then(|| record.preservation_publication_handoff.and_then(|value| value.candidate()))
        .flatten()
        .or_else(|| {
            matches!(&owner, crate::workspace_ops::merge::model::v1::PreservationOwnerV1::Participant { member_id } if member_id == "@root")
                .then(|| record.preservation_publication_handoff.and_then(|value| value.candidate()))
                .flatten()
        });
    let backup_ref = format!(
        "refs/gwz/merge/{}/{}/head",
        record.merge_id,
        if target_id == "@root" {
            "root"
        } else {
            target_id.as_str()
        }
    );
    Ok(V1PreservationOwnerPlan {
        owner,
        target_id,
        path,
        relative_path,
        branch,
        anchor,
        live_commit,
        protected_commit,
        backup_ref,
        root_handoff,
    })
}

pub(in crate::workspace_ops::merge) fn v1_owner_evidence<'a>(
    record: &'a crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    owner: &crate::workspace_ops::merge::model::v1::PreservationOwnerV1,
) -> ModelResult<Option<&'a PreservationEvidence>> {
    use crate::workspace_ops::merge::model::v1::PreservationOwnerV1;
    let rows = match owner {
        PreservationOwnerV1::Participant { member_id } => record
            .participants
            .get(member_id)
            .ok_or_else(|| owner_error(owner, member_id, "preservation participant is missing"))?
            .preservation
            .as_slice(),
        PreservationOwnerV1::PublicationRoot => record
            .publication
            .as_ref()
            .ok_or_else(|| owner_error(owner, ".", "publication progress is missing"))?
            .root_preservation
            .as_slice(),
    };
    match rows {
        [] => Ok(None),
        [row] => Ok(Some(row)),
        _ => Err(owner_error(
            owner,
            if v1_owner_id(owner) == "@root" {
                "."
            } else {
                v1_owner_id(owner)
            },
            "preservation owner has multiple evidence rows",
        )),
    }
}

pub(in crate::workspace_ops::merge) fn v1_owner_id(
    owner: &crate::workspace_ops::merge::model::v1::PreservationOwnerV1,
) -> &str {
    match owner {
        crate::workspace_ops::merge::model::v1::PreservationOwnerV1::Participant { member_id } => {
            member_id
        }
        crate::workspace_ops::merge::model::v1::PreservationOwnerV1::PublicationRoot => "@root",
    }
}

fn attach_v1(error: ModelError, plan: &V1PreservationOwnerPlan) -> ModelError {
    attach_v1_parts(error, &plan.target_id, &plan.relative_path)
}

fn attach_v1_parts(mut error: ModelError, target_id: &str, relative_path: &str) -> ModelError {
    if error.member_id.is_none() {
        error.member_id = Some(target_id.to_owned());
        error.member_path = Some(relative_path.to_owned());
    }
    error
}

fn owner_error(
    owner: &crate::workspace_ops::merge::model::v1::PreservationOwnerV1,
    relative_path: &str,
    detail: impl Into<String>,
) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into()).with_member(
        v1_owner_id(owner),
        if v1_owner_id(owner) == "@root" {
            "."
        } else {
            relative_path
        },
    )
}
