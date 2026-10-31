//! The served core's operation service: core's implementation of the host
//! crate's ports (gwz-dev `dev-docs/GwzSplitCommandChannel.md` §3.2 and §3.3;
//! `GwzCoreSessionDesign.md` §4.1, §4.2, §5; step 2.2b of
//! `GwzTransportReleaseConvergencePlan.md`).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Calls are `SessionCall` frames (tag 1) whose `request` is the existing Taut request; replies are `SessionReply` and `SessionError` (tags 2 and 3) | §4.1 |
//! | One workspace per server: resolution refuses a request whose workspace root is not the served one, or that creates a workspace, with `invalid_request` naming the served root | 2.0 §1, §3.2 |
//! | The session carries no endpoint environment: Git runs with the core process's own | 2.0 §3.1, §3.2, §3.5 |
//! | Host refusals take their own wire codes: `transport_session_full` (77), `operation_expired` (78), `cancelled` (73) and the rest | §4.2; 2.0 §1 |
//! | A cancelled terminal's result has `errors[0] = cancelled` and the handler's own error after it; a detached one adds "detached at the close bound; outcome unknown" | §6 |
//! | The session's `diff.output` and `log.output` logs live in its context under the §5.4 bounds (`spool`) and are released at its end | §5.4, §8 step 7 |
//! | The in-process reader hands every frame to the service's session-class seam; transport-lane frames (tags 16 and 17) and every per-binding state are step 2.1's carrier's, whose router consults the service's live-call registry | 2.0 §3.3 "Reader interface", §3.4 |
//! | The handler context, the worker, carries the call's token: `CoreDispatch` refuses at its gated steps once it is cancelled. The handlers below it read no token yet (plumbing only; the carrier-backed handle of step 2.2c ends a running exchange) | §5.2 |
//!
//! Recorded deviations, accepted by the lane owner (2.2b remediation plan,
//! decision 3): `diff.output` reads are probes, never held (the log
//! service's v0 rule), and cancelling a spent read, result or control call
//! answers `operation_expired`, not `invalid_request`. The §5.1 member lock
//! manager is step 2.3's (decision 4): until it lands, two fetches of one
//! session can meet on a member's refs and one fails on Git's own ref lock,
//! the outcome two CLI processes get, where §5.1 would make the second wait.
//!
//! Sessions of one host share its detached-worker record (2.0 §1), which the
//! host context holds. The in-process carrier is [`serve_in_process`]; the
//! server step (2.3) serves byte streams through 2.1's router.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gwz_session_channel::{InProcessEnd, pair};
use gwz_session_contract::{Frame, FrameSink, FrameSource, Lane, Tag};
use gwz_session_host::{
    Call, Cleanup, Code, Ending, EventsBatch, EventsRead, HostError, HostPorts, Limits, Mode,
    Outcome, ProtocolError, ReplySink, Request, Resolved, Service, Terminal, Verb, Worker,
    validate_limits,
};

use super::context::HostContext;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventSink;
use crate::operation::MemberLockManager;

mod dispatch;
mod methods;
mod spool;

pub(crate) use dispatch::Dispatch;
pub(crate) use spool::{Kind as SpoolKind, Spools};

cfg_if::cfg_if! {
    // The frozen body of the transport lane, `TransportLaneFrame` (2.0
    // §3.3): its codec only. Routing tags 16 and 17, and every per-binding
    // state, is step 2.1's carrier's (2.0 §3.3 "Reader interface").
    if #[cfg(gwz_transport_candidate)] {
        pub(crate) mod lane;
    }
}

/// A served session's own data, which its gates hold (§5.6 without the
/// endpoint environment, 2.0 §3.2).
pub(crate) struct ServedSession {
    /// The one workspace the server serves, as resolution finds it.
    root: PathBuf,
    /// The session's `diff.output` and `log.output` logs, under the §5.4
    /// bounds, until its end.
    spools: Arc<Spools>,
    /// The host context's member lock manager, shared with every session of
    /// that host context (§5.1).
    members: MemberLockManager,
}

impl ServedSession {
    /// The session's spool layer.
    pub(crate) fn spools(&self) -> Arc<Spools> {
        Arc::clone(&self.spools)
    }

