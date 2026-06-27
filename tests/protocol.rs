use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use gwz_core::{
    ActionKind, AggregateStatus, BranchActionResult, BranchOp, BranchRepoSummary, BranchRequest,
    BranchResponse, EventKind, GitBranchDifference, GitBranchGroup, GitFileChange,
    GitMemberBranchStatus, GitObjectIdentity, GwzError, GwzErrorCode, ListSnapshotsResponse,
    MaterializeRequest, MaterializeTarget, MaterializeTargetKind, MemberResponse, MemberStatus,
    OperationActor, OperationAttribution, OperationEvent, RepoSyncRequest, RepoSyncResponse,
    RequestMeta, ResponseEnvelope, ResponseMeta, Severity, SnapshotInfo, SnapshotRequest,
    SnapshotSource, SnapshotSourceKind, SourceKind, StashBundle, StashBundleMember,
    StashDirtySummary, StashDrift, StashErrorDetail, StashOp, StashParticipation,
    StashPushLifecycle, StashRequest, StashResponse, StashRestoreState, StashWarning, StatusMode,
    StatusPathStyle, StatusRequest, StatusResponse, WorkspaceGitStatus, WorkspaceRootFileChange,
    WorkspaceRootGitStatus, decode, encode,
};

fn round_trip<T>(
    value: &T,
    to_cbor: impl Fn(&T) -> gwz_core::Cbor,
    from_cbor: impl Fn(&gwz_core::Cbor) -> T,
) -> T {
    from_cbor(&decode(&encode(&to_cbor(value))))
}

#[test]
fn status_request_round_trips() {
    let request = StatusRequest {
        meta: RequestMeta {
            request_id: "req-1".to_owned(),
            schema_version: "gwz.v0".to_owned(),
            attribution: Some(attribution()),
            ..RequestMeta::default()
        },
        mode: Some(StatusMode::Combined),
        include_file_changes: Some(true),
        include_branch_summary: Some(true),
        path_style: Some(StatusPathStyle::WorkspaceRelative),
    };

    assert_eq!(
        round_trip(&request, StatusRequest::to_cbor, StatusRequest::from_cbor),
        request
    );
}

#[test]
fn repo_sync_request_and_response_round_trip() {
    let request = RepoSyncRequest {
        meta: RequestMeta {
            request_id: "req-sync".to_owned(),
            schema_version: "gwz.v0".to_owned(),
            ..RequestMeta::default()
        },
    };
    assert_eq!(
        round_trip(
            &request,
            RepoSyncRequest::to_cbor,
            RepoSyncRequest::from_cbor
        ),
        request
    );

    let response = RepoSyncResponse {
        response: ResponseEnvelope {
            meta: ResponseMeta {
                request_id: "req-sync".to_owned(),
                schema_version: "gwz.v0".to_owned(),
                action: ActionKind::RepoSync,
                aggregate_status: AggregateStatus::Ok,
                ..ResponseMeta::default()
            },
            members: Vec::new(),
            errors: Vec::new(),
        },
    };
    assert_eq!(
        round_trip(
            &response,
            RepoSyncResponse::to_cbor,
            RepoSyncResponse::from_cbor
        ),
        response
    );
}

#[test]
fn stash_requests_round_trip_for_all_s3_ops() {
    let requests = vec![
        StashRequest {
            meta: request_meta("req-stash-push"),
            op: StashOp::Push,
            stash_id: None,
            message: Some("save protocol work".to_owned()),
            include_untracked: Some(true),
            include_ignored: Some(false),
            expanded: None,
            preserve_index: None,
        },
        StashRequest {
            meta: request_meta("req-stash-list"),
            op: StashOp::List,
            stash_id: None,
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: Some(true),
            preserve_index: None,
        },
        StashRequest {
            meta: request_meta("req-stash-apply"),
            op: StashOp::Apply,
            stash_id: Some("stash_20260625_000001".to_owned()),
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: None,
            preserve_index: Some(true),
        },
        StashRequest {
            meta: request_meta("req-stash-pop"),
            op: StashOp::Pop,
            stash_id: Some("stash_20260625_000001".to_owned()),
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: None,
            preserve_index: Some(true),
        },
        StashRequest {
            meta: request_meta("req-stash-drop"),
            op: StashOp::Drop,
            stash_id: Some("stash_20260625_000001".to_owned()),
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: None,
            preserve_index: None,
        },
    ];

    for request in requests {
        assert_eq!(
            round_trip(&request, StashRequest::to_cbor, StashRequest::from_cbor),
            request
        );
    }
}

