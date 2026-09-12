use std::path::Path;

use crate::artifact::{LockArtifact, ManifestArtifact, ManifestMember, ResolvedMemberArtifact};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::*;

pub(crate) struct MaterializePlan {
    pub(crate) member_id: String,
    pub(crate) state: ResolvedMemberArtifact,
    pub(crate) clone_url: Option<String>,
    /// How `clone_url` was chosen, for a member this operation clones.
    pub(crate) url_resolution: Option<crate::MemberUrlResolution>,
    pub(crate) response: crate::MemberResponse,
}

pub(crate) fn materialize_preflight<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    target_lock: &LockArtifact,
    selected: &[String],
    destructive_allowed: bool,
    scheme: EffectiveUrlScheme,
) -> ModelResult<Vec<MaterializePlan>>
where
    B: GitBackend,
{
    let mut plans = Vec::with_capacity(selected.len());
    for member_id in selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let state = target_lock.members.get(member_id).cloned().ok_or_else(|| {
            ModelError::new(
                ErrorCode::LockNotFound,
                format!("target state missing for member '{member_id}'"),
            )
        })?;
        let member_root = root.join(&state.path);
        let is_repo = member_root.exists() && backend.is_repository(&member_root)?;
        let (clone_url, url_resolution) = if is_repo {
            let status = backend.status(&member_root)?;
            if status.is_dirty && !destructive_allowed {
                return Err(ModelError::new(
                    ErrorCode::DirtyMember,
                    format!("member '{member_id}' has uncommitted changes"),
                ));
            }
            (None, None)
        } else {
            // The manifest URL is a preference: derive the requested form for a
            // known host, refuse what cannot be derived, pass other hosts through.
            let manifest_url = first_remote_url(member)?;
            let resolution =
                crate::git::derive(&manifest_url, scheme.scheme).map_err(|refusal| {
                    url_scheme_refusal_error(
                        &refusal,
                        Some((member.id.as_str(), state.path.as_str())),
                    )
                })?;
            let record = protocol_url_resolution(&resolution, scheme.source);
            (Some(resolution.effective_url), Some(record))
        };
        let action = if clone_url.is_some() {
            crate::PlannedAction::Clone
        } else if state.commit.is_some() {
            crate::PlannedAction::Checkout
        } else {
            crate::PlannedAction::Noop
        };
        plans.push(MaterializePlan {
            member_id: member_id.clone(),
            state: state.clone(),
            clone_url,
            url_resolution: url_resolution.clone(),
            response: crate::MemberResponse {
                member_id: member_id.clone(),
                member_path: state.path.clone(),
                source_kind: crate::SourceKind::Git,
                status: crate::MemberStatus::Planned,
                error: None,
                planned: Some(crate::PlannedChange {
                    action,
                    from_ref: None,
                    to_ref: state.commit.clone(),
                    message: None,
                }),
                state: Some(protocol_state(member, &state)),
                git_status: None,
                target_kind: Some(crate::TargetKind::Member),
                lock_match: Some(crate::LockMatch::Differs),
                lock_difference_reasons: None,
                url_resolution,
            },
        });
    }
    Ok(plans)
}

pub(crate) fn first_remote_url(member: &ManifestMember) -> ModelResult<String> {
    member
        .remotes
        .iter()
        .find(|remote| remote.fetch)
        .map(|remote| remote.url.clone())
        .ok_or_else(|| ModelError::new(ErrorCode::MissingRemote, "member has no fetch remote"))
}
