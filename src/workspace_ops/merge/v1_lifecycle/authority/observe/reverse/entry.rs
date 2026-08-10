use super::super::super::*;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::v1_lifecycle::transition::{
    PreparedReverseEntryView, ReverseEntryKind,
};

pub(in crate::workspace_ops::merge::v1_lifecycle) fn prepare_preservation_entry(
    current: &StoredV1Record,
    preview: &PreparedReverseEntryView,
    handoff: VerifiedPublicationHandoff,
    preflight: VerifiedPreservationEntryPreflight,
) -> ModelResult<PreparedPreservationEntry> {
    require_authority(
        current,
        preview,
        ReverseEntryKind::Preservation,
        &handoff,
        preflight.value(),
        preflight.matches(
            current,
            "@operation",
            "preservation_entry_preflight",
            "verified",
        ),
    )?;
    Ok(PreparedPreservationEntry {
        bound: BoundValue::new(
            current,
            "@operation",
            "begin_preservation",
            "preflight",
            EntryPayload {
                origin: RollbackEntryOrigin::Direct,
                authority: handoff.value().clone(),
            },
        )?,
        handoff,
        preflight,
    })
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn prepare_direct_rollback_entry(
    current: &StoredV1Record,
    preview: &PreparedReverseEntryView,
    handoff: VerifiedPublicationHandoff,
    preflight: VerifiedRollbackEntryPreflight,
) -> ModelResult<PreparedRollbackEntry> {
    prepare_rollback_entry(current, preview, handoff, preflight, None)
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn prepare_exhausted_rollback_entry(
    current: &StoredV1Record,
    preview: &PreparedReverseEntryView,
    handoff: VerifiedPublicationHandoff,
    preflight: VerifiedRollbackEntryPreflight,
    exhausted: VerifiedPreservationExhausted,
) -> ModelResult<PreparedRollbackEntry> {
    prepare_rollback_entry(current, preview, handoff, preflight, Some(exhausted))
}

fn prepare_rollback_entry(
    current: &StoredV1Record,
    preview: &PreparedReverseEntryView,
    handoff: VerifiedPublicationHandoff,
    preflight: VerifiedRollbackEntryPreflight,
    preservation_exhausted: Option<VerifiedPreservationExhausted>,
) -> ModelResult<PreparedRollbackEntry> {
    let (kind, origin) = if preservation_exhausted.is_some() {
        (
            ReverseEntryKind::ExhaustedRollback,
            RollbackEntryOrigin::FromPreserving,
        )
    } else {
        (
            ReverseEntryKind::DirectRollback,
            RollbackEntryOrigin::Direct,
        )
    };
    require_authority(
        current,
        preview,
        kind,
        &handoff,
        preflight.value(),
        preflight.matches(
            current,
            "@operation",
            "rollback_entry_preflight",
            "verified",
        ),
    )?;
    if preservation_exhausted.as_ref().is_some_and(|proof| {
        !proof.matches(current, "@operation", "preservation_exhausted", "verified")
    }) {
        return Err(entry_error("preservation exhaustion authority is stale"));
    }
    Ok(PreparedRollbackEntry {
        bound: BoundValue::new(
            current,
            "@operation",
            "begin_rollback",
            "preflight",
            EntryPayload {
                origin,
                authority: handoff.value().clone(),
            },
        )?,
        handoff,
        preflight,
        preservation_exhausted,
    })
}

fn require_authority(
    current: &StoredV1Record,
    preview: &PreparedReverseEntryView,
    expected_kind: ReverseEntryKind,
    handoff: &VerifiedPublicationHandoff,
    preflight: &ReverseEntryAuthorityPayload,
    preflight_matches: bool,
) -> ModelResult<()> {
    let expected = ReverseEntryAuthorityPayload {
        request: preview.request(),
        kind: expected_kind,
        anticipated_model_sha256: preview.anticipated_model_sha256(),
        publication: handoff.value().publication,
    };
    if preview.kind() != expected_kind
        || handoff.value() != &expected
        || preflight != &expected
        || !handoff.matches(current, "@publication", "handoff", "verified")
        || !preflight_matches
    {
        return Err(entry_error(
            "reverse-entry handoff or global preflight does not match the preview",
        ));
    }
    Ok(())
}

fn entry_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail.into())
}
