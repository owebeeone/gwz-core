//! Dispatch to core's handlers: the served core's call encoding of the
//! existing Taut requests (gwz-dev `dev-docs/GwzCoreSessionDesign.md` §4.1,
//! §5.2; step 2.2b).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | A call's `request` is the Taut request message its method declares, and its reply the Taut response message | §4.1 |
//! | Each method runs its existing gwz-core handler, with the request's own caller directory as its start and the operation's ID | §5.2 |
//! | Handlers report events through the operation's gate | §5.2, §6 |
//! | `diff` and `log` register their spools in the session's own registries, reached through the gate | §5.4, §5.6 |
//! | A method with no handler here is refused with `unsupported_operation`, before any effect | §4.2 |
//!
//! The backend is the composition's: the server step (2.3) passes the 1.7
//! driver-only backend, whose transport the carrier-backed handle (2.2c)
//! backs. Nothing here builds a runtime or reads an endpoint environment.

use std::path::Path;

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventSink;
use crate::workspace_ops as ops;
use gwz_session_host::Worker;

use super::ServedSession;

/// How a served session runs a call's handler. Production uses
/// [`CoreDispatch`]; tests script their own.
pub(crate) trait Dispatch: Send + Sync + 'static {
    /// Runs `method`'s handler on `request` and returns its encoded response.
    fn run(
        &self,
        method: &str,
        request: &[u8],
        start: &Path,
        worker: &Worker<ServedSession>,
        events: &dyn EventSink,
    ) -> ModelResult<Vec<u8>>;
}

/// The handlers of gwz-core, on the composition's backend.
pub(crate) struct CoreDispatch {
    backend: crate::git::Git2Backend,
}

impl CoreDispatch {
    /// Dispatch on `backend`.
    #[allow(
        dead_code,
        reason = "the server step (2.3) composes it with its backend"
    )]
    pub(crate) fn new(backend: crate::git::Git2Backend) -> Self {
        Self { backend }
    }
}

fn undecodable(_: crate::cbor::DecodeError) -> ModelError {
    ModelError::new(
        ErrorCode::InvalidRequest,
        "the call's request message does not decode",
    )
}

fn refused(error: gwz_session_host::Refused) -> ModelError {
    error.into()
}

/// Decodes `$bytes` as `$request`, runs `$call` with it bound to `$name`, and
/// encodes the response.
macro_rules! handle {
    ($bytes:expr, $request:ty, |$name:ident| $call:expr) => {{
        let $name = <$request>::decode($bytes).map_err(undecodable)?;
        let response = $call?;
        Ok(crate::encode(&response.to_cbor()))
    }};
}

