//! A1's production start owner for v1 records.
//!
//! Safety review §2.4: "dispatch routes Start(NoFf) and every v1-record
//! continue/resume/abort/status/gc into the v1 lifecycle service
//! (`v1_lifecycle/service.rs:69 run`, today `pub(super)` — gains its
//! merge-visible production entry)". This module is that entry.
//!
//! The v0 lifecycle's start builds a record and then drives it with the v0
//! engine. Here the same accepted plan builds the record at the version the
//! contract-§2 writer floor selected, the checked v1 store creates it
//! durably, and the v1 service — not the v0 engine — takes the record from
//! `Executing`/`Planned` to its next durable stop.
//!
//! Nothing here names the v0 persistence seam: the call-graph gate F-3
//! (`check_checked_artifact_boundaries.py`, load-bearing for judgment call
//! J-1) requires `v1_lifecycle/` to contain no call into v0 merge
//! persistence, and the record is created through the checked v1 store.

use std::path::Path;

use super::authority::{V1LifecycleRequest, V1ResponseDisposition};
use super::forward::ForwardRuntime;
use super::store::CheckedV1Store;
use crate::git::MergeAuthorityBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext};
use crate::workspace_ops::merge::MergeOperationRecord;
use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;

/// Lift one freshly created record onto the v1 body.
///
/// The v1 body is a strict superset of v0's: every field the creation site
/// fills is shared, and each v1-only field (accepted workspace, recovery
/// context, the two pending journals, the preservation/publication handoff)
/// is absent on a record that has not started executing. Migration — not
/// creation — is where those fields are constructed from an existing v0 row
/// (`record_wire::open_v0`), so creation states them absent.
pub(super) fn created_v1_record(record: MergeOperationRecord) -> MergeOperationRecordV1 {
    MergeOperationRecordV1 {
        schema: record.schema,
        record_schema_version: record.record_schema_version,
        writer_version: record.writer_version,
        workspace_id: record.workspace_id,
        merge_id: record.merge_id,
        operation_id: record.operation_id,
        state: record.state,
        source_ref: record.source_ref,
        mode: record.mode,
        created_at: record.created_at,
        baseline: record.baseline,
        selected_targets: record.selected_targets,
        participants: record.participants,
        publication: record.publication,
        operation_drift: record.operation_drift,
        accepted_workspace: None,
        recovery_context: None,
        pending_rollback: None,
        pending_preservation: None,
        preservation_publication_handoff: None,
        extensions: record.extensions,
    }
}

/// Create the v1 record for one accepted start and run it to its next durable
/// stop under the v1 service.
///
/// **DR-1 ship (1) W3** (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1, 2026-09-03):
/// the crash-recovery decision is made HERE, before any lease is taken, and it
/// is made exactly once for this process. Above the bar nothing changes. Below
/// it, `filesystem_strict` refuses before any lease, any record and any Git
/// work; by default one `Diagnostic`/`Warn` event carries the operator's
/// sentence and the start proceeds on the catalog-free creation lease. The
/// decision is then PASSED to `service::run` rather than re-taken, so the
/// `ResumeStart` invocation inside this start neither re-probes nor re-warns.
pub(in crate::workspace_ops::merge) fn handle_start_durable_v1<B: MergeAuthorityBackend>(
    backend: &B,
    root: &Path,
    record: MergeOperationRecord,
    filesystem_strict: bool,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let record = created_v1_record(record);
    let merge_id = record.merge_id.clone();
    let store = CheckedV1Store::default();

    let decision = crate::checked_artifact::entry::crash_recovery_decision(root)?;
    let below_bar = matches!(
        decision,
        crate::checked_artifact::entry::CrashRecoveryDecision::Unsupported { .. }
    );
    if below_bar {
        if filesystem_strict {
            return Err(decision.crash_recovery_strict_refusal());
        }
        warn_once(emitter, &decision);
    }

    // Scoped: `service::run` takes its own `V1MutationLease`, and the
    // workspace mutator lock is not re-entrant, so the creation lease must be
    // released before the service starts.
    let created_state = {
        // E4.2: the creation lease activates AND bootstraps §10 row `:273`'s two
        // managed parents, so both are durable before this record is written.
        // W3: below the bar there is no catalog to activate, so the same two
        // parents are prepared through the legacy checked boundary instead.
        let lease = if below_bar {
            super::checked::V1MutationLease::acquire_for_merge_start_uncatalogued(root)?
        } else {
            super::checked::V1MutationLease::acquire_for_merge_start(root, &record.workspace_id)?
        };
        store.create_open(&lease, root, &record)?.record().state
    };
    emitter.operation_state_changed(created_state.into());

    let mut runtime = ForwardRuntime::new(backend, context);
    let disposition = super::service::run(
        &store,
        root,
        &merge_id,
        V1LifecycleRequest::ResumeStart,
        Some(&decision),
        &mut runtime,
    )?
    .disposition();
    let mut response = respond(backend, &store, root, &merge_id, disposition, context)?;
    response.crash_recovery = Some(decision.crash_recovery_protocol());
    Ok(response)
}

