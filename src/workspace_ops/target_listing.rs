//! Shared target listing for reporting and driver-owned execution. No command
//! delegates to another command; both use the same policy and projection.
use super::*;
use crate::artifact;
use crate::model::ModelResult;
use std::path::Path;

pub(super) fn target_entries(
    start: &Path,
    meta: &crate::RequestMeta,
    include_unmaterialized: bool,
    action: crate::ActionKind,
) -> ModelResult<Vec<crate::MemberEntry>> {
    let root = resolve_workspace_root(start, meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, meta.workspace.as_ref())?;

    // Read the lock if present; its absence just means nothing is materialized yet.
    let lock = if root.join(artifact::LOCK_PATH).exists() {
        Some(artifact::read_lock(&root)?)
    } else {
        None
    };

    // Manifest-tolerant selection (no lock-presence requirement, unlike lock-dependent mutation planning).
    let selected = resolve_action_targets(&manifest, meta.selection.as_ref(), action)?;
    let members = selected
        .into_iter()
        .filter_map(|member| {
            let SelectedTarget::Member(member) = member else {
                return Some(crate::MemberEntry {
                    id: "@root".to_owned(),
                    path: ".".to_owned(),
                    abspath: root.to_string_lossy().into_owned(),
                    materialized: true,
                    target_kind: Some(crate::TargetKind::Root),
                });
            };
            let materialized = lock
                .as_ref()
                .and_then(|lock| lock.members.get(&member.id))
                .and_then(|entry| entry.materialized)
                == Some(true);
            (materialized || include_unmaterialized).then(|| crate::MemberEntry {
                id: member.id.clone(),
                path: member.path.clone(),
                abspath: root.join(&member.path).to_string_lossy().into_owned(),
                materialized,
                target_kind: Some(crate::TargetKind::Member),
            })
        })
        .collect();

    Ok(members)
}

/// Resolve execution targets without launching commands. Reuses the existing
/// target-list wire messages; the response is attributed to Forall.
pub fn resolve_forall_targets(
    start: &Path,
    request: crate::LsRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::LsResponse> {
    let context = crate::operation::OperationContext::from_meta(
        operation_id.into(),
        crate::operation::ActionKind::Forall,
        &request.meta,
    )?;
    let members = target_entries(
        start,
        &request.meta,
        request.include_unmaterialized.unwrap_or(false),
        crate::ActionKind::Forall,
    )?;
    Ok(crate::LsResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
        members: Some(members),
    })
}
