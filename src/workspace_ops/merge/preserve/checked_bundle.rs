//! Test-gated v1 preservation-bundle checked adapter.

use std::path::{Path, PathBuf};

use crate::checked_artifact::{CheckedArtifact, CheckedArtifactFact, CheckedArtifactTransition};
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
    let before = before_bytes.map_or(CheckedArtifactFact::Missing, CheckedArtifactFact::Bytes);
    Ok(
        match bundle_artifact(root, &after.stash_id)?.classify_replace(&before, &after_bytes)? {
            CheckedArtifactTransition::Before | CheckedArtifactTransition::Recoverable => {
                V1BundleObservation::Before
            }
            CheckedArtifactTransition::After => V1BundleObservation::After,
            CheckedArtifactTransition::Ambiguous => V1BundleObservation::Ambiguous,
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
    let observed = bundle_artifact(root, &expected.stash_id)?.observe_durable()?;
    if expected.members.is_empty() {
        return Ok(observed == CheckedArtifactFact::Missing);
    }
    Ok(observed == CheckedArtifactFact::Bytes(expected.to_yaml()?.into_bytes()))
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
    let artifact = bundle_artifact(root, &after.stash_id)?;
    let before = if before.members.is_empty() {
        CheckedArtifactFact::Missing
    } else {
        CheckedArtifactFact::Bytes(before.to_yaml()?.into_bytes())
    };
    let after = after.to_yaml()?.into_bytes();
    match artifact.classify_replace(&before, &after)? {
        CheckedArtifactTransition::After => return Ok(()),
        CheckedArtifactTransition::Before | CheckedArtifactTransition::Recoverable => {}
        CheckedArtifactTransition::Ambiguous => {
            return Err(ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                "preservation bundle is neither the exact prior prefix nor the exact completed prefix",
            ));
        }
    }
    artifact.replace_exact(&before, &after)?;
    (artifact.classify_replace(&before, &after)? == CheckedArtifactTransition::After)
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

fn bundle_artifact(root: &Path, stash_id: &str) -> ModelResult<CheckedArtifact> {
    let relative = PathBuf::from(crate::stash::STASH_BUNDLE_DIR).join(format!("{stash_id}.yaml"));
    let artifact =
        crate::checked_artifact::entry::acquire_merge_preservation_bundle(root, &relative)?;
    if !artifact.parent_is_canonical()? {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation bundle parent hierarchy is missing or noncanonical",
        ));
    }
    Ok(artifact)
}
