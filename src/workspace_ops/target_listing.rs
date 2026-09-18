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
    let root = resolve_request_workspace_root(start, meta)?;
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
                    note: None,
                });
            };
            // What the lock says the workspace was told to have. It decides
            // which rows are listed, exactly as before: a member the lock
            // never materialized is omitted unless asked for.
            let recorded = lock
                .as_ref()
                .and_then(|lock| lock.members.get(&member.id))
                .and_then(|entry| entry.materialized)
                == Some(true);
            let abspath = root.join(&member.path);
            // GwzOpenDecisions D3: what is actually there. `gwz clone` of a
            // workspace may quietly skip a private member whose access is
            // refused -- it removes the directory and rewrites no lock (the
            // clone materializes a lock target), so the lock goes on saying
            // `materialized: true` about a directory that does not exist.
            // The listing answers for the filesystem instead: the absence is
            // the ground truth and needs no record, it stays right if the
            // member is materialized later or removed by hand, and `gwz ls`
            // never has to be believed over `ls`.
            let present = std::fs::symlink_metadata(&abspath).is_ok();
            let note = match (recorded, present, member.private) {
                (true, false, true) => Some("private, skipped".to_owned()),
                (true, false, false) => Some("recorded in the lock but absent on disk".to_owned()),
                _ => None,
            };
            (recorded || include_unmaterialized).then(|| crate::MemberEntry {
                id: member.id.clone(),
                path: member.path.clone(),
                abspath: abspath.to_string_lossy().into_owned(),
                materialized: recorded && present,
                target_kind: Some(crate::TargetKind::Member),
                note,
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
