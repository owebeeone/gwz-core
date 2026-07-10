use std::collections::BTreeMap;

use crate::artifact::{
    self, ArtifactSourceKind, CreatedByArtifact, ResolvedMemberArtifact, SnapshotArtifact,
    read_lock, read_manifest, write_snapshot,
};
use crate::git::{Git2Backend, GitBackend};
use crate::model::ErrorCode;
use crate::operation::NullSink;

use super::*;

#[test]
fn repo_clone_member_registers_explicit_identity_in_existing_workspace() {
    let temp = TempDir::new("repo-clone-member");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let fixture = RemoteFixture::new("repo-clone-member-source");
    let commit = fixture.commit_and_push("README.md", "one", "initial", &backend);

    let response = handle_clone_repo_member(
        &backend,
        temp.path(),
        crate::CloneRepoMemberRequest {
            meta: request_meta(),
            source: crate::SourceUrl {
                url: fixture.remote_url().to_owned(),
                path: Some("libs/shared".to_owned()),
                remote_name: None,
                branch: None,
            },
            member_id: Some("mem_shared_v2".to_owned()),
            source_id: None,
        },
        "op_clone_member",
        &NullSink,
    )
    .unwrap();

    assert_eq!(
        response.response.meta.action,
        crate::ActionKind::CloneRepoMember
    );
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    let manifest = read_manifest(temp.path()).unwrap();
    let member = manifest
        .members
        .iter()
        .find(|member| member.id == "mem_shared_v2")
        .unwrap();
    assert!(member.active);
    assert_eq!(member.path, "libs/shared");
    assert_eq!(member.source_id, "src_shared_v2");
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_shared_v2"].commit,
        Some(commit)
    );
}

#[test]
fn detach_then_attach_preserves_identity_and_verifies_snapshot_commit() {
    let (temp, backend, commit) = workspace_with_member("detach-attach");
    write_member_snapshot(temp.path(), "mem_shared", "libs/shared", &commit);

    let detached = handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();
    assert_eq!(
        detached.response.meta.action,
        crate::ActionKind::DetachRepoMember
    );
    assert!(!read_manifest(temp.path()).unwrap().members[0].active);
    assert!(
        !read_lock(temp.path())
            .unwrap()
            .members
            .contains_key("mem_shared")
    );

    let attached = handle_attach_repo_member(
        &backend,
        temp.path(),
        crate::AttachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_attach",
        &NullSink,
    )
    .unwrap();
    assert_eq!(
        attached.response.meta.action,
        crate::ActionKind::AttachRepoMember
    );
    assert!(read_manifest(temp.path()).unwrap().members[0].active);
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_shared"].commit,
        Some(commit)
    );
}

#[test]
fn attach_rejects_missing_historical_commit_without_mutating() {
    let (temp, backend, _) = workspace_with_member("attach-mismatch");
    write_member_snapshot(temp.path(), "mem_shared", "libs/shared", &"f".repeat(40));
    handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();

    let error = handle_attach_repo_member(
        &backend,
        temp.path(),
        crate::AttachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_attach",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SourceIdentityMismatch);
    assert!(!read_manifest(temp.path()).unwrap().members[0].active);
    assert!(
        !read_lock(temp.path())
            .unwrap()
            .members
            .contains_key("mem_shared")
    );
}

#[test]
fn bare_repo_add_reattaches_unique_verified_historical_member() {
    let (temp, backend, commit) = workspace_with_member("add-reattach");
    write_member_snapshot(temp.path(), "mem_shared", "libs/shared", &commit);
    handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();

    let response = handle_add_existing_repo(
        &backend,
        temp.path(),
        crate::AddExistingRepoRequest {
            meta: request_meta(),
            repository_path: temp
                .path()
                .join("libs/shared")
                .to_string_lossy()
                .into_owned(),
            member_path: None,
            member_id: None,
            source_id: None,
        },
        "op_add_again",
    )
    .unwrap();

    assert_eq!(response.response.members.single().member_id, "mem_shared");
    let manifest = read_manifest(temp.path()).unwrap();
    assert_eq!(manifest.members.len(), 1);
    assert!(manifest.members[0].active);
    assert!(
        response
            .response
            .meta
            .message
            .as_deref()
            .is_some_and(|message| message.contains("reattached mem_shared"))
    );
}

#[test]
fn detach_rejects_implicit_default_even_for_one_member() {
    let (temp, backend, _) = workspace_with_member("detach-default");
    let error = handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: request_meta(),
        },
        "op_detach",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(read_manifest(temp.path()).unwrap().members[0].active);
}

