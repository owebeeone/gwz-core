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
///
/// GwzOpenDecisions D3: which rows are listed is the LOCK's decision (what the workspace was told
/// to have), and `materialized` is the FILESYSTEM's answer (what is actually there). They disagree
/// after `gwz clone` quietly skips a private member whose access was refused: the directory is
/// removed and no lock is rewritten, so the lock keeps claiming the member. Such a row is still
/// listed — hiding the discrepancy is what made it invisible — but reports `materialized: false`
/// and a `note` saying why (`private, skipped`).
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
