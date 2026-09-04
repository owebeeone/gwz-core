use std::path::Path;

use super::archive_result::ValidatedArchivedMerge;
use super::store::CheckedV1Store;
use crate::git::MergeAuthorityBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;

pub(super) fn open_status<B: MergeAuthorityBackend>(
    backend: &B,
    store: &CheckedV1Store,
    root: &Path,
    merge_id: &str,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    optimistic_open_status(store, root, merge_id, |current| {
        let mut response = current.record().to_v1_response(context)?;
        let live = super::super::status::observe_status_view(
            backend,
            current.location().root(),
            super::super::status::MergeStatusRecordView::from_v1(current.record()),
        )?;
        for repo in &mut response.repos {
            let observation = live
                .participants
                .get(&repo.target_id)
                .ok_or_else(|| unreadable("selected participant is missing"))?;
            repo.live_commit.clone_from(&observation.live_commit);
            repo.conflict_paths.clone_from(&observation.conflict_paths);
            repo.continue_eligible = Some(observation.continue_eligibility.eligible);
            repo.abort_eligible = Some(observation.abort_eligibility.eligible);
            repo.drift = observation.drift.iter().map(Into::into).collect();
            repo.pending_action = observation.pending_action.as_ref().map(Into::into);
        }
        response.operation_drift = live.operation_drift.iter().map(Into::into).collect();
        Ok(response)
    })
}

fn optimistic_open_status(
    _store: &CheckedV1Store,
    root: &Path,
    merge_id: &str,
    mut snapshot: impl FnMut(&super::checked::StoredV1Record) -> ModelResult<crate::MergeResponse>,
) -> ModelResult<crate::MergeResponse> {
    for attempt in 0..2 {
        let (locations, current) = match acquire_open_status_v1(root, merge_id) {
            Ok(value) => value,
            Err(error) if attempt == 0 => return Err(error),
            Err(_) => return Err(status_contention(merge_id)),
        };
        let response = snapshot(&current)?;
        let unchanged = acquire_open_status_v1(root, merge_id).ok().is_some_and(
            |(reread_locations, reread)| {
                reread_locations == locations && current.same_source_as(&reread)
            },
        );
        if unchanged {
            return Ok(response);
        }
        if attempt == 1 {
            return Err(status_contention(merge_id));
        }
    }
    Err(status_contention(merge_id))
}

fn acquire_open_status_v1(
    root: &Path,
    merge_id: &str,
) -> ModelResult<(
    super::super::record_wire::CanonicalMergeLocations,
    super::checked::StoredV1Record,
)> {
    let locations = super::super::record_wire::acquire_canonical_merge_locations(root, merge_id)?;
    let open = locations
        .open()
        .exact()
        .map(|(path, bytes, _)| {
            super::checked::StoredV1Record::from_open_bytes(root, path.as_path(), bytes.as_slice())
        })
        .transpose()?;
    let _archived = locations
        .archived()
        .exact()
        .map(|(_, bytes, _)| super::super::record_wire::decode_archived(bytes.as_slice(), merge_id))
        .transpose()?;
    match super::super::status::select_canonical_status_source(
        &locations,
        merge_id,
        open.as_ref()
            .map(|current| !current.record().state.is_open()),
    )? {
        super::super::status::CanonicalStatusSource::Open => {
            Ok((locations, open.expect("open source decoded")))
        }
        super::super::status::CanonicalStatusSource::Archived => Err(ModelError::new(
            ErrorCode::OperationNotFound,
            format!("open merge record '{merge_id}' was not found"),
        )),
    }
}

fn status_contention(merge_id: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("checked v1 source '{merge_id}' changed during both status attempts"),
    )
}

/// The terminal answer for a v1 start, continue or abort that reached its
/// archive.
///
/// **M5d charter §4 ("Responses").** The per-repo rows, `participant_counts`,
/// `publication_step`, `preservation` and `operation_drift` are projected from
/// the archived record, so a completed `--no-ff` merge reports the participants
/// it merged instead of `participants: total 0`.
pub(super) fn archived_status(
    merge_id: &str,
    archived: &ValidatedArchivedMerge,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    super::super::response::archived_terminal_response(merge_id, archived.decoded(), context)
}

fn unreadable(detail: &str) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, detail)
}

#[cfg(test)]
#[path = "tests/status.rs"]
mod tests;