#[test]
fn explicit_attach_with_empty_evidence_succeeds_with_frozen_warning_event() {
    let (temp, backend, _) = workspace_with_member("attach-empty-evidence");
    handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();
    let events = CollectingSink::default();

    let response = handle_attach_repo_member(
        &backend,
        temp.path(),
        crate::AttachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_attach",
        &events,
    )
    .unwrap();

    let warning = "attached mem_shared; no snapshot or marker commit evidence was available to verify repository identity";
    assert_eq!(response.response.meta.message.as_deref(), Some(warning));
    assert!(events.take().iter().any(|event| {
        event.severity == crate::Severity::Warn && event.message.as_deref() == Some(warning)
    }));
    assert!(read_manifest(temp.path()).unwrap().members[0].active);
}

#[test]
fn bare_add_cannot_infer_reattach_from_empty_evidence() {
    let (temp, backend, _) = workspace_with_member("add-empty-evidence");
    handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();

    let error = handle_add_existing_repo(
        &backend,
        temp.path(),
        crate::AddExistingRepoRequest {
            meta: request_meta(),
            repository_path: temp
                .path()
                .join("libs/shared")
                .to_string_lossy()
                .into_owned(),
            member_path: None,
            member_id: None,
            source_id: None,
        },
        "op_add_again",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("no historical commit evidence"));
    assert!(!read_manifest(temp.path()).unwrap().members[0].active);
}

#[test]
fn detach_dry_run_plans_without_changing_manifest_or_lock() {
    let (temp, backend, commit) = workspace_with_member("detach-dry-run");

    let response = handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", true),
        },
        "op_detach",
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Accepted
    );
    assert_eq!(
        response
            .response
            .members
            .single()
            .planned
            .as_ref()
            .unwrap()
            .action,
        crate::PlannedAction::DetachMember
    );
    assert!(read_manifest(temp.path()).unwrap().members[0].active);
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_shared"].commit,
        Some(commit)
    );
}

#[test]
fn attach_rejects_a_path_selector_even_when_unambiguous() {
    let (temp, backend, _) = workspace_with_member("attach-path-selector");
    let error = handle_attach_repo_member(
        &backend,
        temp.path(),
        crate::AttachRepoMemberRequest {
            meta: selected_meta("libs/shared", false),
        },
        "op_attach",
        &NullSink,
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("member id"));
}

#[test]
fn explicit_source_reuse_rejects_missing_history_without_new_designation() {
    let (temp, backend, _) = workspace_with_member("add-source-mismatch");
    write_member_snapshot(temp.path(), "mem_shared", "libs/shared", &"f".repeat(40));
    handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();

    let error = handle_add_existing_repo(
        &backend,
        temp.path(),
        crate::AddExistingRepoRequest {
            meta: request_meta(),
            repository_path: temp
                .path()
                .join("libs/shared")
                .to_string_lossy()
                .into_owned(),
            member_path: None,
            member_id: Some("mem_shared_v2".to_owned()),
            source_id: Some("src_shared".to_owned()),
        },
        "op_add_new",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::SourceIdentityMismatch);
    assert_eq!(read_manifest(temp.path()).unwrap().members.len(), 1);
}

#[test]
fn clone_source_reuse_mismatch_removes_fresh_checkout_and_artifacts() {
    let (temp, backend, _) = workspace_with_member("clone-source-mismatch");
    write_member_snapshot(temp.path(), "mem_shared", "libs/shared", &"f".repeat(40));
    let fixture = RemoteFixture::new("clone-source-mismatch-fixture");
    fixture.commit_and_push("README.md", "one", "initial", &backend);
    let target = temp.path().join("libs/replacement");

    let error = handle_clone_repo_member(
        &backend,
        temp.path(),
        crate::CloneRepoMemberRequest {
            meta: request_meta(),
            source: crate::SourceUrl {
                url: fixture.remote_url().to_owned(),
                path: Some("libs/replacement".to_owned()),
                remote_name: None,
                branch: None,
            },
            member_id: Some("mem_replacement".to_owned()),
            source_id: Some("src_shared".to_owned()),
        },
        "op_clone_reuse",
        &NullSink,
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::SourceIdentityMismatch);
    assert!(!target.exists());
    assert_eq!(read_manifest(temp.path()).unwrap().members.len(), 1);
    assert!(
        !read_lock(temp.path())
            .unwrap()
            .members
            .contains_key("mem_replacement")
    );
}