#[test]
fn stash_response_round_trips_bundle_projection() {
    let response = StashResponse {
        response: response_envelope("req-stash-list", ActionKind::Stash),
        bundles: Some(vec![stash_bundle()]),
    };

    assert_eq!(
        round_trip(&response, StashResponse::to_cbor, StashResponse::from_cbor),
        response
    );
}

#[test]
fn branch_requests_round_trip_for_b4a_ops() {
    let requests = vec![
        BranchRequest {
            meta: request_meta("req-branch-list"),
            op: BranchOp::List,
            name: None,
            start_ref: None,
            switch_after_create: None,
        },
        BranchRequest {
            meta: request_meta("req-branch-create"),
            op: BranchOp::Create,
            name: Some("feature/protocol".to_owned()),
            start_ref: Some("HEAD".to_owned()),
            switch_after_create: Some(true),
        },
        BranchRequest {
            meta: request_meta("req-branch-delete"),
            op: BranchOp::Delete,
            name: Some("feature/protocol".to_owned()),
            start_ref: None,
            switch_after_create: None,
        },
    ];

    for request in requests {
        assert_eq!(
            round_trip(&request, BranchRequest::to_cbor, BranchRequest::from_cbor),
            request
        );
    }
}

#[test]
fn branch_response_round_trips_repo_summary() {
    let response = BranchResponse {
        response: response_envelope("req-branch-list", ActionKind::Branch),
        repos: Some(vec![
            branch_repo_summary("mem_core", BranchActionResult::Listed),
            branch_repo_summary("mem_cli", BranchActionResult::Created),
            branch_repo_summary("mem_docs", BranchActionResult::Deleted),
        ]),
    };

    assert_eq!(
        round_trip(
            &response,
            BranchResponse::to_cbor,
            BranchResponse::from_cbor
        ),
        response
    );
}

#[test]
fn branch_response_round_trips_clean_merge_summary() {
    let response = BranchResponse {
        response: response_envelope("req-branch-merge-clean", ActionKind::Branch),
        repos: Some(vec![BranchRepoSummary {
            result: BranchActionResult::Merged,
            source_ref: Some("feature/protocol".to_owned()),
            target_branch: Some("main".to_owned()),
            resulting_commit: Some("2222222222222222222222222222222222222222".to_owned()),
            conflict_paths: Vec::new(),
            ..branch_repo_summary("mem_core", BranchActionResult::Merged)
        }]),
    };

    assert_eq!(
        round_trip(
            &response,
            BranchResponse::to_cbor,
            BranchResponse::from_cbor
        ),
        response
    );
}

#[test]
fn branch_response_round_trips_conflicted_merge_summary() {
    let mut envelope = response_envelope("req-branch-merge-conflict", ActionKind::Branch);
    envelope.meta.aggregate_status = AggregateStatus::Conflicted;

    let response = BranchResponse {
        response: envelope,
        repos: Some(vec![
            BranchRepoSummary {
                result: BranchActionResult::Merged,
                source_ref: Some("feature/protocol".to_owned()),
                target_branch: Some("main".to_owned()),
                resulting_commit: Some("2222222222222222222222222222222222222222".to_owned()),
                conflict_paths: Vec::new(),
                ..branch_repo_summary("mem_core", BranchActionResult::Merged)
            },
            BranchRepoSummary {
                result: BranchActionResult::Conflicted,
                source_ref: Some("feature/protocol".to_owned()),
                target_branch: Some("main".to_owned()),
                resulting_commit: None,
                conflict_paths: vec!["src/lib.rs".to_owned(), "Cargo.toml".to_owned()],
                ..branch_repo_summary("mem_cli", BranchActionResult::Conflicted)
            },
        ]),
    };

    assert_eq!(
        round_trip(
            &response,
            BranchResponse::to_cbor,
            BranchResponse::from_cbor
        ),
        response
    );
}

