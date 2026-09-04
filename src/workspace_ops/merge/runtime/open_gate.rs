use std::path::Path;

use super::super::{
    MergeTargetKind, OpenRecordEnvelope, classify_open_record,
    discover_open_envelope_before_manifest, discover_open_v1_record, participant_semantics,
};
use crate::model::{ErrorCode, ModelError, ModelResult};

/// Central pre-dispatch guard used by synchronous drivers. Recovery discovery
/// intentionally precedes manifest parsing so an invalid in-flight root merge
/// cannot make the gate disappear.
///
/// A1: discovery is by ENVELOPE, not through the v0 store. `discover_open`
/// reads with the v0-only decoder, so an open v1 record answered
/// `UnsupportedRecordVersion` here and this gate — which owns the only message
/// that names `merge status`, `merge continue` and `merge abort` — was never
/// reached. `merge start` already classified first (`start.rs`); every other
/// gated command now does the same.
pub fn enforce_workspace_open_merge_gate(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
) -> ModelResult<()> {
    if command.gate_decision() == crate::operation::OpenMergeGateDecision::NotGated {
        return Ok(());
    }
    let open = if let Some(root) = workspace.and_then(|workspace| workspace.root.as_ref()) {
        classify_open_record(Path::new(root))?
    } else {
        discover_open_envelope_before_manifest(start)?
    };
    enforce_open_merge_gate_for_envelope(open.as_ref(), command)
}

/// The gate table, applied to an open record's ENVELOPE.
///
/// **M5d (`GwzM5-8M5d-Charter.md` §2, revision 4 L-P3-3).** For an open v1
/// record this is the gate exactly as it has always been. For an open v0
/// envelope the standing remedy — "use merge status, merge continue, or merge
/// abort" — is **suppressed**, because under 0.14 all three of those verbs
/// refuse: the record is a pre-0.14 merge and the whole remedy is the release
/// that can still finish it. Read verbs stay allowed, as they are for v1; the
/// merge verbs are `Allow` here and meet the same sentence at the merge
/// dispatch.
pub(in crate::workspace_ops::merge) fn enforce_open_merge_gate_for_envelope(
    open: Option<&OpenRecordEnvelope>,
    command: crate::operation::OpenMergeCommand,
) -> ModelResult<()> {
    let Some(envelope) = open else {
        return crate::operation::enforce_open_merge_gate(None, command);
    };
    if envelope.is_v1() {
        return crate::operation::enforce_open_merge_gate(Some(&envelope.merge_id), command);
    }
    match command.gate_decision() {
        crate::operation::OpenMergeGateDecision::Block
        | crate::operation::OpenMergeGateDecision::Conditional => {
            Err(super::super::open_record::pre_014_merge_error())
        }
        crate::operation::OpenMergeGateDecision::Allow
        | crate::operation::OpenMergeGateDecision::NotGated => Ok(()),
    }
}

/// The merge start gate's own refusal for an open v1 record.
///
/// Charter §2/§10.3 names `merge/start.rs` as one of the sites that must not
/// print this for a v0 envelope; `handle_start_durable` calls
/// `refuse_if_pre_014` first, so this text is only ever a v1 answer.
pub(in crate::workspace_ops::merge) fn open_merge_start_error(merge_id: &str) -> ModelError {
    ModelError::new(
        ErrorCode::OpenOperation,
        format!("merge '{merge_id}' is open; use merge status, merge continue, or merge abort"),
    )
}

/// `add`'s open-merge scope check.
///
/// **M5d.** A v0 envelope refuses inside `discover_open_v1_record` with the §2
/// sentence, without a body decode, so `add` never routes by a record this
/// binary cannot run.
pub(crate) fn enforce_open_merge_stage_targets(
    root: &Path,
    targets: &[crate::workspace_ops::StageTarget],
) -> ModelResult<()> {
    let Some(open) = discover_open_v1_record(root)? else {
        return Ok(());
    };
    let record = open.view();
    let allowed = record
        .participants()
        .values()
        .filter(|participant| {
            participant_semantics::status::status_policy(participant.state).conflict_role
                == participant_semantics::status::ConflictRole::NativeMerge
        })
        .map(|participant| match participant.target_kind {
            MergeTargetKind::Member => Some(participant.path.as_str()),
            MergeTargetKind::Root => None,
        })
        .collect::<Vec<_>>();
    if targets
        .iter()
        .all(|target| allowed.contains(&target.member_path.as_deref()))
    {
        return Ok(());
    }
    Err(ModelError::new(
        ErrorCode::OpenOperation,
        format!(
            "merge '{}' is open; add may target only its conflicted participants; \
             use merge status to inspect the allowed repositories",
            record.merge_id()
        ),
    ))
}
