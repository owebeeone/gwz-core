use std::path::Path;

use crate::model::ModelResult;
use crate::operation::OperationRequest;

use super::*;

/// `gwz ls` — list the workspace's members (`id`, `path`, `abspath`, `materialized`). A read-only
/// op: manifest + lock only, **no git**. Selection rides in `meta.selection` (the global
/// `--member`/`-A`). `include_unmaterialized` surfaces configured-but-uncloned members; by default
/// only materialized members are listed (so `cd $path` can't fail). The filter is uniform — an
/// explicitly-selected member that isn't materialized is simply omitted unless `include_unmaterialized`
/// is set (a non-existent member still errors via selection resolution).
pub fn handle_ls(
    start: &Path,
    request: crate::LsRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::LsResponse> {
    let context = OperationRequest::Ls(request.clone()).context(operation_id.into())?;
    let members = super::target_listing::target_entries(
        start,
        &request.meta,
        request.include_unmaterialized.unwrap_or(false),
        crate::ActionKind::Ls,
    )?;

    Ok(crate::LsResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
        members: Some(members),
    })
}