#[test]
fn branch_materialize_target_round_trips() {
    let request = MaterializeRequest {
        meta: request_meta("req-materialize-branch"),
        target: MaterializeTarget {
            kind: MaterializeTargetKind::Branch,
            name: Some("feature/protocol".to_owned()),
            commit: None,
        },
    };

    assert_eq!(
        round_trip(
            &request,
            MaterializeRequest::to_cbor,
            MaterializeRequest::from_cbor
        ),
        request
    );
}

#[test]
fn snapshot_sources_round_trip() {
    let current = SnapshotRequest {
        meta: request_meta("req-snapshot-current"),
        snapshot_id: "snap-current".to_owned(),
        source: Some(SnapshotSource {
            kind: SnapshotSourceKind::Current,
            branch: None,
        }),
    };
    assert_eq!(
        round_trip(
            &current,
            SnapshotRequest::to_cbor,
            SnapshotRequest::from_cbor
        ),
        current
    );

    let branch = SnapshotRequest {
        meta: request_meta("req-snapshot-branch"),
        snapshot_id: "snap-feature".to_owned(),
        source: Some(SnapshotSource {
            kind: SnapshotSourceKind::Branch,
            branch: Some("feature/protocol".to_owned()),
        }),
    };
    assert_eq!(
        round_trip(
            &branch,
            SnapshotRequest::to_cbor,
            SnapshotRequest::from_cbor
        ),
        branch
    );
}

#[test]
fn list_snapshots_response_round_trips() {
    let response = ListSnapshotsResponse {
        response: response_envelope("req-list-snapshots", ActionKind::ListSnapshots),
        snapshots: Some(vec![SnapshotInfo {
            name: "snap-one".to_owned(),
            created_at: "2026-06-28T00:00:00Z".to_owned(),
            created_by: "tester".to_owned(),
            members: 2,
        }]),
    };

    assert_eq!(
        round_trip(
            &response,
            ListSnapshotsResponse::to_cbor,
            ListSnapshotsResponse::from_cbor
        ),
        response
    );
}

#[test]
fn status_response_round_trips_combined_workspace_status() {
    let response = StatusResponse {
        response: ResponseEnvelope {
            meta: ResponseMeta {
                request_id: "req-1".to_owned(),
                schema_version: "gwz.v0".to_owned(),
                action: ActionKind::Status,
                aggregate_status: AggregateStatus::Ok,
                ..ResponseMeta::default()
            },
            members: Vec::new(),
            errors: Vec::new(),
        },
        workspace_git_status: Some(WorkspaceGitStatus {
            clean: false,
            root_status: Some(WorkspaceRootGitStatus {
                branch: Some("main".to_owned()),
                detached: false,
                head: Some("def456".to_owned()),
                staged: 2,
                unstaged: 0,
                untracked: 0,
                dirty: true,
                unborn: false,
            }),
            root_file_changes: vec![WorkspaceRootFileChange {
                repo_path: "gwz.conf/gwz.yml".to_owned(),
                workspace_path: "gwz.conf/gwz.yml".to_owned(),
                index_status: "A".to_owned(),
                worktree_status: " ".to_owned(),
                original_repo_path: None,
            }],
            file_changes: vec![GitFileChange {
                member_id: "mem_core".to_owned(),
                member_path: "repos/core".to_owned(),
                repo_path: "src/lib.rs".to_owned(),
                workspace_path: "repos/core/src/lib.rs".to_owned(),
                index_status: " ".to_owned(),
                worktree_status: "M".to_owned(),
                original_repo_path: None,
            }],
            branches: vec![GitMemberBranchStatus {
                member_id: "mem_core".to_owned(),
                member_path: "repos/core".to_owned(),
                label: "main".to_owned(),
                branch: Some("main".to_owned()),
                detached: false,
                unborn: false,
                head: Some("abc123".to_owned()),
                upstream: Some("origin/main".to_owned()),
                ahead: Some(1),
                behind: Some(0),
            }],
            branch_groups: vec![GitBranchGroup {
                label: "main".to_owned(),
                member_ids: vec!["mem_core".to_owned()],
                member_paths: vec!["repos/core".to_owned()],
            }],
            branch_differences: vec![GitBranchDifference {
                label: "feature/app".to_owned(),
                majority_label: Some("main".to_owned()),
                member_ids: vec!["mem_app".to_owned()],
                member_paths: vec!["repos/app".to_owned()],
                message: Some("repos/app differs from majority branch main".to_owned()),
            }],
        }),
    };

    assert_eq!(
        round_trip(
            &response,
            StatusResponse::to_cbor,
            StatusResponse::from_cbor
        ),
        response
    );
}