    /// The member lock manager of the session's host context.
    pub(crate) fn members(&self) -> MemberLockManager {
        self.members.clone()
    }

    /// The served workspace root.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
}

/// A terminal's error as the wire carries it.
#[derive(Clone, Debug)]
pub(crate) struct ServedFailure {
    pub(crate) error: crate::GwzError,
    pub(crate) meta: Option<crate::ResponseMeta>,
    /// The handler's own error behind a cancellation, for the result (§6).
    pub(crate) cause: Option<crate::GwzError>,
}

impl ServedFailure {
    fn model(error: &ModelError) -> Self {
        Self {
            error: error.into(),
            meta: error.response_meta.as_deref().cloned(),
            cause: None,
        }
    }

    fn code(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::model(&ModelError::new(code, message))
    }
}

/// Core's ports.
pub(crate) struct ServedPorts {
    dispatch: Arc<dyn Dispatch>,
}

fn encode(cbor: &crate::Cbor) -> Vec<u8> {
    crate::encode(cbor)
}

/// The caller's directory of a request, from its `RequestMeta` (field 1).
fn request_meta(request: &[u8]) -> ModelResult<crate::RequestMeta> {
    let undecodable = |_| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            "the call's request message does not decode",
        )
    };
    let cbor = crate::try_decode(request).map_err(undecodable)?;
    let meta = cbor.try_get(1).map_err(undecodable)?;
    crate::RequestMeta::from_cbor(meta).map_err(undecodable)
}

/// The methods that carry no `RequestMeta` and act on no workspace.
fn takes_no_workspace(method: &str) -> bool {
    matches!(
        method,
        "transport_capabilities" | "configure_transport_runtime"
    )
}

fn action(method: &str) -> crate::ActionKind {
    use crate::ActionKind as A;
    match method {
        "add_existing_repo" => A::AddExistingRepo,
        "create_repo" => A::CreateRepo,
        "materialize" => A::Materialize,
        "status" => A::Status,
        "snapshot" => A::Snapshot,
        "tag" => A::Tag,
        "pull_head" => A::PullHead,
        "pull_snapshot" => A::PullSnapshot,
        "push" => A::Push,
        "capture" => A::Capture,
        "commit" => A::Commit,
        "stage" => A::Stage,
        "ls" => A::Ls,
        "resolve_forall_targets" => A::Forall,
        "repo_sync" => A::RepoSync,
        "stash" => A::Stash,
        "branch" => A::Branch,
        "list_snapshots" => A::ListSnapshots,
        "diff" => A::Diff,
        "clone_repo_member" => A::CloneRepoMember,
        "detach_repo_member" => A::DetachRepoMember,
        "attach_repo_member" => A::AttachRepoMember,
        "merge" => A::Merge,
        "log" => A::Log,
        "local_family" => A::LocalFamily,
        "remote_identity" => A::RemoteIdentity,
        "fetch" => A::Fetch,
        _ => A::default(),
    }
}

/// An operation's result (§6), from its response's envelope when it has one.
fn result(
    call: &Call,
    operation_id: &str,
    started: i64,
    envelope: Option<crate::ResponseEnvelope>,
    errors: Vec<crate::GwzError>,
) -> Vec<u8> {
    let request_id = request_meta(&call.request)
        .map(|meta| meta.request_id)
        .unwrap_or_default();
    let (status, members, mut all_errors, transport) = match envelope {
        Some(envelope) => (
            envelope.meta.aggregate_status,
            envelope.members,
            envelope.errors,
            envelope.meta.transport,
        ),
        None => (crate::AggregateStatus::Failed, Vec::new(), Vec::new(), None),
    };
    let mut leading = errors;
    leading.append(&mut all_errors);
    let result = crate::OperationResult {
        operation_id: operation_id.to_owned(),
        request_id,
        action: action(&call.method),
        aggregate_status: status,
        started_at_ms: started,
        finished_at_ms: crate::operation::now_ms().0,
        members,
        errors: leading,
        attribution: None,
        transport,
    };
    encode(&result.to_cbor())
}

/// The envelope in a response message's field 1, when it has one.
fn envelope(response: &[u8]) -> Option<crate::ResponseEnvelope> {
    let cbor = crate::try_decode(response).ok()?;
    crate::ResponseEnvelope::from_cbor(cbor.try_get(1).ok()?).ok()
}

