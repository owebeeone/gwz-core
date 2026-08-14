//! Test-gated v1 preservation-bundle checked adapter.

#![forbid(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};

use crate::checked_artifact::entry::MergeArtifactTransition;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::plan::V1PreservationOwnerPlan;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum V1BundleObservation {
    Before,
    After,
    Ambiguous,
}

pub(in crate::workspace_ops::merge) fn v1_bundle_observation<B: GitBackend>(
    _backend: &B,
    root: &Path,
    record: &super::super::model::v1::MergeOperationRecordV1,
    plans: &[V1PreservationOwnerPlan],
    owner: &super::super::model::v1::PreservationOwnerV1,
) -> ModelResult<V1BundleObservation> {
    let index = owner_index(plans, owner)?;
    let before = expected_bundle(record, &plans[..index])?;
    let after = expected_bundle(record, &plans[..=index])?;
    let before_bytes = (!before.members.is_empty())
        .then(|| before.to_yaml().map(String::into_bytes))
        .transpose()?;
    let after_bytes = after.to_yaml()?.into_bytes();
    Ok(
        match crate::checked_artifact::entry::classify_merge_preservation_bundle(
            root,
            &bundle_relative(&after.stash_id),
            before_bytes.as_deref(),
            &after_bytes,
        )? {
            MergeArtifactTransition::Before | MergeArtifactTransition::Recoverable => {
                V1BundleObservation::Before
            }
            MergeArtifactTransition::After => V1BundleObservation::After,
            MergeArtifactTransition::Ambiguous => V1BundleObservation::Ambiguous,
        },
    )
}

pub(in crate::workspace_ops::merge) fn v1_bundle_cursor_is_exact<B: GitBackend>(
    _backend: &B,
    root: &Path,
    record: &super::super::model::v1::MergeOperationRecordV1,
    plans: &[V1PreservationOwnerPlan],
) -> ModelResult<bool> {
    let expected = expected_bundle(record, plans)?;
    let bytes = (!expected.members.is_empty())
        .then(|| expected.to_yaml().map(String::into_bytes))
        .transpose()?;
    crate::checked_artifact::entry::observe_merge_preservation_bundle(
        root,
        &bundle_relative(&expected.stash_id),
        bytes.as_deref(),
    )
}

pub(in crate::workspace_ops::merge) fn v1_write_bundle_checked<B: GitBackend>(
    _backend: &B,
    root: &Path,
    record: &super::super::model::v1::MergeOperationRecordV1,
    plans: &[V1PreservationOwnerPlan],
    owner: &super::super::model::v1::PreservationOwnerV1,
) -> ModelResult<()> {
    let index = owner_index(plans, owner)?;
    let before = expected_bundle(record, &plans[..index])?;
    let after = expected_bundle(record, &plans[..=index])?;
    let relative = bundle_relative(&after.stash_id);
    let before = (!before.members.is_empty())
        .then(|| before.to_yaml().map(String::into_bytes))
        .transpose()?;
    let after = after.to_yaml()?.into_bytes();
    match crate::checked_artifact::entry::classify_merge_preservation_bundle(
        root,
        &relative,
        before.as_deref(),
        &after,
    )? {
        MergeArtifactTransition::After => return Ok(()),
        MergeArtifactTransition::Before | MergeArtifactTransition::Recoverable => {}
        MergeArtifactTransition::Ambiguous => {
            return Err(ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                "preservation bundle is neither the exact prior prefix nor the exact completed prefix",
            ));
        }
    }
    crate::checked_artifact::entry::replace_merge_preservation_bundle(
        root,
        &relative,
        before.as_deref(),
        &after,
    )?;
    (crate::checked_artifact::entry::classify_merge_preservation_bundle(
        root,
        &relative,
        before.as_deref(),
        &after,
    )? == MergeArtifactTransition::After)
        .then_some(())
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                "preservation bundle failed exact post-write verification",
            )
        })
}

fn owner_index(
    plans: &[V1PreservationOwnerPlan],
    owner: &super::super::model::v1::PreservationOwnerV1,
) -> ModelResult<usize> {
    plans
        .iter()
        .position(|plan| &plan.owner == owner)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                "bundle owner is outside the preservation cursor",
            )
        })
}

fn bundle_relative(stash_id: &str) -> PathBuf {
    PathBuf::from(crate::stash::STASH_BUNDLE_DIR).join(format!("{stash_id}.yaml"))
}

