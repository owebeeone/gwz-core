use super::super::*;
use crate::git::MergeAuthorityBackend;
use crate::workspace_ops::merge::PreservationEvidence;
use crate::workspace_ops::merge::model::v1::{
    GitObjectAlgorithmV1, GitObjectIdV1, PreservationStashPhaseV1 as S,
};
use crate::workspace_ops::merge::preserve::v1_owner_evidence;

type StashEvidence = (
    S,
    (Option<String>, Option<GitObjectIdV1>),
    Option<PreservationEvidence>,
);

pub(super) fn stash_evidence<B: MergeAuthorityBackend>(
    _backend: &B,
    current: &StoredV1Record,
    plan: &V1PreservationOwnerPlan,
    action: &PendingPreservationActionV1,
) -> ModelResult<StashEvidence> {
    let PendingPreservationActionV1::Stash {
        message,
        head_commit,
        preimage_sha256,
        ..
    } = action
    else {
        return Err(preservation_error("stash evidence received another action"));
    };
    let stashes =
        crate::git::observe_preservation_stashes_read_only(&plan.path, &current.record().merge_id)?;
    let [stash] = stashes.as_slice() else {
        return Err(owner_error(
            plan,
            "completed stash has missing or duplicate native evidence",
        ));
    };
    if stash.message != *message
        || stash.head_commit != *head_commit
        || stash.image.preimage_sha256 != *preimage_sha256
    {
        return Err(owner_error(
            plan,
            "completed stash does not match its persisted preimage",
        ));
    }

    let stable_id = format!("stash_{}", current.record().merge_id);
    let object_id = oid(&stash.object_id)?;
    let prior = v1_owner_evidence(current.record(), &plan.owner)?;
    let evidence = PreservationEvidence {
        backup_ref: prior.and_then(|row| row.backup_ref.clone()),
        backup_commit: prior.and_then(|row| row.backup_commit.clone()),
        stash_id: Some(stable_id.clone()),
        stash_object_id: Some(stash.object_id.clone()),
    };
    Ok((
        super::steps::next_stash(S::CreateStash, plan.root_handoff.is_some())?,
        (Some(stable_id), Some(object_id)),
        Some(evidence),
    ))
}

fn oid(value: &str) -> ModelResult<GitObjectIdV1> {
    let algorithm = match value.len() {
        40 => GitObjectAlgorithmV1::Sha1,
        64 => GitObjectAlgorithmV1::Sha256,
        _ => {
            return Err(preservation_error(
                "stash object id has an unsupported width",
            ));
        }
    };
    Ok(GitObjectIdV1 {
        algorithm,
        digest_hex: value.into(),
    })
}