/// Events from a handler, appended through its gate (§5.2, §6).
struct GateEvents<'a> {
    worker: &'a Worker<ServedSession>,
}

impl EventSink for GateEvents<'_> {
    fn deliver(&self, event: crate::OperationEvent) {
        let _ = self.worker.event(encode(&event.to_cbor()));
    }
}

impl HostPorts for ServedPorts {
    type Session = ServedSession;
    type Failure = ServedFailure;

    fn decode(&self, body: &[u8]) -> Result<Call, ProtocolError> {
        let call = crate::SessionCall::decode(body)
            .map_err(|_| ProtocolError("a SessionCall body that does not decode".to_owned()))?;
        if call.call_id < 1 {
            return Err(ProtocolError(format!(
                "call_id {} is below 1",
                call.call_id
            )));
        }
        Ok(Call {
            call_id: call.call_id as u64,
            method: call.method,
            mode: match call.mode {
                Some(crate::SessionMode::Submit) => Mode::Submit,
                _ => Mode::Unary,
            },
            request: call.request,
            verb: call.log_verb.map(|verb| match verb {
                crate::LogVerb::Read => Verb::Read,
                crate::LogVerb::EndStream => Verb::EndStream,
            }),
        })
    }

    fn lane(&self, method: &str) -> Lane {
        methods::lane(method)
    }

    fn classify(&self, call: &Call) -> Result<Request, HostError> {
        methods::classify(call)
    }

    fn resolve(&self, session: &ServedSession, call: &Call) -> Result<Resolved, ServedFailure> {
        let served = session.root().display().to_string();
        if takes_no_workspace(&call.method) {
            return Ok(Resolved {
                workspace: served,
                request_id: String::new(),
            });
        }
        let refuse = |message: String| ServedFailure::code(ErrorCode::InvalidRequest, message);
        if methods::CREATING_METHODS.contains(&call.method.as_str()) {
            return Err(refuse(format!(
                "this server serves the existing workspace at {served}; it creates no workspace"
            )));
        }
        let meta = request_meta(&call.request).map_err(|error| ServedFailure::model(&error))?;
        let start = crate::workspace_ops::caller_directory(&meta)
            .map_err(|error| ServedFailure::model(&error))?;
        let root = crate::workspace_ops::resolve_request_workspace_root(&start, &meta)
            .map_err(|error| ServedFailure::model(&error))?;
        if root != session.root() {
            return Err(refuse(format!(
                "this server serves the workspace at {served}, not {}",
                root.display()
            )));
        }
        Ok(Resolved {
            workspace: served,
            request_id: meta.request_id,
        })
    }

    fn run(
        &self,
        call: &Call,
        _resolved: &Resolved,
        worker: &Worker<ServedSession>,
    ) -> Outcome<ServedFailure> {
        let started = crate::operation::now_ms().0;
        let start = if takes_no_workspace(&call.method) {
            worker
                .effect(|scope| scope.session().root().to_path_buf())
                .map_err(ModelError::from)
        } else {
            request_meta(&call.request)
                .and_then(|meta| crate::workspace_ops::caller_directory(&meta))
        };
        let events = GateEvents { worker };
        let ran = start.and_then(|start| {
            self.dispatch
                .run(&call.method, &call.request, &start, worker, &events)
        });
        let operation_id = worker.operation_id().map(str::to_owned);
        let finish = |errors: Vec<crate::GwzError>, envelope| {
            operation_id
                .as_deref()
                .map_or_else(Vec::new, |id| result(call, id, started, envelope, errors))
        };
        match ran {
            Ok(response) => Outcome {
                result: finish(Vec::new(), envelope(&response)),
                terminal: Terminal::Completed(response),
                cleanup: Cleanup::default(),
            },
            Err(error) => {
                let mut failure = ServedFailure::model(&error);
                if worker.token().is_cancelled() && error.code != ErrorCode::Cancelled {
                    let cause = failure.error.clone();
                    failure.error = (&ModelError::new(ErrorCode::Cancelled, "cancelled")).into();
                    failure.cause = Some(cause);
                }
                let mut errors = vec![failure.error.clone()];
                errors.extend(failure.cause.clone());
                let result = finish(errors, None);
                let cancelled = failure.error.code == crate::GwzErrorCode::Cancelled;
                Outcome {
                    terminal: if cancelled {
                        Terminal::Cancelled(failure)
                    } else {
                        Terminal::Failed(failure)
                    },
                    result,
                    cleanup: Cleanup::default(),
                }
            }
        }
    }

