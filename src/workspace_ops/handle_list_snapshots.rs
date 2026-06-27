use std::path::Path;

use crate::artifact;
use crate::model::ModelResult;
use crate::operation::OperationRequest;

use super::*;

pub fn handle_list_snapshots(
    start: &Path,
    request: crate::ListSnapshotsRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::ListSnapshotsResponse> {
    let context = OperationRequest::ListSnapshots(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;

    let snapshots = artifact::list_snapshots(&root)?
        .into_iter()
        .map(|snapshot| crate::SnapshotInfo {
            name: snapshot.snapshot_id,
            created_at: snapshot.created_at,
            created_by: snapshot.created_by.actor_id,
            members: snapshot.members.len() as i64,
        })
        .collect();

    Ok(crate::ListSnapshotsResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
        snapshots: Some(snapshots),
    })
}
