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
        filesystem_strict: None,
    }
}

/// Write one valid **open v1** record — `gwz.merge-operation/v1`,
/// `record_schema_version: 1` — with a single conflicted member participant,
/// and answer its merge id.
///
/// This is the shape A1's writer floor leaves on disk for every conflicted
/// `--no-ff` merge, and the shape the gates could not read: the v0 store's
/// decoder installs v0 alone, so discovery through it answered
/// `UnsupportedRecordVersion` before any gate ran. Built from the v1 model's
/// own validated fixture rather than hand-written YAML, so the baseline
/// digests, the baseline manifest and every lifecycle invariant hold and
/// `decode_production` really accepts it.
pub(super) fn write_open_v1_record(root: &Path) -> String {
    let mut record = crate::workspace_ops::merge::test_v1_record();
    record.state = crate::workspace_ops::merge::OperationState::AwaitingResolution;
    let participant = record
        .participants
        .get_mut("mem_a")
        .expect("the v1 fixture owns one member participant");
    participant.state = crate::workspace_ops::merge::ParticipantState::Conflicted;
    participant.expected_merge_head = Some(participant.source_commit.clone());
    participant.conflict_paths = vec!["README.md".to_owned()];
    let merge_id = record.merge_id.clone();

    let directory = root.join(".gwz/merge");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(format!("{merge_id}.yaml")),
        serde_yaml::to_string(&record).unwrap(),
    )
    .unwrap();
    merge_id
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