#[test]
fn repo_create_defaults_source_id_from_final_explicit_member_id() {
    let temp = TempDir::new("create-default-source");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    handle_create_repo(
        &backend,
        temp.path(),
        crate::CreateRepoRequest {
            meta: request_meta(),
            member_path: "libs/shared".to_owned(),
            initial_branch: None,
            member_id: Some("mem_shared_v2".to_owned()),
            source_id: None,
        },
        "op_create_member",
    )
    .unwrap();

    assert_eq!(
        read_manifest(temp.path()).unwrap().members[0].source_id,
        "src_shared_v2"
    );
}

#[test]
fn bare_add_refuses_multiple_fully_verified_historical_designations() {
    let (temp, backend, commit) = workspace_with_member("add-multiple-matches");
    write_member_snapshot(temp.path(), "mem_shared", "libs/shared", &commit);
    handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();
    let mut manifest = read_manifest(temp.path()).unwrap();
    let mut alias = manifest.members[0].clone();
    alias.id = "mem_shared_alias".to_owned();
    alias.source_id = "src_shared_alias".to_owned();
    manifest.members.push(alias);
    artifact::write_manifest(temp.path(), &manifest).unwrap();
    let mut snapshot = artifact::read_snapshot(temp.path(), "identity_evidence").unwrap();
    let mut alias_state = snapshot.members["mem_shared"].clone();
    alias_state.source_id = Some("src_shared_alias".to_owned());
    snapshot
        .members
        .insert("mem_shared_alias".to_owned(), alias_state);
    snapshot
        .selected_members
        .push("mem_shared_alias".to_owned());
    write_snapshot(temp.path(), &snapshot).unwrap();

    let error = handle_add_existing_repo(
        &backend,
        temp.path(),
        crate::AddExistingRepoRequest {
            meta: request_meta(),
            repository_path: temp
                .path()
                .join("libs/shared")
                .to_string_lossy()
                .into_owned(),
            member_path: None,
            member_id: None,
            source_id: None,
        },
        "op_add_again",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("mem_shared"));
    assert!(error.message.contains("mem_shared_alias"));
    assert!(
        read_manifest(temp.path())
            .unwrap()
            .members
            .iter()
            .all(|member| !member.active)
    );
}

#[test]
fn explicit_attach_requires_the_historical_checkout_to_exist() {
    let (temp, backend, commit) = workspace_with_member("attach-missing-checkout");
    write_member_snapshot(temp.path(), "mem_shared", "libs/shared", &commit);
    handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();
    std::fs::remove_dir_all(temp.path().join("libs/shared")).unwrap();

    let error = handle_attach_repo_member(
        &backend,
        temp.path(),
        crate::AttachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_attach",
        &NullSink,
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MemberNotFound);
    assert!(error.message.contains("restore it"));
    assert!(!read_manifest(temp.path()).unwrap().members[0].active);
}

#[test]
fn attach_of_an_already_active_member_is_a_noop() {
    let (temp, backend, commit) = workspace_with_member("attach-active-noop");

    let response = handle_attach_repo_member(
        &backend,
        temp.path(),
        crate::AttachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_attach",
        &NullSink,
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Noop
    );
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_shared"].commit,
        Some(commit)
    );
}

#[test]
fn explicit_new_designation_can_reuse_a_verified_source_identity() {
    let (temp, backend, commit) = workspace_with_member("add-verified-source-reuse");
    write_member_snapshot(temp.path(), "mem_shared", "libs/shared", &commit);
    handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();

    let response = handle_add_existing_repo(
        &backend,
        temp.path(),
        crate::AddExistingRepoRequest {
            meta: request_meta(),
            repository_path: temp
                .path()
                .join("libs/shared")
                .to_string_lossy()
                .into_owned(),
            member_path: None,
            member_id: Some("mem_shared_v2".to_owned()),
            source_id: Some("src_shared".to_owned()),
        },
        "op_add_new",
    )
    .unwrap();

    assert_eq!(
        response.response.members.single().member_id,
        "mem_shared_v2"
    );
    let manifest = read_manifest(temp.path()).unwrap();
    assert_eq!(manifest.members.len(), 2);
    assert!(!manifest.members[0].active);
    assert!(manifest.members[1].active);
    assert_eq!(manifest.members[1].source_id, "src_shared");
}