    fn spool(&self, session: &ServedSession, call: &Call) -> Result<Vec<u8>, ServedFailure> {
        session
            .spools
            .answer(call)
            .map_err(|error| ServedFailure::model(&error))
    }

    fn release_spools(&self, session: &ServedSession) {
        session.spools.release_all();
    }

    fn failure(&self, error: HostError) -> ServedFailure {
        let code = match error.code {
            Code::InvalidRequest => ErrorCode::InvalidRequest,
            Code::OperationNotFound => ErrorCode::OperationNotFound,
            Code::OpenOperation => ErrorCode::OpenOperation,
            Code::OperationExpired => ErrorCode::OperationExpired,
            Code::SessionFull => ErrorCode::TransportSessionFull,
            Code::Cancelled => ErrorCode::Cancelled,
            Code::Internal => ErrorCode::InternalError,
            Code::WorkspaceBusy => ErrorCode::UnsupportedOperation,
        };
        ServedFailure::code(code, error.message)
    }

    fn settled_result(&self, call: &Call, operation_id: &str, failure: &ServedFailure) -> Vec<u8> {
        let started = crate::operation::now_ms().0;
        result(
            call,
            operation_id,
            started,
            None,
            vec![failure.error.clone()],
        )
    }

    fn detached(&self, call: &Call, operation_id: Option<&str>) -> (ServedFailure, Vec<u8>) {
        let failure = ServedFailure::code(ErrorCode::Cancelled, "cancelled at the close bound");
        let detached: crate::GwzError = (&ModelError::new(
            ErrorCode::InternalError,
            "detached at the close bound; outcome unknown",
        ))
            .into();
        let started = crate::operation::now_ms().0;
        let result = operation_id.map_or_else(Vec::new, |id| {
            result(
                call,
                id,
                started,
                None,
                vec![failure.error.clone(), detached],
            )
        });
        (failure, result)
    }

    fn accepted(&self, call: &Call, resolved: &Resolved, operation_id: &str) -> Vec<u8> {
        let meta = request_meta(&call.request).unwrap_or_default();
        let envelope = crate::ResponseEnvelope {
            meta: crate::ResponseMeta {
                request_id: resolved.request_id.clone(),
                schema_version: meta.schema_version,
                action: action(&call.method),
                aggregate_status: crate::AggregateStatus::Accepted,
                operation_id: Some(operation_id.to_owned()),
                attribution: meta.attribution,
                ..crate::ResponseMeta::default()
            },
            members: Vec::new(),
            errors: Vec::new(),
        };
        encode(&spool::accepted(&call.method, envelope))
    }

    fn reset_event(&self, operation_id: &str) -> Vec<u8> {
        let event = crate::OperationEvent {
            operation_id: operation_id.to_owned(),
            kind: crate::EventKind::Reset,
            timestamp_ms: crate::operation::now_ms().0,
            message: Some("older events were dropped".to_owned()),
            ..crate::OperationEvent::default()
        };
        encode(&event.to_cbor())
    }

    fn events(&self, read: &EventsRead, batch: &EventsBatch) -> Vec<u8> {
        use taut_shape::generated as shape;
        let state = if batch.complete {
            shape::LogState::Eof
        } else if batch.records.is_empty() {
            shape::LogState::WouldBlock
        } else {
            shape::LogState::Data
        };
        let response = shape::LogReadResponse {
            log_id: read.operation_id.clone(),
            stream_id: read.stream_id.clone(),
            records: batch
                .records
                .iter()
                .map(|(seq, payload)| shape::LogRecord {
                    seq: *seq as i64,
                    payload: payload.clone(),
                })
                .collect(),
            next_cursor: shape::LogCursor {
                seq: batch.next_cursor as i64,
            },
            state,
            error: None,
        };
        taut_shape::cbor::encode(&response.to_cbor())
    }

