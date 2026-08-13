//! Test-gated v1 preservation-bundle checked adapter.

use std::path::{Path, PathBuf};

use crate::checked_artifact::entry::MergeArtifactTransition;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::artifacts::expected_bundle;
use super::plan::V1PreservationOwnerPlan;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum V1BundleObservation {
    Before,
    After,
    Ambiguous,
}

pub(in crate::workspace_ops::merge) fn v1_bundle_observation<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &super::super::model::v1::MergeOperationRecordV1,
    plans: &[V1PreservationOwnerPlan],
    owner: &super::super::model::v1::PreservationOwnerV1,
) -> ModelResult<V1BundleObservation> {
    let index = owner_index(plans, owner)?;
    let before = expected_bundle(backend, record, &plans[..index])?;
    let after = expected_bundle(backend, record, &plans[..=index])?;
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
    backend: &B,
    root: &Path,
    record: &super::super::model::v1::MergeOperationRecordV1,
    plans: &[V1PreservationOwnerPlan],
) -> ModelResult<bool> {
    let expected = expected_bundle(backend, record, plans)?;
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
    backend: &B,
    root: &Path,
    record: &super::super::model::v1::MergeOperationRecordV1,
    plans: &[V1PreservationOwnerPlan],
    owner: &super::super::model::v1::PreservationOwnerV1,
) -> ModelResult<()> {
    let index = owner_index(plans, owner)?;
    let before = expected_bundle(backend, record, &plans[..index])?;
    let after = expected_bundle(backend, record, &plans[..=index])?;
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
