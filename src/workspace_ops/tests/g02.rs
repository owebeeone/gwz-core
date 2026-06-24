    use std::fs;
    use std::path::{Path, PathBuf};
    
    
    

    use crate::artifact::{read_lock, read_manifest, read_snapshot};
    use crate::git::{Git2Backend, GitBackend};
    use crate::model::ErrorCode;
    use crate::operation::NullSink;

    
use super::*;

#[test]
    pub(crate) fn create_workspace_writes_empty_manifest_and_lock() {
        let temp = TempDir::new("create-workspace");
        let backend = Git2Backend::new();
        let response =
            handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        assert!(response.response.members.is_empty());
        assert!(backend.is_repository(temp.path()).unwrap());
        assert!(temp.path().join("gwz.conf/gwz.yml").is_file());
        assert!(temp.path().join("gwz.conf/gwz.lock.yml").is_file());
        assert!(!temp.path().join("workspace").exists());
        let root_status = backend.status(temp.path()).unwrap();
        assert_eq!(root_status.untracked, 0);
        assert!(
            root_status
                .files
                .iter()
                .any(|file| { file.path == "gwz.conf/gwz.yml" && file.index_status == "A" })
        );
        assert!(
            root_status
                .files
                .iter()
                .any(|file| { file.path == "gwz.conf/gwz.lock.yml" && file.index_status == "A" })
        );
        assert_eq!(read_manifest(temp.path()).unwrap().members.len(), 0);
        assert_eq!(read_lock(temp.path()).unwrap().members.len(), 0);
    }

    #[test]
    pub(crate) fn create_workspace_rejects_existing_and_nested_workspaces() {
        let temp = TempDir::new("reject-workspace");
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        assert_eq!(
            handle_create_workspace(create_workspace_request(temp.path()), "op_create")
                .unwrap_err()
                .code,
            ErrorCode::WorkspaceAlreadyExists
        );

        let child = temp.path().join("repos/child");
        fs::create_dir_all(&child).unwrap();
        assert_eq!(
            handle_create_workspace(create_workspace_request(&child), "op_create")
                .unwrap_err()
                .code,
            ErrorCode::NestedWorkspace
        );
    }

    #[test]
    pub(crate) fn create_repo_writes_manifest_lock_and_empty_git_repo() {
        let temp = TempDir::new("create-repo");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        let response = handle_create_repo(
            &backend,
            temp.path(),
            create_repo_request("repos/app", None, None),
            "op_repo",
        )
        .unwrap();

        let member = response.response.members.single();
        assert_eq!(member.status, crate::MemberStatus::Ok);
        assert_eq!(member.state.as_ref().unwrap().member_id, "mem_app");
        assert_eq!(member.state.as_ref().unwrap().commit, None);
        assert_eq!(
            member.state.as_ref().unwrap().branch,
            Some("main".to_owned())
        );
        assert!(
            backend
                .is_repository(&temp.path().join("repos/app"))
                .unwrap()
        );

        let manifest = read_manifest(temp.path()).unwrap();
        assert_eq!(manifest.members.len(), 1);
        assert_eq!(manifest.members[0].id, "mem_app");
        assert_eq!(manifest.members[0].source_id, "src_app");
        assert_eq!(
            manifest.members[0]
                .desired
                .as_ref()
                .and_then(|desired| desired.local_only),
            Some(true)
        );

        let lock = read_lock(temp.path()).unwrap();
        let locked = lock.members.get("mem_app").unwrap();
        assert_eq!(locked.commit, None);
        assert_eq!(locked.branch, Some("main".to_owned()));
        assert_eq!(locked.dirty, Some(false));
        assert_eq!(locked.materialized, Some(true));
    }

    #[test]
    pub(crate) fn add_existing_repo_records_current_git_state_and_remotes_without_reclone() {
        let temp = TempDir::new("add-existing");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let repo_path = temp.path().join("repos/existing");
        backend.create_repo(&repo_path).unwrap();
        let commit = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
        backend
            .add_remote(&repo_path, "origin", "file:///tmp/existing.git")
            .unwrap();
        fs::write(repo_path.join("README.md"), "dirty").unwrap();

        let response = handle_add_existing_repo(
            &backend,
            temp.path(),
            crate::AddExistingRepoRequest {
                meta: request_meta_with_workspace(),
                repository_path: repo_path.to_string_lossy().into_owned(),
                member_path: None,
                member_id: None,
                source_id: None,
            },
            "op_add",
        )
        .unwrap();

        let member = response.response.members.single();
        assert_eq!(member.member_path, "repos/existing");
        assert_eq!(member.state.as_ref().unwrap().commit, Some(commit.clone()));
        assert_eq!(member.state.as_ref().unwrap().dirty, Some(true));
        assert!(repo_path.join(".git").is_dir());

        let manifest = read_manifest(temp.path()).unwrap();
        assert_eq!(
            manifest.members[0].remotes[0].url,
            "file:///tmp/existing.git"
        );
        let locked = read_lock(temp.path())
            .unwrap()
            .members
            .get("mem_existing")
            .cloned()
            .unwrap();
        assert_eq!(locked.commit, Some(commit));
        assert_eq!(locked.dirty, Some(true));
    }

    #[test]
    pub(crate) fn add_existing_repo_accepts_relative_path_inside_workspace() {
        let temp = TempDir::new("add-existing-relative");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let repo_path = temp.path().join("local-repo");
        backend.create_repo(&repo_path).unwrap();
        commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
        let start = temp.path().join("gwz.conf");

        let response = handle_add_existing_repo(
            &backend,
            &start,
            crate::AddExistingRepoRequest {
                meta: request_meta_with_workspace(),
                repository_path: "../local-repo".to_owned(),
                member_path: None,
                member_id: None,
                source_id: None,
            },
            "op_add",
        )
        .unwrap();

        assert_eq!(response.response.members.single().member_path, "local-repo");
        let manifest = read_manifest(temp.path()).unwrap();
        assert_eq!(manifest.members[0].path, "local-repo");
    }

    #[test]
    pub(crate) fn init_from_sources_derives_default_paths_and_rejects_collisions() {
        let temp = TempDir::new("init-sources");
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        let backend = Git2Backend::new();
        let response = handle_init_from_sources(
            &backend,
            temp.path(),
            crate::InitFromSourcesRequest {
                meta: request_meta(),
                workspace_root: temp.path().to_string_lossy().into_owned(),
                sources: vec![
                    crate::SourceUrl {
                        url: "git@github.com:org/repo-a.git".to_owned(),
                        path: None,
                        remote_name: None,
                        branch: None,
                    },
                    crate::SourceUrl {
                        url: "https://github.com/org/repo-b".to_owned(),
                        path: None,
                        remote_name: Some("github".to_owned()),
                        branch: Some("main".to_owned()),
                    },
                ],
                target: None,
                workspace_id: Some("ws_ops".to_owned()),
            },
            "op_init",
            &NullSink,
        )
        .unwrap();

        assert_eq!(response.response.members[0].member_path, "repo-a");
        assert_eq!(response.response.members[1].member_path, "repo-b");
        assert_eq!(
            response.response.members[0]
                .planned
                .as_ref()
                .unwrap()
                .action,
            crate::PlannedAction::Clone
        );

        let collision = handle_init_from_sources(
            &backend,
            temp.path(),
            crate::InitFromSourcesRequest {
                meta: request_meta(),
                workspace_root: temp.path().to_string_lossy().into_owned(),
                sources: vec![
                    crate::SourceUrl {
                        url: "https://example.invalid/dup.git".to_owned(),
                        path: None,
                        remote_name: None,
                        branch: None,
                    },
                    crate::SourceUrl {
                        url: "ssh://example.invalid/dup".to_owned(),
                        path: None,
                        remote_name: None,
                        branch: None,
                    },
                ],
                target: None,
                workspace_id: Some("ws_ops".to_owned()),
            },
            "op_init",
            &NullSink,
        )
        .unwrap_err();
        assert_eq!(collision.code, ErrorCode::PathCollision);
    }

    #[test]
    pub(crate) fn snapshot_write_selected_member_records_with_attribution() {
        let temp = TempDir::new("snapshot-tag");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        handle_create_repo(
            &backend,
            temp.path(),
            create_repo_request("repos/app", None, None),
            "op_repo",
        )
        .unwrap();
        let lock_before = read_lock(temp.path()).unwrap();

        let snapshot_response = handle_snapshot(
            &backend,
            temp.path(),
            crate::SnapshotRequest {
                meta: request_meta_with_actor_selection("agent://tester", &["mem_app"]),
                snapshot_id: "snap_one".to_owned(),
            },
            "op_snapshot",
        )
        .unwrap();

        assert_eq!(
            snapshot_response.response.members.single().member_id,
            "mem_app"
        );
        let snapshot = read_snapshot(temp.path(), "snap_one").unwrap();
        assert_eq!(snapshot.created_by.actor_id, "agent://tester");
        assert_eq!(snapshot.selected_members, vec!["mem_app"]);
        assert!(snapshot.members.contains_key("mem_app"));
        assert_eq!(read_lock(temp.path()).unwrap(), lock_before);
    }

    #[test]
    pub(crate) fn snapshot_rejects_a_duplicate_id() {
        // F13: snapshot must refuse to overwrite an existing snapshot id, like `tag` does.
        let temp = TempDir::new("snapshot-dup");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        handle_create_repo(
            &backend,
            temp.path(),
            create_repo_request("repos/app", None, None),
            "op_repo",
        )
        .unwrap();
        let request = || crate::SnapshotRequest {
            meta: request_meta_with_actor_selection("agent://tester", &["mem_app"]),
            snapshot_id: "snap_dup".to_owned(),
        };
        handle_snapshot(&backend, temp.path(), request(), "op_snap1").unwrap();
        let err = handle_snapshot(&backend, temp.path(), request(), "op_snap2").unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    pub(crate) fn snapshot_records_observed_dirty_state_not_stale_lock() {
        let temp = TempDir::new("snapshot-observe");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("snap-dirty-source");
        let commit = fixture.commit_and_push("README.md", "one", "initial", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &commit);
        backend
            .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
            .unwrap();
        // Dirty the tracked worktree AFTER the lock was recorded (lock says clean).
        std::fs::write(temp.path().join("repos/app/README.md"), "dirty").unwrap();

        let response = handle_snapshot(
            &backend,
            temp.path(),
            crate::SnapshotRequest {
                meta: request_meta_with_actor_selection("agent://tester", &["mem_app"]),
                snapshot_id: "snap_dirty".to_owned(),
            },
            "op_snapshot",
        )
        .unwrap();
        assert_eq!(response.response.members.single().member_id, "mem_app");

        // F3: the snapshot records the OBSERVED dirty worktree, not the stale clean
        // lock (the fixture lock records dirty=false).
        let snapshot = read_snapshot(temp.path(), "snap_dirty").unwrap();
        assert_eq!(snapshot.members["mem_app"].dirty, Some(true));
    }

    #[test]
    pub(crate) fn capture_adopts_observed_state_into_lock_without_mutating() {
        let temp = TempDir::new("capture");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("capture-source");
        let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &first);
        backend
            .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
            .unwrap();
        // Developer advances the member past the lock with a local commit.
        let first_oid = git2::Oid::from_str(&first).unwrap();
        let second = commit_file(
            &temp.path().join("repos/app"),
            "README.md",
            "two",
            "second",
            &[first_oid],
        )
        .unwrap();
        assert_eq!(
            read_lock(temp.path()).unwrap().members["mem_app"].commit,
            Some(first)
        );

        let response = handle_capture(
            &backend,
            temp.path(),
            crate::CaptureRequest {
                meta: request_meta_with_actor_selection("agent://tester", &["mem_app"]),
            },
            "op_capture",
        )
        .unwrap();

        // The lock now records the OBSERVED commit; the worktree is untouched.
        assert_eq!(
            read_lock(temp.path()).unwrap().members["mem_app"].commit,
            Some(second.clone())
        );
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(second)
        );
        assert_eq!(response.response.members.single().member_id, "mem_app");
    }

    #[test]
    pub(crate) fn creating_a_duplicate_git_tag_is_idempotent() {
        let temp = TempDir::new("tag-errors");
        let backend = Git2Backend::new();
        let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-errors-source");
        let request = crate::TagRequest {
            meta: request_meta(),
            op: crate::TagOp::Create,
            name: Some("release-one".to_owned()),
            message: None,
            signed: None,
            remote: None,
            all: None,
        };
        handle_tag(&backend, temp.path(), request.clone(), "op_tag").unwrap();
        // A second create is an idempotent no-op (members already carrying the tag are skipped),
        // not an error — the duplicate guard lives in the handler, ahead of the primitive.
        handle_tag(&backend, temp.path(), request, "op_tag").unwrap();
    }

    #[test]
    pub(crate) fn clone_workspace_rejects_url_that_is_not_a_workspace() {
        let temp = TempDir::new("clone-non-workspace");
        let backend = Git2Backend::new();
        let fixture = RemoteFixture::new("clone-non-workspace-source");
        fixture.commit_and_push("README.md", "one", "initial", &backend);
        let target = temp.path().join("clone");

        let err = handle_clone_workspace(
            &backend,
            request_meta(),
            fixture.remote_url(),
            target.to_str().unwrap(),
            "op_clone",
            &NullSink,
        )
        .unwrap_err();

        assert_eq!(err.code, ErrorCode::WorkspaceNotFound);
    }

    pub(crate) fn create_workspace_request(root: &Path) -> crate::CreateWorkspaceRequest {
        crate::CreateWorkspaceRequest {
            meta: request_meta(),
            workspace_root: root.to_string_lossy().into_owned(),
            workspace_id: Some("ws_ops".to_owned()),
        }
    }

    pub(crate) fn create_repo_request(
        member_path: &str,
        member_id: Option<&str>,
        source_id: Option<&str>,
    ) -> crate::CreateRepoRequest {
        crate::CreateRepoRequest {
            meta: request_meta_with_workspace(),
            member_path: member_path.to_owned(),
            initial_branch: None,
            member_id: member_id.map(ToOwned::to_owned),
            source_id: source_id.map(ToOwned::to_owned),
        }
    }

    pub(crate) fn commit_file(
        repo_path: &Path,
        relative_path: &str,
        content: &str,
        message: &str,
        parents: &[git2::Oid],
    ) -> Result<String, git2::Error> {
        fs::write(repo_path.join(relative_path), content).unwrap();
        let repo = git2::Repository::open(repo_path)?;
        let mut index = repo.index()?;
        index.add_path(Path::new(relative_path))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid")?;
        let parent_commits = parents
            .iter()
            .map(|id| repo.find_commit(*id))
            .collect::<Result<Vec<_>, _>>()?;
        let parent_refs = parent_commits.iter().collect::<Vec<_>>();
        Ok(repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent_refs,
            )?
            .to_string())
    }

    pub(crate) fn request_meta() -> crate::RequestMeta {
        crate::RequestMeta {
            request_id: "req_ops".to_owned(),
            schema_version: "gwz.protocol/v0".to_owned(),
            ..Default::default()
        }
    }

    pub(crate) fn request_meta_with_workspace() -> crate::RequestMeta {
        crate::RequestMeta {
            workspace: Some(crate::WorkspaceRef {
                root: None,
                workspace_id: Some("ws_ops".to_owned()),
            }),
            ..request_meta()
        }
    }

    pub(crate) fn request_meta_with_actor_selection(
        actor_id: &str,
        member_ids: &[&str],
    ) -> crate::RequestMeta {
        crate::RequestMeta {
            selection: Some(crate::Selection {
                all: Some(false),
                member_ids: member_ids.iter().map(|value| (*value).to_owned()).collect(),
                paths: Vec::new(),
            }),
            attribution: Some(crate::OperationAttribution {
                actor: Some(crate::OperationActor {
                    actor_id: actor_id.to_owned(),
                    display_name: None,
                    email: None,
                    authority: None,
                }),
                ..Default::default()
            }),
            ..request_meta_with_workspace()
        }
    }

    pub(crate) struct TempDir {
        pub(crate) path: PathBuf,
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