    fn events_ended(&self, operation_id: &str, stream_id: &str) -> Vec<u8> {
        let ended = taut_shape::generated::LogEndStream {
            log_id: operation_id.to_owned(),
            stream_id: stream_id.to_owned(),
        };
        taut_shape::cbor::encode(&ended.to_cbor())
    }

    fn cleanup(&self, report: Cleanup) -> Vec<u8> {
        let report = crate::CleanupReport {
            pending_local_work: i64::from(report.pending_local_work),
            peer_cleanup_confirmed: report.peer_cleanup_confirmed,
        };
        encode(&report.to_cbor())
    }

    fn released(&self) -> Vec<u8> {
        encode(&crate::OperationReleaseResponse {}.to_cbor())
    }

    fn reply(&self, call_id: u64, body: Vec<u8>) -> Frame {
        let reply = crate::SessionReply {
            call_id: call_id as i64,
            response: body,
        };
        Frame::new(Tag::SessionReply, encode(&reply.to_cbor()))
    }

    fn error(&self, call_id: u64, failure: &ServedFailure) -> Frame {
        let error = crate::SessionError {
            call_id: call_id as i64,
            error: failure.error.clone(),
            response_meta: failure.meta.clone(),
        };
        Frame::new(Tag::SessionError, encode(&error.to_cbor()))
    }
}

/// A served session over the in-process carrier: the client's end and the
/// service, whose reader thread routes the client's frames.
#[allow(
    dead_code,
    reason = "the server step (2.3) composes it; tests drive it now"
)]
pub(crate) struct ServedChannel {
    pub(crate) client: InProcessEnd,
    pub(crate) service: Service<ServedPorts>,
}

/// Serves one session of the workspace at `root` over an in-process pair,
/// with `dispatch` running its handlers and `host`'s detached-worker record.
/// The in-process session is open at once: `open` is its handshake.
///
/// # Errors
///
/// Limits `open` refuses, a host context already shut down, or a thread that
/// could not be created, each before any effect.
#[allow(
    dead_code,
    reason = "the server step (2.3) composes it; tests drive it now"
)]
pub(crate) fn serve_in_process(
    dispatch: Arc<dyn Dispatch>,
    root: PathBuf,
    host: &HostContext,
    limits: Limits,
) -> ModelResult<ServedChannel> {
    let channel_limits = validate_limits(&limits)?;
    if host.is_shut_down() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "the host context has been shut down",
        ));
    }
    let (client, host_end) = pair(channel_limits);
    let host_end = Arc::new(host_end);
    let sink = Arc::clone(&host_end) as Arc<dyn ReplySink>;
    let session = Arc::new(ServedSession {
        root,
        spools: Arc::new(Spools::new(
            session_tag()?,
            limits.open_logs,
            limits.read_bytes,
        )),
        members: host.members().clone(),
    });
    let ports = ServedPorts { dispatch };
    let service = Service::new(ports, session, sink, limits, host.detached(), true);
    service
        .start()
        .map_err(|_| ModelError::new(ErrorCode::InternalError, "no session thread could start"))?;
    let reader_service = service.clone();
    std::thread::Builder::new()
        .name("gwz-session-reader".into())
        .spawn(move || {
            loop {
                let frame = match host_end.recv() {
                    Ok(frame) => frame,
                    Err(_) => {
                        reader_service.channel_closed(Ending::Channel);
                        return;
                    }
                };
                // The in-process adapter carries tags 1 to 3 only; step 2.1's
                // router, which owns tags 16 and 17, is a byte stream's.
                if reader_service.route(&frame).is_err() {
                    host_end.close();
                    reader_service.channel_closed(Ending::Protocol);
                    return;
                }
            }
        })
        .map_err(|_| ModelError::new(ErrorCode::InternalError, "no session reader could start"))?;
    Ok(ServedChannel { client, service })
}

/// A session's own prefix of the log ids it issues, from the operating
/// system's random source, so no other session can name its logs (§5.4).
fn session_tag() -> ModelResult<String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|error| {
        ModelError::new(
            ErrorCode::InternalError,
            format!("no random session tag: {error}"),
        )
    })?;
    Ok(format!(
        "s{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