impl Dispatch for CoreDispatch {
    fn run(
        &self,
        method: &str,
        request: &[u8],
        start: &Path,
        worker: &Worker<ServedSession>,
        events: &dyn EventSink,
    ) -> ModelResult<Vec<u8>> {
        let backend = &self.backend;
        let id = worker.operation_id().unwrap_or("direct").to_owned();
        match method {
            "status" => handle!(request, crate::StatusRequest, |r| {
                crate::status::handle_status(backend, start, r, id)
            }),
            "ls" => handle!(request, crate::LsRequest, |r| ops::handle_ls(start, r, id)),
            "resolve_forall_targets" => handle!(request, crate::LsRequest, |r| {
                ops::resolve_forall_targets(start, r, id)
            }),
            "list_snapshots" => handle!(request, crate::ListSnapshotsRequest, |r| {
                ops::handle_list_snapshots(start, r, id)
            }),
            "transport_capabilities" => {
                handle!(request, crate::TransportCapabilitiesRequest, |r| {
                    crate::protocol::transport_capabilities::handle(r)
                })
            }
            "configure_transport_runtime" => {
                handle!(request, crate::TransportRuntimeRequest, |r| {
                    crate::protocol::transport_capabilities::configure_runtime(r)
                })
            }
            "diff" => {
                let registry = worker
                    .effect(|scope| scope.session().spools())
                    .map_err(refused)??
                    .diffs;
                handle!(request, crate::DiffRequest, |r| {
                    crate::diff::handle_diff(start, r, id, &registry)
                        .map(|outcome| outcome.response)
                })
            }
            "log" => {
                let registry = worker
                    .effect(|scope| scope.session().spools())
                    .map_err(refused)??
                    .logs;
                handle!(request, crate::LogRequest, |r| {
                    crate::operation::handle_log(backend, start, r, id, &registry)
                })
            }
            "fetch" => handle!(request, crate::FetchRequest, |r| {
                ops::handle_fetch_with_events(backend, start, r, id, events)
            }),
            "push" => handle!(request, crate::PushRequest, |r| {
                ops::handle_push_with_events(backend, start, r, id, events)
            }),
            "pull_head" => handle!(request, crate::PullHeadRequest, |r| {
                ops::handle_pull_head_with_events(backend, start, r, id, events)
            }),
            "pull_snapshot" => handle!(request, crate::PullSnapshotRequest, |r| {
                ops::handle_pull_snapshot(backend, start, r, id, events)
            }),
            "materialize" => handle!(request, crate::MaterializeRequest, |r| {
                ops::handle_materialize(backend, start, r, id, events)
            }),
            "clone_repo_member" => handle!(request, crate::CloneRepoMemberRequest, |r| {
                ops::handle_clone_repo_member(backend, start, r, id, events)
            }),
            "attach_repo_member" => handle!(request, crate::AttachRepoMemberRequest, |r| {
                ops::handle_attach_repo_member(backend, start, r, id, events)
            }),
            "detach_repo_member" => handle!(request, crate::DetachRepoMemberRequest, |r| {
                ops::handle_detach_repo_member(backend, start, r, id)
            }),
            "merge" => handle!(request, crate::MergeRequest, |r| {
                ops::handle_merge_with_local_family(backend, start, r, id, events)
            }),
            "local_family" => handle!(request, crate::LocalFamilyRequest, |r| {
                ops::handle_local_family(backend, start, r, id, events)
            }),
            "snapshot" => handle!(request, crate::SnapshotRequest, |r| {
                ops::handle_snapshot(backend, start, r, id)
            }),
            "capture" => handle!(request, crate::CaptureRequest, |r| {
                ops::handle_capture(backend, start, r, id)
            }),
            "commit" => handle!(request, crate::CommitRequest, |r| {
                ops::handle_commit(backend, start, r, id)
            }),
            "stage" => handle!(request, crate::StageRequest, |r| {
                ops::handle_stage(backend, start, r, id)
            }),
            "branch" => handle!(request, crate::BranchRequest, |r| {
                ops::handle_branch(backend, start, r, id)
            }),
            "stash" => handle!(request, crate::StashRequest, |r| {
                ops::handle_stash(backend, start, r, id)
            }),
            "repo_sync" => handle!(request, crate::RepoSyncRequest, |r| {
                ops::handle_repo_sync(backend, start, r, id)
            }),
            "create_repo" => handle!(request, crate::CreateRepoRequest, |r| {
                ops::handle_create_repo(backend, start, r, id)
            }),
            "add_existing_repo" => handle!(request, crate::AddExistingRepoRequest, |r| {
                ops::handle_add_existing_repo(backend, start, r, id)
            }),
            "remote_identity" => handle!(request, crate::RemoteIdentityRequest, |r| {
                ops::handle_remote_identity(backend, start, r, id)
            }),
            "tag" => handle!(request, crate::TagRequest, |r| {
                let services = backend.operation_services();
                ops::handle_tag_with_services(&services, backend, start, r, id)
            }),
            other => Err(ModelError::new(
                ErrorCode::UnsupportedOperation,
                format!("a served core runs no method {other}"),
            )),
        }
    }
}
