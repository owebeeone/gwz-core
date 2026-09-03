//! The v1 reverse path's preservation owner plan.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** The v0 engine's participant/root
//! preservation planning left with it; the v1 owner plan the reverse service
//! drives is what remains.

use super::*;
use crate::git::GitRepositoryState;

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
