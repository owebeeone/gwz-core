use std::path::Path;

use crate::artifact::{ManifestMember, ResolvedMemberArtifact};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::*;

pub(crate) fn protocol_state(
    member: &ManifestMember,
    state: &ResolvedMemberArtifact,
) -> crate::ResolvedMemberState {
    crate::ResolvedMemberState {
        member_id: member.id.clone(),
        path: state.path.clone(),
        source_id: member.source_id.clone(),
        source_kind: crate::SourceKind::Git,
        commit: state.commit.clone(),
        branch: state.branch.clone(),
        detached: state.detached,
        upstream: state.upstream.clone(),
        dirty: state.dirty,
        materialized: state.materialized.unwrap_or(false),
        remotes: member
            .remotes
            .iter()
            .map(|remote| crate::RemoteSpec {
                name: remote.name.clone(),
                url: remote.url.clone(),
                fetch: Some(remote.fetch),
                push: Some(remote.push),
            })
            .collect(),
    }
}

#[allow(
    clippy::needless_update,
    reason = "gwz_transport_candidate adds fields"
)]
pub(crate) fn response_envelope(
    context: crate::operation::OperationContext,
    aggregate_status: crate::AggregateStatus,
    members: Vec<crate::MemberResponse>,
) -> crate::ResponseEnvelope {
    let errors = partial_member_errors(aggregate_status, &members);
    crate::ResponseEnvelope {
        meta: crate::ResponseMeta {
            transport: None,
            request_id: context.request_id,
            schema_version: context.schema_version,
            action: context.action.into(),
            aggregate_status,
            operation_id: Some(context.operation_id),
            message: None,
            attribution: context.attribution.as_ref().map(Into::into),
            ..Default::default()
        },
        members,
        errors,
    }
}

/// The errors a `Partial` result repeats in its top-level `errors` (TR2.3,
/// OD7): a copy of the error of every member entry that failed or was refused,
/// in member order, so a caller that reads only `errors` sees each failure.
/// Every other aggregate repeats none; its member failures stay on the member
/// entries (gwz-cli docs/MachineOutput.md, "Partial results").
fn partial_member_errors(
    aggregate_status: crate::AggregateStatus,
    members: &[crate::MemberResponse],
) -> Vec<crate::GwzError> {
    if aggregate_status != crate::AggregateStatus::Partial {
        return Vec::new();
    }
    members
        .iter()
        .filter(|member| {
            matches!(
                member.status,
                crate::MemberStatus::Failed | crate::MemberStatus::Rejected
            )
        })
        .filter_map(|member| member.error.clone())
        .collect()
}

pub(crate) fn path_slug(path: &str) -> ModelResult<String> {
    let leaf = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| invalid("member path must have a final component"))?;
    let slug = leaf
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_owned();
    if slug.is_empty() {
        Err(invalid("member path does not contain a usable id slug"))
    } else {
        Ok(slug)
    }
}

pub(crate) fn io_error(error: std::io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}