#[test]
fn response_envelope_round_trips_with_member_error() {
    let response = ResponseEnvelope {
        meta: ResponseMeta {
            request_id: "req-1".to_owned(),
            schema_version: "gwz.v0".to_owned(),
            action: ActionKind::Status,
            aggregate_status: AggregateStatus::Rejected,
            message: Some("workspace has errors".to_owned()),
            attribution: Some(attribution()),
            ..ResponseMeta::default()
        },
        members: vec![MemberResponse {
            member_id: "core".to_owned(),
            member_path: "libs/core".to_owned(),
            source_kind: SourceKind::Git,
            status: MemberStatus::Rejected,
            error: Some(member_error()),
            ..MemberResponse::default()
        }],
        errors: vec![member_error()],
    };

    assert_eq!(
        round_trip(
            &response,
            ResponseEnvelope::to_cbor,
            ResponseEnvelope::from_cbor
        ),
        response
    );
}

#[test]
fn operation_event_round_trips_with_attribution() {
    let event = OperationEvent {
        operation_id: "op-1".to_owned(),
        request_id: "req-1".to_owned(),
        sequence: 42,
        timestamp_ms: 1_727_000_000_000,
        kind: EventKind::MemberFinished,
        severity: Severity::Warn,
        member_id: Some("core".to_owned()),
        member_path: Some("libs/core".to_owned()),
        message: Some("member rejected".to_owned()),
        error: Some(member_error()),
        attribution: Some(attribution()),
        ..OperationEvent::default()
    };

    assert_eq!(
        round_trip(&event, OperationEvent::to_cbor, OperationEvent::from_cbor),
        event
    );
}

#[test]
fn attribution_round_trips_actor_and_git_identities_separately() {
    let attribution = attribution();

    assert_eq!(
        round_trip(
            &attribution,
            OperationAttribution::to_cbor,
            OperationAttribution::from_cbor
        ),
        attribution
    );
}

#[test]
fn error_code_wire_values_are_pinned() {
    assert_eq!(GwzErrorCode::Ok.wire(), 0);
    assert_eq!(GwzErrorCode::InvalidRequest.wire(), 1);
    assert_eq!(GwzErrorCode::DivergedMember.wire(), 16);
    assert_eq!(GwzErrorCode::AttributionDenied.wire(), 26);
    assert_eq!(GwzErrorCode::InternalError.wire(), 29);
    assert_eq!(GwzErrorCode::BranchDetachedHead.wire(), 30);
    assert_eq!(GwzErrorCode::BranchUnbornHead.wire(), 31);
    assert_eq!(GwzErrorCode::BranchMixed.wire(), 32);
    assert_eq!(GwzErrorCode::StashNotFound.wire(), 33);
    assert_eq!(GwzErrorCode::StashIncomplete.wire(), 34);
    assert_eq!(GwzErrorCode::StashConflict.wire(), 35);
}