#[test]
fn replacement_clone_at_detached_path_requires_and_uses_a_new_member_id() {
    let (temp, backend, _) = workspace_with_member("clone-replacement");
    handle_detach_repo_member(
        &backend,
        temp.path(),
        crate::DetachRepoMemberRequest {
            meta: selected_meta("mem_shared", false),
        },
        "op_detach",
    )
    .unwrap();
    std::fs::remove_dir_all(temp.path().join("libs/shared")).unwrap();
    let fixture = RemoteFixture::new("clone-replacement-fixture");
    fixture.commit_and_push("README.md", "replacement", "initial", &backend);

    let implicit_error = handle_clone_repo_member(
        &backend,
        temp.path(),
        crate::CloneRepoMemberRequest {
            meta: request_meta(),
            source: crate::SourceUrl {
                url: fixture.remote_url().to_owned(),
                path: Some("libs/shared".to_owned()),
                remote_name: None,
                branch: None,
            },
            member_id: None,
            source_id: None,
        },
        "op_clone_implicit",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(implicit_error.code, ErrorCode::InvalidRequest);

    handle_clone_repo_member(
        &backend,
        temp.path(),
        crate::CloneRepoMemberRequest {
            meta: request_meta(),
            source: crate::SourceUrl {
                url: fixture.remote_url().to_owned(),
                path: Some("libs/shared".to_owned()),
                remote_name: None,
                branch: None,
            },
            member_id: Some("mem_replacement".to_owned()),
            source_id: None,
        },
        "op_clone_replacement",
        &NullSink,
    )
    .unwrap();

    let manifest = read_manifest(temp.path()).unwrap();
    assert_eq!(manifest.members.len(), 2);
    assert_eq!(manifest.members[0].id, "mem_shared");
    assert!(!manifest.members[0].active);
    assert_eq!(manifest.members[1].id, "mem_replacement");
    assert!(manifest.members[1].active);
    assert_eq!(manifest.members[1].source_id, "src_replacement");
}

fn workspace_with_member(name: &str) -> (TempDir, Git2Backend, String) {
    let temp = TempDir::new(name);
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let repo = temp.path().join("libs/shared");
    backend.create_repo(&repo).unwrap();
    let commit = commit_file(&repo, "README.md", "one", "initial", &[]).unwrap();
    handle_add_existing_repo(
        &backend,
        temp.path(),
        crate::AddExistingRepoRequest {
            meta: request_meta(),
            repository_path: repo.to_string_lossy().into_owned(),
            member_path: None,
            member_id: Some("mem_shared".to_owned()),
            source_id: Some("src_shared".to_owned()),
        },
        "op_add",
    )
    .unwrap();
    (temp, backend, commit)
}

fn selected_meta(target: &str, dry_run: bool) -> crate::RequestMeta {
    crate::RequestMeta {
        selection: Some(crate::Selection {
            targets: vec![target.to_owned()],
            ..Default::default()
        }),
        dry_run: dry_run.then_some(true),
        ..request_meta()
    }
}

fn write_member_snapshot(root: &std::path::Path, member_id: &str, path: &str, commit: &str) {
    write_snapshot(
        root,
        &SnapshotArtifact {
            schema: artifact::SNAPSHOT_SCHEMA.to_owned(),
            workspace_id: "ws_ops".to_owned(),
            snapshot_id: "identity_evidence".to_owned(),
            created_at: "2026-07-10T00:00:00Z".to_owned(),
            created_by: CreatedByArtifact {
                actor_id: "test".to_owned(),
            },
            selected_members: vec![member_id.to_owned()],
            members: BTreeMap::from([(
                member_id.to_owned(),
                ResolvedMemberArtifact {
                    path: path.to_owned(),
                    source_id: Some("src_shared".to_owned()),
                    source_kind: ArtifactSourceKind::Git,
                    commit: Some(commit.to_owned()),
                    branch: Some("main".to_owned()),
                    detached: Some(false),
                    upstream: None,
                    dirty: Some(false),
                    materialized: Some(true),
                },
            )]),
        },
    )
    .unwrap();
}
