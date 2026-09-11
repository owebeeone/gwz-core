use super::*;
use crate::git::Git2Backend;
use crate::workspace_ops::tests::{TempDir, commit_file, create_workspace_request};
use std::fs;

#[test]
fn update_commit_accepts_configuration_without_claiming_unrelated_staged_work() {
    let temp = TempDir::new("bootstrap-scoped-commit");
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    commit_file(temp.path(), "README", "baseline\n", "initial", &[]).unwrap();
    let backend = Git2Backend::new();
    let parent = backend.head(temp.path()).unwrap().commit.unwrap();
    fs::write(temp.path().join("unrelated"), "staged\n").unwrap();
    backend.stage_paths(temp.path(), &["unrelated"]).unwrap();
    fs::write(temp.path().join("unrelated"), "unstaged\n").unwrap();
    let manifest =
        fs::read_to_string(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
    fs::write(
        temp.path().join(crate::workspace::WORKSPACE_MANIFEST),
        format!("# accepted comment\n{manifest}"),
    )
    .unwrap();
    let meta = crate::RequestMeta {
        policy: Some(crate::OperationPolicy {
            destructive: Some(crate::DestructiveBehavior::Allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    let response = handle_update_workspace_bootstrap_with_commit(
        &backend,
        temp.path(),
        meta.clone(),
        true,
        "recover",
    )
    .unwrap();
    let head = backend.head(temp.path()).unwrap().commit.unwrap();
    assert_ne!(head, parent);
    let repo = git2::Repository::open(temp.path()).unwrap();
    let tree = repo.head().unwrap().peel_to_tree().unwrap();
    assert!(tree.get_path(Path::new("unrelated")).is_err());
    let entry = repo
        .index()
        .unwrap()
        .get_path(Path::new("unrelated"), 0)
        .unwrap();
    assert_eq!(repo.find_blob(entry.id).unwrap().content(), b"staged\n");
    assert_eq!(
        fs::read(temp.path().join("unrelated")).unwrap(),
        b"unstaged\n"
    );
    assert_eq!(
        artifact::inspect_conf_integrity(temp.path()),
        artifact::ConfIntegrityVerdict::Verified
    );
    let status = backend.status(temp.path()).unwrap();
    assert_eq!(status.files.len(), 1, "{status:?}");
    assert!(response.meta.message.unwrap().contains(&head));
    handle_update_workspace_bootstrap_with_commit(&backend, temp.path(), meta, true, "noop")
        .unwrap();
    assert_eq!(backend.head(temp.path()).unwrap().commit.unwrap(), head);
}

#[test]
fn update_commit_dry_run_leaves_head_index_and_files_unchanged() {
    let temp = TempDir::new("bootstrap-commit-dry-run");
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    commit_file(temp.path(), "README", "baseline\n", "initial", &[]).unwrap();
    let backend = Git2Backend::new();
    let head = backend.head(temp.path()).unwrap();
    fs::write(temp.path().join(AGENTS_GWZ_PATH), "local instructions\n").unwrap();
    let index = fs::read(temp.path().join(".git/index")).unwrap();
    let meta = crate::RequestMeta {
        dry_run: Some(true),
        policy: Some(crate::OperationPolicy {
            destructive: Some(crate::DestructiveBehavior::Allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    handle_update_workspace_bootstrap_with_commit(&backend, temp.path(), meta, true, "dry")
        .unwrap();
    assert_eq!(backend.head(temp.path()).unwrap(), head);
    assert_eq!(fs::read(temp.path().join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(temp.path().join(AGENTS_GWZ_PATH)).unwrap(),
        "local instructions\n"
    );
}

#[test]
fn update_commit_respects_git_line_ending_conversion() {
    let temp = TempDir::new("bootstrap-commit-crlf");
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    commit_file(temp.path(), "README", "baseline\n", "initial", &[]).unwrap();
    let repo = git2::Repository::open(temp.path()).unwrap();
    repo.config()
        .unwrap()
        .set_bool("core.autocrlf", true)
        .unwrap();
    let path = temp.path().join(crate::workspace::WORKSPACE_MANIFEST);
    let manifest = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!("# accepted\n{manifest}").replace('\n', "\r\n"),
    )
    .unwrap();
    let backend = Git2Backend::new();
    let meta = crate::RequestMeta {
        policy: Some(crate::OperationPolicy {
            destructive: Some(crate::DestructiveBehavior::Allow),
            ..Default::default()
        }),
        ..Default::default()
    };
    handle_update_workspace_bootstrap_with_commit(&backend, temp.path(), meta, true, "crlf")
        .unwrap();
    let status = backend.status(temp.path()).unwrap();
    assert!(status.files.is_empty(), "{status:?}");
    assert_eq!(
        artifact::inspect_conf_integrity(temp.path()),
        artifact::ConfIntegrityVerdict::Verified
    );
}