#[test]
fn branch_protocol_wire_values_are_pinned() {
    assert_eq!(ActionKind::Stash.wire(), 17);
    assert_eq!(ActionKind::Branch.wire(), 18);
    assert_eq!(ActionKind::ListSnapshots.wire(), 20);
    assert_eq!(MaterializeTargetKind::Lock.wire(), 0);
    assert_eq!(MaterializeTargetKind::Commit.wire(), 4);
    assert_eq!(MaterializeTargetKind::Branch.wire(), 5);
    assert_eq!(SnapshotSourceKind::Current.wire(), 0);
    assert_eq!(SnapshotSourceKind::Branch.wire(), 1);
    assert_eq!(BranchOp::List.wire(), 0);
    assert_eq!(BranchOp::Create.wire(), 1);
    assert_eq!(BranchOp::Delete.wire(), 2);
    assert_eq!(BranchOp::Merge.wire(), 3);
    assert_eq!(BranchActionResult::Listed.wire(), 0);
    assert_eq!(BranchActionResult::Created.wire(), 1);
    assert_eq!(BranchActionResult::Exists.wire(), 2);
    assert_eq!(BranchActionResult::Deleted.wire(), 3);
    assert_eq!(BranchActionResult::Switched.wire(), 4);
    assert_eq!(BranchActionResult::Noop.wire(), 5);
    assert_eq!(BranchActionResult::Skipped.wire(), 6);
    assert_eq!(BranchActionResult::Merged.wire(), 7);
    assert_eq!(BranchActionResult::Conflicted.wire(), 8);
}

#[test]
fn stash_protocol_wire_values_are_pinned() {
    assert_eq!(StashOp::Push.wire(), 0);
    assert_eq!(StashOp::List.wire(), 1);
    assert_eq!(StashOp::Apply.wire(), 2);
    assert_eq!(StashOp::Pop.wire(), 3);
    assert_eq!(StashOp::Drop.wire(), 4);
    assert_eq!(StashParticipation::Stashed.wire(), 0);
    assert_eq!(StashParticipation::Empty.wire(), 1);
    assert_eq!(StashParticipation::Skipped.wire(), 2);
    assert_eq!(StashPushLifecycle::Unattempted.wire(), 0);
    assert_eq!(StashPushLifecycle::Saving.wire(), 1);
    assert_eq!(StashPushLifecycle::Saved.wire(), 2);
    assert_eq!(StashPushLifecycle::Empty.wire(), 3);
    assert_eq!(StashPushLifecycle::Failed.wire(), 4);
    assert_eq!(StashRestoreState::Pending.wire(), 0);
    assert_eq!(StashRestoreState::Applied.wire(), 1);
    assert_eq!(StashRestoreState::Popped.wire(), 2);
    assert_eq!(StashRestoreState::Dropped.wire(), 3);
    assert_eq!(StashRestoreState::Noop.wire(), 4);
    assert_eq!(StashRestoreState::Missing.wire(), 5);
}

#[test]
fn generated_protocol_is_current() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = std::env::temp_dir().join(format!("gwz-taut-gen-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out_dir);

    let status = taut_command(&root)
        .args([
            "gen",
            "protocol/gwz.taut.py",
            "-o",
            out_dir.to_str().expect("temp path is not utf-8"),
            "-l",
            "rust",
            "--api-only",
            "--with-runtime",
        ])
        .status()
        .expect("failed to run taut generator");
    assert!(status.success(), "taut generator failed");

    assert_same(
        &root.join("src/protocol/generated.rs"),
        &out_dir.join("rust/api.rs"),
    );
    assert_same(&root.join("src/cbor.rs"), &out_dir.join("rust/cbor.rs"));

    let status = taut_command(&root)
        .args([
            "corpus",
            "protocol/gwz.taut.py",
            "-o",
            "protocol/corpus",
            "-l",
            "rust",
            "--check",
        ])
        .status()
        .expect("failed to run taut corpus check");
    assert!(status.success(), "taut corpus is stale");

    fs::remove_dir_all(&out_dir).expect("failed to clean generated protocol temp dir");
}

#[test]
fn protocol_schema_uses_keyword_message_dsl() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let schema = fs::read_to_string(root.join("protocol/gwz.taut.py"))
        .expect("failed to read protocol/gwz.taut.py");

    assert!(
        !schema.contains("Msg(\""),
        "protocol/gwz.taut.py should name messages with schema keywords"
    );
    assert!(
        !schema.contains("Enum(\""),
        "protocol/gwz.taut.py should name enums with schema keywords"
    );
    assert!(
        !schema.contains("F(\""),
        "protocol/gwz.taut.py should name fields with Msg keywords"
    );
    assert!(
        !schema.contains("Ref(\""),
        "protocol/gwz.taut.py should reference messages and enums with Ref attributes"
    );
    assert!(
        !schema.contains("params=[("),
        "protocol/gwz.taut.py should name method params with Params keywords"
    );
}

