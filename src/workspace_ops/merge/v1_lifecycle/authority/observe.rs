use std::fs;

use super::super::super::model::v1::rollback_cursor;
use super::*;
use crate::artifact::LOCK_PATH;
use crate::workspace::WORKSPACE_MANIFEST;

mod finalization;
mod forward;

pub(in crate::workspace_ops::merge::v1_lifecycle) use finalization::{
    observe_finalization, verify_finalization_action, verify_finalization_recovery_origin,
};
pub(in crate::workspace_ops::merge::v1_lifecycle) use forward::{
    observe_forward, verify_participant_action,
};

pub(in crate::workspace_ops::merge::v1_lifecycle) fn no_mutation_abort(
    current: &StoredV1Record,
) -> ModelResult<VerifiedNoMutationAbort> {
    let RollbackCursor::NoMutationParticipant { member_id } = rollback_cursor(current.record())
    else {
        return Err(authority_error(
            "rollback cursor does not identify a no-mutation participant",
        ));
    };
    VerifiedNoMutationAbort::issue(
        &AuthorityIssuer::for_observer(current),
        member_id,
        "record_no_mutation_abort",
        "cursor_verified",
        member_id.into(),
    )
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn rollback_exhausted(
    current: &StoredV1Record,
) -> ModelResult<VerifiedRollbackExhausted> {
    let payload = match rollback_cursor(current.record()) {
        RollbackCursor::Complete => RollbackExhaustedPayload {
            selected_root_manifest_sha256: None,
            selected_root_lock_sha256: None,
        },
        RollbackCursor::SelectedRootMetadata => selected_root_baseline(current)?,
        _ => return Err(authority_error("rollback cursor is not complete")),
    };
    VerifiedRollbackExhausted::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "rollback_exhausted",
        "cursor_verified",
        payload,
    )
}

fn selected_root_baseline(current: &StoredV1Record) -> ModelResult<RollbackExhaustedPayload> {
    let record = current.record();
    let expected_manifest =
        record.baseline.manifest_yaml.as_deref().ok_or_else(|| {
            authority_error("selected-root baseline manifest bytes are unavailable")
        })?;
    let expected_lock = record
        .baseline
        .lock_yaml
        .as_deref()
        .ok_or_else(|| authority_error("selected-root baseline lock bytes are unavailable"))?;
    let root = current.location().root();
    let live_manifest = fs::read(root.join(WORKSPACE_MANIFEST))
        .map_err(|error| authority_error(error.to_string()))?;
    let live_lock =
        fs::read(root.join(LOCK_PATH)).map_err(|error| authority_error(error.to_string()))?;
    if live_manifest != expected_manifest.as_bytes() || live_lock != expected_lock.as_bytes() {
        return Err(authority_error(
            "selected-root manifest and lock do not exactly match the operation baseline",
        ));
    }
    Ok(RollbackExhaustedPayload {
        selected_root_manifest_sha256: Some(record.baseline.manifest_sha256.clone()),
        selected_root_lock_sha256: Some(record.baseline.lock_sha256.clone()),
    })
}