/// The one diagnostic a below-bar invocation emits (charter §3.4, channel 1).
///
/// `EventKind::Diagnostic` with `Severity::Warn`, no member, the operator's
/// exact sentence as `message`. Called from exactly two places — a start's
/// decision and a continue's — and each of those decides once, so the "at most
/// one diagnostic per process" invariant is a property of the call sites, not
/// of a counter.
fn warn_once(
    emitter: &EventEmitter<'_>,
    decision: &crate::checked_artifact::entry::CrashRecoveryDecision,
) {
    emitter.emit(
        crate::EventKind::Diagnostic,
        crate::Severity::Warn,
        None,
        None,
        Some(decision.crash_recovery_warning()),
        None,
    );
}

/// Project one v1 service disposition into the merge response.
///
/// A terminal record is archived before it is projected: the archived
/// projection reads only the exact done-record bytes (contract §7), so the
/// response a completed start returns is the archive's, never a synthesized
/// one.
pub(in crate::workspace_ops::merge) fn respond<B: MergeAuthorityBackend>(
    backend: &B,
    store: &CheckedV1Store,
    root: &Path,
    merge_id: &str,
    disposition: V1ResponseDisposition,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    match disposition {
        V1ResponseDisposition::Terminal(_) | V1ResponseDisposition::ArchiveReady => {
            let archived =
                super::archive::archive_terminal(backend, store, root, merge_id, context)?;
            super::status::archived_status(merge_id, &archived, context)
        }
        V1ResponseDisposition::Status | V1ResponseDisposition::Stopped(_) => {
            super::status::open_status(backend, store, root, merge_id, context)
        }
    }
}