#[test]
fn taut_command_can_use_configured_python_executable() {
    let command = taut_command_for_python(Path::new("/tmp/gwz-core"), "python");

    assert_eq!(command.get_program().to_string_lossy(), "python");
}

#[test]
fn taut_command_forces_utf8_for_generated_source_files() {
    let command = taut_command_for_python(Path::new("/tmp/gwz-core"), "python");

    assert_command_env(&command, "PYTHONUTF8", "1");
    assert_command_env(&command, "PYTHONIOENCODING", "utf-8");
    assert_command_env(&command, "SETUPTOOLS_SCM_PRETEND_VERSION", "0.6.0");
}

fn attribution() -> OperationAttribution {
    OperationAttribution {
        actor: Some(OperationActor {
            actor_id: "agent://gryth/dev".to_owned(),
            display_name: Some("Gryth Agent".to_owned()),
            email: Some("agent@example.invalid".to_owned()),
            authority: Some("local-test".to_owned()),
        }),
        git_author: Some(GitObjectIdentity {
            name: "AI Agent".to_owned(),
            email: "agent@example.invalid".to_owned(),
            time_ms: Some(1_727_000_000_000),
            timezone_offset_minutes: Some(600),
        }),
        git_committer: Some(GitObjectIdentity {
            name: "Workspace Bot".to_owned(),
            email: "workspace@example.invalid".to_owned(),
            time_ms: Some(1_727_000_000_100),
            timezone_offset_minutes: Some(600),
        }),
        credential_ref: Some("cred:test".to_owned()),
    }
}

fn request_meta(request_id: &str) -> RequestMeta {
    RequestMeta {
        request_id: request_id.to_owned(),
        schema_version: "gwz.v0".to_owned(),
        ..RequestMeta::default()
    }
}

fn response_envelope(request_id: &str, action: ActionKind) -> ResponseEnvelope {
    ResponseEnvelope {
        meta: ResponseMeta {
            request_id: request_id.to_owned(),
            schema_version: "gwz.v0".to_owned(),
            action,
            aggregate_status: AggregateStatus::Ok,
            ..ResponseMeta::default()
        },
        members: Vec::new(),
        errors: Vec::new(),
    }
}

