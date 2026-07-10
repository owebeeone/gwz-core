use std::fs;
use std::path::Path;

use crate::git::{Git2Backend, GitBackend};

use super::*;

// P1.3: the `gwz add` handler routes pathspecs to their owning repos and stages there.

fn stage_request(cwd: &Path, pathspecs: &[&str], all: bool) -> crate::StageRequest {
    crate::StageRequest {
        meta: request_meta(),
        cwd: cwd.to_string_lossy().into_owned(),
        pathspecs: pathspecs.iter().map(|s| (*s).to_owned()).collect(),
        all: all.then_some(true),
    }
}

fn staged(backend: &Git2Backend, repo: &Path, path: &str) -> bool {
    backend
        .status(repo)
        .unwrap()
        .files
        .iter()
        .any(|file| file.path == path && file.index_status == "A")
}

#[test]
fn stages_pathspec_into_owning_member() {
    let temp = TempDir::new("stage-member");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stage-member-source");
    let member_root = temp.path().join("remote");
    fs::write(member_root.join("new.txt"), "x\n").unwrap();
    assert_eq!(backend.status(&member_root).unwrap().staged, 0);

    let response = handle_stage(
        &backend,
        temp.path(),
        stage_request(temp.path(), &["remote/new.txt"], false),
        "op_stage",
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert!(
        staged(&backend, &member_root, "new.txt"),
        "new.txt staged in the member"
    );
}

#[test]
fn stages_root_level_path_in_root_repo() {
    let temp = TempDir::new("stage-root");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stage-root-source");
    fs::write(temp.path().join("root.txt"), "y\n").unwrap();

    handle_stage(
        &backend,
        temp.path(),
        stage_request(temp.path(), &["root.txt"], false),
        "op_stage",
    )
    .unwrap();
    assert!(
        staged(&backend, temp.path(), "root.txt"),
        "root.txt staged in the root repo"
    );
}

#[test]
fn dot_at_root_stages_member_and_root() {
    let temp = TempDir::new("stage-dot");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stage-dot-source");
    let member_root = temp.path().join("remote");
    fs::write(member_root.join("a.txt"), "x\n").unwrap();
    fs::write(temp.path().join("root.txt"), "y\n").unwrap();

    handle_stage(
        &backend,
        temp.path(),
        stage_request(temp.path(), &["."], false),
        "op_stage",
    )
    .unwrap();
    assert!(
        staged(&backend, &member_root, "a.txt"),
        "member file staged"
    );
    assert!(
        staged(&backend, temp.path(), "root.txt"),
        "root file staged"
    );
}

#[test]
fn all_flag_stages_member_and_root() {
    let temp = TempDir::new("stage-all");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stage-all-source");
    let member_root = temp.path().join("remote");
    fs::write(member_root.join("a.txt"), "x\n").unwrap();
    fs::write(temp.path().join("root.txt"), "y\n").unwrap();

    handle_stage(
        &backend,
        temp.path(),
        stage_request(temp.path(), &[], true),
        "op_stage",
    )
    .unwrap();
    assert!(
        staged(&backend, &member_root, "a.txt"),
        "member file staged via -A"
    );
    assert!(
        staged(&backend, temp.path(), "root.txt"),
        "root file staged via -A"
    );
}

#[test]
fn pathspec_outside_workspace_errors() {
    let temp = TempDir::new("stage-escape");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stage-escape-source");

    let err = handle_stage(
        &backend,
        temp.path(),
        stage_request(temp.path(), &["../escape.txt"], false),
        "op_stage",
    )
    .unwrap_err();
    assert_eq!(err.code, crate::model::ErrorCode::PathEscape);
}

#[test]
fn all_with_member_selection_scopes_to_selected_member() {
    let temp = TempDir::new("stage-all-select");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stage-all-select-source");
    let member_root = temp.path().join("remote");
    fs::write(member_root.join("a.txt"), "x\n").unwrap();
    fs::write(temp.path().join("root.txt"), "y\n").unwrap();

    let member_id = crate::artifact::read_manifest(temp.path()).unwrap().members[0]
        .id
        .clone();
    let request = crate::StageRequest {
        meta: request_meta_with_actor_selection("agent_test", &[member_id.as_str()]),
        cwd: temp.path().to_string_lossy().into_owned(),
        pathspecs: Vec::new(),
        all: Some(true),
    };
    handle_stage(&backend, temp.path(), request, "op_stage").unwrap();

    assert!(
        staged(&backend, &member_root, "a.txt"),
        "selected member staged"
    );
    assert!(
        !staged(&backend, temp.path(), "root.txt"),
        "root NOT staged when scoped to a member"
    );
}

#[test]
fn dot_skips_unmaterialized_member_but_stages_root() {
    let temp = TempDir::new("stage-skip");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stage-skip-source");
    // Un-materialize the member: drop its .git so it is no longer a repo.
    fs::remove_dir_all(temp.path().join("remote/.git")).unwrap();
    fs::write(temp.path().join("root.txt"), "y\n").unwrap();

    // `gwz add .` reaches the member only by fan-out → it is skipped, not an error.
    handle_stage(
        &backend,
        temp.path(),
        stage_request(temp.path(), &["."], false),
        "op_stage",
    )
    .unwrap();
    assert!(
        staged(&backend, temp.path(), "root.txt"),
        "root still staged"
    );
}

#[test]
fn explicit_pathspec_into_unmaterialized_member_errors() {
    let temp = TempDir::new("stage-explicit-err");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stage-explicit-err-source");
    fs::remove_dir_all(temp.path().join("remote/.git")).unwrap();

    let err = handle_stage(
        &backend,
        temp.path(),
        stage_request(temp.path(), &["remote/x.txt"], false),
        "op_stage",
    )
    .unwrap_err();
    assert_eq!(err.code, crate::model::ErrorCode::MemberNotFound);
}

#[test]
fn inactive_nested_history_does_not_steal_stage_routing_from_active_owner() {
    let temp = TempDir::new("stage-inactive-routing");
    let backend = Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "stage-inactive-routing-source");
    let member_root = temp.path().join("remote");
    fs::create_dir_all(member_root.join("historical")).unwrap();
    fs::write(member_root.join("historical/new.txt"), "x\n").unwrap();

    let mut manifest = crate::artifact::read_manifest(temp.path()).unwrap();
    manifest.members.push(crate::artifact::ManifestMember {
        id: "mem_historical".to_owned(),
        path: "remote/historical".to_owned(),
        source_kind: crate::artifact::ArtifactSourceKind::Git,
        source_id: "src_historical".to_owned(),
        active: false,
        desired: None,
        remotes: Vec::new(),
    });
    crate::artifact::write_manifest(temp.path(), &manifest).unwrap();

    handle_stage(
        &backend,
        temp.path(),
        stage_request(temp.path(), &["remote/historical/new.txt"], false),
        "op_stage",
    )
    .unwrap();

    assert!(staged(&backend, &member_root, "historical/new.txt"));
}

#[test]
fn root_stage_refresh_protects_detached_checkout_from_gitlink_staging() {
    let temp = TempDir::new("stage-detached-boundary");
    let backend = Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "stage-detached-boundary-source");

    let mut manifest = crate::artifact::read_manifest(temp.path()).unwrap();
    let member_id = manifest.members[0].id.clone();
    manifest.members[0].active = false;
    crate::artifact::write_manifest(temp.path(), &manifest).unwrap();
    let mut lock = crate::artifact::read_lock(temp.path()).unwrap();
    lock.members.remove(&member_id);
    crate::artifact::write_lock(temp.path(), &lock).unwrap();
    // Simulate stale/missing local projection: handle_stage must rebuild it before root add.
    fs::write(temp.path().join(".git/info/exclude"), "").unwrap();
    fs::write(temp.path().join("root.txt"), "root\n").unwrap();

    handle_stage(
        &backend,
        temp.path(),
        stage_request(temp.path(), &["."], false),
        "op_stage",
    )
    .unwrap();

    assert!(staged(&backend, temp.path(), "root.txt"));
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(temp.path())
        .args(["ls-files", "--stage", "remote"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        output.stdout.is_empty(),
        "detached checkout must not be staged as a root gitlink"
    );
}
