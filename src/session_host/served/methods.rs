//! The method registry and class table: core's half of the host crate's
//! ports (gwz-dev `dev-docs/GwzCoreSessionDesign.md` §4.2, §5.1; crate map §2,
//! §3), and the decoding of the request messages of the methods declared with
//! scalar parameters (§13).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Direct methods: `status`, `ls`, `resolve_forall_targets`, `list_snapshots`, `diff`, `log`, `transport_capabilities`, `configure_transport_runtime` | §4.2 |
//! | Class N: `fetch`, and `push` with its one-per-workspace rule | §5.1 |
//! | Class W: every other method, `remote_identity` included, and any method the table does not name | §4.2, §5.1; `GwzSplitCommandChannel.md` §3.2 |
//! | `log_verb` only on the log methods; on any other method it is `invalid_request` | §4.1 |
//! | `operation.cancel` names exactly one target, a `call_id` of at least 1 or an `operation_id` | §4.2, §13 |

use std::time::Duration;

use gwz_session_contract::Lane;

use gwz_session_host::{
    Call, CancelTarget, Class, Code, EventsCall, EventsRead, HostError, MAX_READ_WAIT, Request,
    Verb,
};

/// The direct methods (§4.2).
pub(crate) const DIRECT_METHODS: [&str; 8] = [
    "status",
    "ls",
    "resolve_forall_targets",
    "list_snapshots",
    "diff",
    "log",
    "transport_capabilities",
    "configure_transport_runtime",
];

/// The log methods, which alone take a `log_verb` (§4.1, §4.2).
const LOG_METHODS: [&str; 3] = ["events.subscribe", "diff.output", "log.output"];

/// The methods that create a workspace. A served core serves one existing
/// workspace, so resolution refuses them (`GwzSplitCommandChannel.md` §1,
/// §4.5).
pub(crate) const CREATING_METHODS: [&str; 4] = [
    "create_workspace",
    "init_from_sources",
    "clone_workspace",
    "clone_local_workspace",
];

/// The lane a call of `method` travels on (§3): `operation.cancel` and
/// `session.close` on the control lane, every other call on the call lane.
pub(crate) fn lane(method: &str) -> Lane {
    match method {
        "operation.cancel" | "session.close" => Lane::Control,
        _ => Lane::Call,
    }
}

/// The class of an operation method (§5.1): any method not named here is W.
pub(crate) fn class(method: &str) -> Class {
    match method {
        "fetch" => Class::Fetch,
        "push" => Class::Push,
        _ => Class::Write,
    }
}

fn invalid(message: impl Into<String>) -> HostError {
    HostError::new(Code::InvalidRequest, message)
}

/// Classifies a call by its method (§4.2, §5.1).
pub(crate) fn classify(call: &Call) -> Result<Request, HostError> {
    let method = call.method.as_str();
    if call.verb.is_some() && !LOG_METHODS.contains(&method) {
        return Err(invalid(format!(
            "log_verb applies only to the log methods, not {method}"
        )));
    }
    let request = &call.request;
    Ok(match method {
        "events.subscribe" => Request::Events(events(call)?),
        "diff.output" | "log.output" => Request::Spool,
        "operation.result" => Request::Result {
            operation_id: decode(request, crate::OperationResultRequest::decode)?.operation_id,
        },
        "operation.response" => Request::Response {
            operation_id: decode(request, crate::OperationResponseRequest::decode)?.operation_id,
        },
        "operation.release" => Request::Release {
            operation_id: decode(request, crate::OperationReleaseRequest::decode)?.operation_id,
        },
        "operation.cancel" => {
            let cancel = decode(request, crate::OperationCancelRequest::decode)?;
            Request::Cancel(match (cancel.call_id, cancel.operation_id) {
                (Some(call_id), None) if call_id >= 1 => CancelTarget::Call(call_id as u64),
                (None, Some(operation_id)) => CancelTarget::Operation(operation_id),
                _ => {
                    return Err(invalid(
                        "operation.cancel names exactly one target: a call_id of at least 1 or an operation_id",
                    ));
                }
            })
        }
        "session.close" => {
            decode(request, crate::SessionCloseRequest::decode)?;
            Request::Close
        }
        _ if DIRECT_METHODS.contains(&method) => Request::Direct,
        _ => Request::Operation {
            class: class(method),
            // §5.4: a merge declares its response as a view beside its result.
            response_view: method == "merge",
        },
    })
}

fn decode<T>(
    bytes: &[u8],
    decoder: impl FnOnce(&[u8]) -> Result<T, crate::cbor::DecodeError>,
) -> Result<T, HostError> {
    decoder(bytes).map_err(|_| invalid("the call's request message does not decode"))
}

/// An `events.subscribe` call: taut-shape's `LogReadRequest` or
/// `LogEndStream`, whose `log_id` is the operation's ID (§4.2).
fn events(call: &Call) -> Result<EventsCall, HostError> {
    let undecodable = || invalid("the events.subscribe request does not decode");
    match call.verb.unwrap_or(Verb::Read) {
        Verb::EndStream => {
            let end = taut_shape::generated::LogEndStream::decode(&call.request)
                .map_err(|_| undecodable())?;
            Ok(EventsCall::End {
                operation_id: end.log_id,
                stream_id: end.stream_id,
            })
        }
        Verb::Read => {
            let read = taut_shape::generated::LogReadRequest::decode(&call.request)
                .map_err(|_| undecodable())?;
            let count = |value: Option<i64>| value.map(|value| value.max(0) as usize);
            let wait = read.timeout_ms.map_or(MAX_READ_WAIT, |ms| {
                Duration::from_millis(ms.max(0) as u64).min(MAX_READ_WAIT)
            });
            Ok(EventsCall::Read(EventsRead {
                operation_id: read.log_id,
                stream_id: read.stream_id,
                cursor: read.cursor.map(|cursor| cursor.seq.max(0) as u64),
                max_records: count(read.max_records),
                max_bytes: count(read.max_bytes),
                wait,
            }))
        }
    }
}