fn expected_bundle(
    record: &super::super::model::v1::MergeOperationRecordV1,
    plans: &[V1PreservationOwnerPlan],
) -> ModelResult<crate::stash::StashBundle> {
    use crate::stash::{
        STASH_BUNDLE_SCHEMA, StashBundle, StashBundleMember, StashDirtySummary, StashParticipation,
        StashPushLifecycle, StashRestoreState,
    };

    let stash_id = format!("stash_{}", record.merge_id);
    let mut selected_members = Vec::new();
    let mut members = Vec::new();
    for plan in plans {
        let Some(evidence) = owner_evidence(record, &plan.owner)? else {
            continue;
        };
        let Some(expected_oid) = evidence.stash_object_id.as_deref() else {
            continue;
        };
        let stashes =
            crate::git::observe_preservation_stashes_read_only(&plan.path, &record.merge_id)
                .map_err(|error| attach_owner(error, plan))?;
        let [stash] = stashes.as_slice() else {
            return Err(owner_error(
                plan,
                "bundle source stash is missing or duplicated",
            ));
        };
        if stash.object_id != expected_oid
            || evidence.stash_id.as_deref() != Some(stash_id.as_str())
            || stash.head_commit != plan.protected_commit
            || stash.message != format!("gwz:{stash_id}: merge preservation")
            || stash.image.dirty == crate::git::GitPreservationDirtySummary::default()
        {
            return Err(owner_error(
                plan,
                "bundle source stash does not match durable preservation evidence",
            ));
        }
        selected_members.push(plan.target_id.clone());
        members.push(StashBundleMember {
            member_id: plan.target_id.clone(),
            path: plan.relative_path.clone(),
            participation: StashParticipation::Stashed,
            push_lifecycle: StashPushLifecycle::Saved,
            restore_state: StashRestoreState::Pending,
            branch_before: Some(plan.branch.clone()),
            head_before: Some(stash.head_commit.clone()),
            full_stash_message: stash.message.clone(),
            dirty_summary: StashDirtySummary {
                staged: stash.image.dirty.staged,
                unstaged: stash.image.dirty.unstaged,
                untracked: stash.image.dirty.untracked,
                ignored: false,
            },
            native_stash_object_id: Some(stash.object_id.clone()),
            native_stash_display_ref: None,
            error: None,
        });
    }
    members.sort_by(|left, right| left.member_id.cmp(&right.member_id));
    selected_members.sort();
    Ok(StashBundle {
        schema: STASH_BUNDLE_SCHEMA.into(),
        workspace_id: record.workspace_id.clone(),
        stash_id,
        created_at: record.created_at.clone(),
        message_suffix: "merge preservation".into(),
        include_untracked: true,
        include_ignored: false,
        selected_members,
        members,
        warnings: Vec::new(),
        drift: Vec::new(),
    })
}

fn owner_evidence<'a>(
    record: &'a super::super::model::v1::MergeOperationRecordV1,
    owner: &super::super::model::v1::PreservationOwnerV1,
) -> ModelResult<Option<&'a super::super::PreservationEvidence>> {
    use super::super::model::v1::PreservationOwnerV1;

    let rows = match owner {
        PreservationOwnerV1::Participant { member_id } => record
            .participants
            .get(member_id)
            .ok_or_else(|| {
                owner_parts_error(owner, member_id, "preservation participant is missing")
            })?
            .preservation
            .as_slice(),
        PreservationOwnerV1::PublicationRoot => record
            .publication
            .as_ref()
            .ok_or_else(|| owner_parts_error(owner, ".", "publication progress is missing"))?
            .root_preservation
            .as_slice(),
    };
    match rows {
        [] => Ok(None),
        [row] => Ok(Some(row)),
        _ => Err(owner_parts_error(
            owner,
            if owner_id(owner) == "@root" {
                "."
            } else {
                owner_id(owner)
            },
            "preservation owner has multiple evidence rows",
        )),
    }
}

fn attach_owner(mut error: ModelError, plan: &V1PreservationOwnerPlan) -> ModelError {
    if error.member_id.is_none() {
        error.member_id = Some(plan.target_id.clone());
        error.member_path = Some(plan.relative_path.clone());
    }
    error
}

fn owner_error(plan: &V1PreservationOwnerPlan, detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
        .with_member(&plan.target_id, &plan.relative_path)
}

fn owner_parts_error(
    owner: &super::super::model::v1::PreservationOwnerV1,
    relative_path: &str,
    detail: impl Into<String>,
) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into()).with_member(
        owner_id(owner),
        if owner_id(owner) == "@root" {
            "."
        } else {
            relative_path
        },
    )
}

fn owner_id(owner: &super::super::model::v1::PreservationOwnerV1) -> &str {
    match owner {
        super::super::model::v1::PreservationOwnerV1::Participant { member_id } => member_id,
        super::super::model::v1::PreservationOwnerV1::PublicationRoot => "@root",
    }
}
