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
use super::events::LifecycleEvents;
use super::forward::ForwardRuntime;
use super::store::CheckedV1Store;
use crate::git::MergeAuthorityBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext};
use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;

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
    record: MergeOperationRecordV1,
    filesystem_strict: bool,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
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
        store.create_open(&lease, root, &record)?.record().clone()
    };
    // M5d charter §4: the creation write is one `artifact_written`, then the
    // state the store committed — `merge/start.rs:119-120` on the v0 path.
    let mut events = LifecycleEvents::new(emitter);
    events.created(&created_state);

    let mut runtime = ForwardRuntime::new(backend, context);
    let disposition = super::service::run(
        &store,
        root,
        &merge_id,
        V1LifecycleRequest::ResumeStart,
        Some(&decision),
        &mut runtime,
        &mut events,
    )?
    .disposition();
    let mut response = respond(
        backend,
        &store,
        root,
        &merge_id,
        disposition,
        context,
        &mut events,
    )?;
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
    events: &mut LifecycleEvents<'_>,
) -> ModelResult<crate::MergeResponse> {
    match disposition {
        V1ResponseDisposition::Terminal(_) | V1ResponseDisposition::ArchiveReady => {
            let archived =
                super::archive::archive_terminal(backend, store, root, merge_id, context, events)?;
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
            let mut events = LifecycleEvents::new(emitter);
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
                        &mut events,
                    )?
                }
                _ => {
                    let mut runtime = super::reverse::ReverseRuntime::new(backend, context);
                    super::service::run(
                        &store,
                        root,
                        merge_id,
                        lifecycle,
                        None,
                        &mut runtime,
                        &mut events,
                    )?
                }
            }
            .disposition();
            let mut response = respond(
                backend,
                &store,
                root,
                merge_id,
                disposition,
                context,
                &mut events,
            )?;
            response.crash_recovery = decision.map(|decision| decision.crash_recovery_protocol());
            Ok(response)
        }
        crate::MergeOp::Start => unreachable!("start never routes through an open record"),
    }
}