fn stash_bundle() -> StashBundle {
    StashBundle {
        schema: "gwz.stash-bundle/v0".to_owned(),
        workspace_id: "ws_protocol".to_owned(),
        stash_id: "stash_20260625_000001".to_owned(),
        created_at: "2026-06-25T00:00:00Z".to_owned(),
        message_suffix: "save protocol work".to_owned(),
        include_untracked: true,
        include_ignored: false,
        selected_members: vec!["mem_core".to_owned(), "mem_cli".to_owned()],
        members: vec![
            StashBundleMember {
                member_id: "mem_core".to_owned(),
                path: "repos/core".to_owned(),
                participation: StashParticipation::Stashed,
                push_lifecycle: StashPushLifecycle::Saved,
                restore_state: StashRestoreState::Pending,
                branch_before: Some("main".to_owned()),
                head_before: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
                full_stash_message: "gwz:stash_20260625_000001: save protocol work".to_owned(),
                dirty_summary: StashDirtySummary {
                    staged: true,
                    unstaged: true,
                    untracked: true,
                    ignored: false,
                },
                native_stash_object_id: Some("fedcba9876543210fedcba9876543210fedcba98".to_owned()),
                native_stash_display_ref: Some("stash@{0}".to_owned()),
                error: None,
            },
            StashBundleMember {
                member_id: "mem_cli".to_owned(),
                path: "repos/cli".to_owned(),
                participation: StashParticipation::Stashed,
                push_lifecycle: StashPushLifecycle::Failed,
                restore_state: StashRestoreState::Missing,
                branch_before: Some("main".to_owned()),
                head_before: Some("1111111111111111111111111111111111111111".to_owned()),
                full_stash_message: "gwz:stash_20260625_000001: save protocol work".to_owned(),
                dirty_summary: StashDirtySummary {
                    staged: false,
                    unstaged: true,
                    untracked: false,
                    ignored: false,
                },
                native_stash_object_id: None,
                native_stash_display_ref: None,
                error: Some(StashErrorDetail {
                    code: "git_command_failed".to_owned(),
                    message: "native stash failed".to_owned(),
                }),
            },
        ],
        warnings: vec![StashWarning {
            code: "partial_push".to_owned(),
            message: "one member failed".to_owned(),
            member_id: Some("mem_cli".to_owned()),
        }],
        drift: vec![StashDrift {
            code: "missing_native_stash".to_owned(),
            message: "native stash payload is missing".to_owned(),
            member_id: "mem_cli".to_owned(),
        }],
    }
}

fn branch_repo_summary(member_id: &str, result: BranchActionResult) -> BranchRepoSummary {
    BranchRepoSummary {
        member_id: member_id.to_owned(),
        member_path: format!("repos/{member_id}"),
        source_kind: SourceKind::Git,
        result,
        branch: Some("feature/protocol".to_owned()),
        current_branch: Some("main".to_owned()),
        detached: false,
        unborn: false,
        head: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
        upstream: Some("origin/main".to_owned()),
        ahead: Some(1),
        behind: Some(0),
        source_ref: None,
        target_branch: None,
        resulting_commit: None,
        conflict_paths: Vec::new(),
    }
}

fn member_error() -> GwzError {
    GwzError {
        code: GwzErrorCode::DivergedMember,
        message: "member diverged".to_owned(),
        member_id: Some("core".to_owned()),
        member_path: Some("libs/core".to_owned()),
        target_kind: Some(gwz_core::TargetKind::Member),
        detail: Some("HEAD and upstream have distinct commits".to_owned()),
    }
}

fn taut_command(root: &Path) -> Command {
    let default_python = if cfg!(windows) { "python" } else { "python3" };
    let python = std::env::var("TAUT_PYTHON").unwrap_or_else(|_| default_python.to_owned());
    taut_command_for_python(root, &python)
}

fn taut_command_for_python(root: &Path, python: &str) -> Command {
    let mut command = Command::new(python);
    let taut_src = root
        .parent()
        .expect("gwz-core should have a parent")
        .join("taut/src");
    command
        .current_dir(root)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("SETUPTOOLS_SCM_PRETEND_VERSION", "0.6.0")
        .env("PYTHONPATH", taut_src)
        .args(["-m", "taut.cli"]);
    command
}

fn assert_command_env(command: &Command, key: &str, expected: &str) {
    let actual = command
        .get_envs()
        .find_map(|(name, value)| (name.to_string_lossy() == key).then_some(value))
        .flatten()
        .map(|value| value.to_string_lossy().into_owned());

    assert_eq!(actual.as_deref(), Some(expected));
}

fn assert_same(committed: &Path, generated: &Path) {
    let committed_text = fs::read_to_string(committed)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", committed.display()));
    let generated_text = fs::read_to_string(generated)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", generated.display()));
    assert_eq!(
        normalize_line_endings(&committed_text),
        normalize_line_endings(&generated_text),
        "{} is stale",
        committed.display()
    );
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
