use super::*;
use std::path::Path;

fn request() -> crate::MergeRequest {
    crate::MergeRequest {
        meta: crate::RequestMeta {
            request_id: "req".to_owned(),
            schema_version: "gwz.v0".to_owned(),
            workspace: Some(crate::WorkspaceRef {
                root: Some(".".to_owned()),
                workspace_id: None,
            }),
            ..crate::RequestMeta::default()
        },
        op: crate::MergeOp::Status,
        source_ref: None,
        merge_id: None,
        mode: None,
        message: None,
        preserve: None,
    }
}

#[test]
fn public_handler_exposes_the_frozen_service_entry() {
    let backend = crate::git::Git2Backend::new();
    let response = handle_merge(&backend, Path::new("."), request(), "op_1").unwrap();
    assert_eq!(response.state, crate::MergeOperationState::Idle);
}

mod dispatch;
mod mutation_guard;
mod open_gate;