/// Serve one merge command whose open record is v1.
///
/// Safety review §2.4: "dispatch routes ... every v1-record
/// continue/resume/abort/status/gc into the v1 lifecycle service". `Resume`
/// and `Abort` enter the service with their own request kinds; `Status` is
/// read-only and never mutates; `Gc` refuses while a record is open, exactly
/// as the v0 lifecycle does.
///
/// **DR-1 ship (1) W3** (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1, 2026-09-03):
/// a continue is a NEW PROCESS, so it decides for itself — once — and, below the
/// bar, warns once for this invocation before proceeding catalog-free. It never
/// consults `filesystem_strict`: that flag is accepted only on a start
/// (`validate.rs`), and the operator's rule is that a later continue or abort of
/// an attempt "does not consult the flag again". `Abort`, `Preserve`, `Status`
/// and `Gc` are untouched and answer `crash_recovery = None`: they decide
/// nothing, and abort in particular must stay capability-free by path.
pub(in crate::workspace_ops::merge) fn handle_v1_command<B: MergeAuthorityBackend>(
    backend: &B,
    root: &Path,
    merge_id: &str,
    request: &crate::MergeRequest,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let store = CheckedV1Store::default();
    super::super::validate_open_merge_id(request.merge_id.as_deref(), merge_id)?;
    match request.op {
        crate::MergeOp::Status => {
            super::status::open_status(backend, &store, root, merge_id, context)
        }
        crate::MergeOp::Gc => Err(ModelError::new(
            ErrorCode::OpenOperation,
            format!("cannot collect archived merge records while merge '{merge_id}' is open"),
        )),
        crate::MergeOp::Resume | crate::MergeOp::Abort => {
            let lifecycle = match (request.op, request.preserve) {
                (crate::MergeOp::Resume, _) => V1LifecycleRequest::Continue,
                (crate::MergeOp::Abort, Some(true)) => V1LifecycleRequest::Preserve,
                (crate::MergeOp::Abort, _) => V1LifecycleRequest::Abort,
                _ => unreachable!("only resume and abort reach this arm"),
            };
            let mut decision = None;
            let disposition = match lifecycle {
                V1LifecycleRequest::Continue => {
                    let made = crate::checked_artifact::entry::crash_recovery_decision(root)?;
                    if matches!(
                        made,
                        crate::checked_artifact::entry::CrashRecoveryDecision::Unsupported { .. }
                    ) {
                        warn_once(emitter, &made);
                    }
                    let decided = decision.insert(made);
                    let mut runtime = ForwardRuntime::new(backend, context);
                    super::service::run(
                        &store,
                        root,
                        merge_id,
                        lifecycle,
                        Some(decided),
                        &mut runtime,
                    )?
                }
                _ => {
                    let mut runtime = super::reverse::ReverseRuntime::new(backend, context);
                    super::service::run(&store, root, merge_id, lifecycle, None, &mut runtime)?
                }
            }
            .disposition();
            let mut response = respond(backend, &store, root, merge_id, disposition, context)?;
            response.crash_recovery = decision.map(|decision| decision.crash_recovery_protocol());
            Ok(response)
        }
        crate::MergeOp::Start => unreachable!("start never routes through an open record"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::workspace_ops::merge::model::v1::{
        MERGE_RECORD_SCHEMA_V1, MERGE_RECORD_SCHEMA_VERSION_V1,
    };
    use crate::workspace_ops::merge::{MergeBaseline, MergeExecutionMode, OperationState};

    /// A record shaped exactly as `start::create_record` leaves one: the
    /// writer floor's envelope, `Executing`, no participants resolved yet.
    fn created(mode: MergeExecutionMode) -> MergeOperationRecord {
        MergeOperationRecord {
            schema: MERGE_RECORD_SCHEMA_V1.to_owned(),
            record_schema_version: MERGE_RECORD_SCHEMA_VERSION_V1,
            writer_version: crate::VERSION.to_owned(),
            workspace_id: "ws_default".to_owned(),
            merge_id: "merge_1".to_owned(),
            operation_id: "op_1".to_owned(),
            state: OperationState::Executing,
            source_ref: "feature/source".to_owned(),
            mode,
            created_at: "now".to_owned(),
            baseline: MergeBaseline {
                lock_sha256: "lock".to_owned(),
                manifest_sha256: "manifest".to_owned(),
                lock_yaml: None,
                manifest_yaml: None,
                lock_commit_sha256: None,
                manifest_commit_sha256: None,
                root_head: None,
                root_branch: None,
                extensions: BTreeMap::new(),
            },
            selected_targets: Vec::new(),
            participants: BTreeMap::new(),
            publication: None,
            operation_drift: Vec::new(),
            extensions: BTreeMap::new(),
        }
    }

    /// Creation lifts the shared body onto v1 and states every v1-only field
    /// absent. Those fields are constructed by migration, not by creation
    /// (contract §4: "Migration constructs or recovers `AcceptedWorkspace` in
    /// the migration write. It is not a later lifecycle action."), so a
    /// freshly created record must carry none of them.
    #[test]
    fn creation_lifts_the_shared_body_and_states_v1_only_fields_absent() {
        let v0 = created(MergeExecutionMode::NoFf);
        let lifted = created_v1_record(v0.clone());

        assert_eq!(lifted.schema, MERGE_RECORD_SCHEMA_V1);
        assert_eq!(lifted.record_schema_version, MERGE_RECORD_SCHEMA_VERSION_V1);
        assert_eq!(lifted.merge_id, v0.merge_id);
        assert_eq!(lifted.operation_id, v0.operation_id);
        assert_eq!(lifted.state, v0.state);
        assert_eq!(lifted.source_ref, v0.source_ref);
        assert_eq!(lifted.mode, v0.mode);
        assert_eq!(lifted.baseline, v0.baseline);
        assert_eq!(lifted.selected_targets, v0.selected_targets);
        assert_eq!(lifted.participants, v0.participants);
        assert_eq!(lifted.publication, v0.publication);
        assert_eq!(lifted.operation_drift, v0.operation_drift);
        assert_eq!(lifted.extensions, v0.extensions);

        assert!(lifted.accepted_workspace.is_none());
        assert!(lifted.recovery_context.is_none());
        assert!(lifted.pending_rollback.is_none());
        assert!(lifted.pending_preservation.is_none());
        assert!(lifted.preservation_publication_handoff.is_none());
    }
}
