    use std::fs;
    use std::path::{Path, PathBuf};
    
    
    

    use crate::artifact::{read_lock, read_manifest};
    use crate::git::{Git2Backend, GitBackend};
    use crate::model::ErrorCode;
    use crate::operation::{EventSink, NullSink};

    
use super::*;

#[derive(Default)]
    pub(crate) struct CollectingSink {
        pub(crate) events: std::sync::Mutex<Vec<crate::OperationEvent>>,
    }

    impl EventSink for CollectingSink {
        fn deliver(&self, event: crate::OperationEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    impl CollectingSink {
        pub(crate) fn take(&self) -> Vec<crate::OperationEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    #[test]
    pub(crate) fn init_from_sources_can_create_workspace_clone_local_urls_and_write_lock() {
        let temp = TempDir::new("init-exec");
        let backend = Git2Backend::new();
        let fixture = RemoteFixture::new("init-exec-source");
        let commit = fixture.commit_and_push("README.md", "one", "initial", &backend);

        let events = CollectingSink::default();
        let response = handle_init_from_sources(
            &backend,
            temp.path(),
            crate::InitFromSourcesRequest {
                meta: request_meta(),
                workspace_root: temp.path().to_string_lossy().into_owned(),
                sources: vec![crate::SourceUrl {
                    url: fixture.remote_url().to_owned(),
                    path: None,
                    remote_name: None,
                    branch: None,
                }],
                target: None,
                workspace_id: Some("ws_ops".to_owned()),
            },
            "op_init",
            &events,
        )
        .unwrap();

        // init emits the per-member lifecycle, bracketed by operation events.
        let kinds: Vec<_> = events.take().into_iter().map(|event| event.kind).collect();
        assert_eq!(kinds.first(), Some(&crate::EventKind::OperationStarted));
        assert_eq!(kinds.last(), Some(&crate::EventKind::OperationFinished));
        assert!(kinds.contains(&crate::EventKind::MemberStarted));
        assert!(kinds.contains(&crate::EventKind::MemberFinished));

        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        assert!(backend.is_repository(temp.path()).unwrap());
        assert!(temp.path().join("gwz.conf/gwz.yml").is_file());
        assert!(temp.path().join("gwz.conf/gwz.lock.yml").is_file());
        assert!(!temp.path().join("workspace").exists());
        // Members + tmp are hidden via local .git/info/exclude; gwz writes no .gitignore.
        let exclude = fs::read_to_string(temp.path().join(".git/info/exclude")).unwrap();
        assert!(exclude.contains("/gwz.conf/.tmp/"));
        assert!(exclude.contains("/remote/"));
        assert!(!temp.path().join(".gitignore").exists());
        let root_status = backend.status(temp.path()).unwrap();
        assert_eq!(root_status.untracked, 0, "the member is excluded, not untracked");
        assert!(
            root_status
                .files
                .iter()
                .any(|file| { file.path == "gwz.conf/gwz.yml" && file.index_status == "A" })
        );
        assert_eq!(
            backend.head(&temp.path().join("remote")).unwrap().commit,
            Some(commit.clone())
        );
        let manifest = read_manifest(temp.path()).unwrap();
        assert_eq!(manifest.members[0].path, "remote");
        assert_eq!(manifest.members[0].remotes[0].name, "origin");
        assert_eq!(
            read_lock(temp.path()).unwrap().members["mem_remote"].commit,
            Some(commit)
        );
    }

    #[test]
    pub(crate) fn init_rolls_back_fresh_clones_on_mid_batch_failure() {
        let temp = TempDir::new("init-rollback");
        let backend = Git2Backend::new();
        let fixture = RemoteFixture::new("init-rollback-source");
        fixture.commit_and_push("README.md", "one", "initial", &backend);
        let bad_url = temp.path().join("does-not-exist.git");

        let result = handle_init_from_sources(
            &backend,
            temp.path(),
            crate::InitFromSourcesRequest {
                meta: request_meta(),
                workspace_root: temp.path().to_string_lossy().into_owned(),
                sources: vec![
                    crate::SourceUrl {
                        url: fixture.remote_url().to_owned(),
                        path: Some("app".to_owned()),
                        remote_name: None,
                        branch: None,
                    },
                    crate::SourceUrl {
                        url: bad_url.to_string_lossy().into_owned(),
                        path: Some("lib".to_owned()),
                        remote_name: None,
                        branch: None,
                    },
                ],
                target: None,
                workspace_id: Some("ws_ops".to_owned()),
            },
            "op_init",
            &NullSink,
        );

        assert!(result.is_err(), "init must fail when a source fails to clone");
        // F2/Q6 reject-partial: the successful clone is rolled back; no lock written.
        assert!(
            !temp.path().join("app").exists(),
            "fresh clone app must be rolled back"
        );
        assert!(
            !temp.path().join("lib").exists(),
            "fresh clone lib must be rolled back"
        );
        assert!(
            !temp.path().join("gwz.conf/gwz.lock.yml").is_file(),
            "no lock written on failed init"
        );
    }

    #[test]
    pub(crate) fn materialize_lock_clones_missing_member_and_checks_out_recorded_commit() {
        let temp = TempDir::new("materialize-clone");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("clone-source");
        let commit = fixture.commit_and_push("README.md", "one", "initial", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &commit);

        let events = CollectingSink::default();
        let response = handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(false),
            "op_materialize",
            &events,
        )
        .unwrap();

        assert_eq!(
            response.response.members.single().status,
            crate::MemberStatus::Ok
        );
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(commit)
        );

        // The clone-missing materialize emits a per-member lifecycle bracketed
        // by operation_started/finished. Transfer-progress volume depends on
        // libgit2's local-clone behavior, so assert the deterministic envelope
        // and that any emitted progress is well-formed for this member.
        let collected = events.take();
        let kinds: Vec<_> = collected.iter().map(|event| event.kind).collect();
        assert_eq!(kinds.first(), Some(&crate::EventKind::OperationStarted));
        assert_eq!(kinds.last(), Some(&crate::EventKind::OperationFinished));
        let started = collected
            .iter()
            .position(|event| event.kind == crate::EventKind::MemberStarted)
            .expect("member_started emitted");
        let finished = collected
            .iter()
            .position(|event| event.kind == crate::EventKind::MemberFinished)
            .expect("member_finished emitted");
        assert!(
            started < finished,
            "member_started precedes member_finished"
        );
        assert_eq!(collected[started].member_path.as_deref(), Some("repos/app"));
        for event in &collected {
            if event.kind == crate::EventKind::MemberProgress {
                assert_eq!(event.member_path.as_deref(), Some("repos/app"));
                let progress = event.progress.as_ref().expect("progress payload present");
                assert!(matches!(
                    progress.phase,
                    crate::GitProgressPhase::Receiving | crate::GitProgressPhase::Resolving
                ));
            }
        }
    }

    #[test]
    pub(crate) fn clone_workspace_clones_root_and_materializes_missing_members() {
        let temp = TempDir::new("clone-workspace");
        let backend = Git2Backend::new();
        // Build a source workspace whose root repo commits gwz.conf, with a
        // member that lives at a remote and is absent from the root tree.
        let source_ws = temp.path().join("origin");
        fs::create_dir_all(&source_ws).unwrap();
        handle_create_workspace(create_workspace_request(&source_ws), "op_create").unwrap();
        let fixture = RemoteFixture::new("clone-workspace-member");
        let commit = fixture.commit_and_push("README.md", "one", "initial", &backend);
        write_materialize_fixture(&source_ws, fixture.remote_url(), &commit);
        commit_workspace_root(&source_ws);

        // Clone the workspace from its root URL into a fresh target.
        let target = temp.path().join("clone");
        let response = handle_clone_workspace(
            &backend,
            request_meta(),
            source_ws.to_str().unwrap(),
            target.to_str().unwrap(),
            "op_clone",
            &NullSink,
        )
        .unwrap();

        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        assert_eq!(
            response.response.members.single().status,
            crate::MemberStatus::Ok
        );
        // gwz.conf came over with the clone, and the member was materialized.
        assert!(target.join(crate::artifact::LOCK_PATH).is_file());
        assert_eq!(
            backend.head(&target.join("repos/app")).unwrap().commit,
            Some(commit)
        );
    }

    #[test]
    pub(crate) fn materialize_lock_blocks_dirty_member_by_default() {
        let temp = TempDir::new("materialize-dirty");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("dirty-source");
        let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
        let second = fixture.commit_and_push("README.md", "two", "second", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &first);
        backend
            .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
            .unwrap();
        fs::write(temp.path().join("repos/app/README.md"), "dirty").unwrap();

        let err = handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(false),
            "op_materialize",
            &NullSink,
        )
        .unwrap_err();

        assert_eq!(err.code, ErrorCode::DirtyMember);
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(second)
        );
    }

    #[test]
    pub(crate) fn materialize_lock_moves_clean_member_and_dry_run_does_not_mutate() {
        let temp = TempDir::new("materialize-clean");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("clean-source");
        let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
        let second = fixture.commit_and_push("README.md", "two", "second", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &first);
        backend
            .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
            .unwrap();

        let dry_run = handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(true),
            "op_materialize",
            &NullSink,
        )
        .unwrap();
        assert_eq!(
            dry_run
                .response
                .members
                .single()
                .planned
                .as_ref()
                .unwrap()
                .action,
            crate::PlannedAction::Checkout
        );
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(second)
        );

        handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(false),
            "op_materialize",
            &NullSink,
        )
        .unwrap();
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(first)
        );
    }

    #[test]
    pub(crate) fn materialize_snapshot_rewrites_lock_after_success() {
        let temp = TempDir::new("materialize-snapshot-tag");
        let backend = Git2Backend::new();
        let fixture = materialize_snapshot_fixture(temp.path(), &backend);

        handle_materialize(
            &backend,
            temp.path(),
            materialize_named_request(crate::MaterializeTargetKind::Snapshot, "snap_first"),
            "op_materialize",
            &NullSink,
        )
        .unwrap();
        assert_eq!(
            read_lock(temp.path()).unwrap().members["mem_app"].commit,
            Some(fixture.first.clone())
        );

    }

    pub(crate) fn commit_workspace_root(root: &Path) {
        let repo = git2::Repository::open(root).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["."], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "init workspace",
            &tree,
            &[],
        )
        .unwrap();
    }

    #[test]
    pub(crate) fn materialize_rolls_back_fresh_clones_on_mid_batch_failure() {
        let temp = TempDir::new("materialize-rollback");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("rollback-source");
        let commit = fixture.commit_and_push("README.md", "one", "initial", &backend);
        let bad_oid = "0".repeat(40);
        // app clones + checks out cleanly; lib clones then fails to check out a
        // commit that does not exist -> a mid-batch failure.
        write_pull_fixture(
            temp.path(),
            vec![
                ("mem_app", "repos/app", fixture.remote_url(), &commit),
                ("mem_lib", "repos/lib", fixture.remote_url(), &bad_oid),
            ],
        );

        let result = handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(false),
            "op_materialize",
            &NullSink,
        );

        assert!(result.is_err(), "materialize must fail when a member fails");
        // F2/Q6 reject-partial: this op's fresh clones are rolled back — no orphans.
        assert!(
            !temp.path().join("repos/app").exists(),
            "fresh clone repos/app must be rolled back"
        );
        assert!(
            !temp.path().join("repos/lib").exists(),
            "fresh clone repos/lib must be rolled back"
        );
    }

    #[test]
    pub(crate) fn materialize_restores_onto_saved_branch_not_detached() {
        let temp = TempDir::new("materialize-branch");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("branch-source");
        let commit = fixture.commit_and_push("README.md", "one", "initial", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &commit);

        let response = handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(false),
            "op_materialize",
            &NullSink,
        )
        .unwrap();

        // AD3(c): the lock fixture records branch=main; materialize restores ONTO the
        // branch (not detached) and records that observed state — F1's re-observe.
        let member = response.response.members.single();
        let state = member.state.as_ref().expect("member state");
        assert_eq!(state.detached, Some(false));
        assert_eq!(state.branch.as_deref(), Some("main"));
        assert_eq!(state.commit.as_deref(), Some(commit.as_str()));
        // The member HEAD is genuinely on `main`, not detached.
        let head = backend.head(&temp.path().join("repos/app")).unwrap();
        assert!(!head.is_detached);
        assert_eq!(head.branch.as_deref(), Some("main"));
    }

    #[test]
    pub(crate) fn materialize_detaches_a_diverged_member_preserving_its_branch() {
        let temp = TempDir::new("materialize-diverged");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("diverged-source");
        let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &first);
        backend
            .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
            .unwrap();
        // Developer advances main past the lock commit.
        let first_oid = git2::Oid::from_str(&first).unwrap();
        let app = temp.path().join("repos/app");
        let second = commit_file(&app, "README.md", "two", "second", &[first_oid]).unwrap();

        handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(false),
            "op_materialize",
            &NullSink,
        )
        .unwrap();

        // AD3(c) orphan-safety: materialize reaches the lock commit by DETACHING —
        // it must NOT reset main (that would orphan `second`).
        let head = backend.head(&app).unwrap();
        assert!(head.is_detached);
        assert_eq!(head.commit.as_deref(), Some(first.as_str()));
        // main is preserved at the developer's commit.
        assert_eq!(
            backend.read_ref(&app, "refs/heads/main").unwrap().as_deref(),
            Some(second.as_str())
        );
    }

    pub(crate) fn materialize_lock_request(dry_run: bool) -> crate::MaterializeRequest {
        crate::MaterializeRequest {
            meta: crate::RequestMeta {
                dry_run: Some(dry_run),
                ..request_meta_with_workspace()
            },
            target: crate::MaterializeTarget {
                kind: crate::MaterializeTargetKind::Lock,
                name: None,
                commit: None,
            },
        }
    }

    pub(crate) fn materialize_named_request(
        kind: crate::MaterializeTargetKind,
        name: &str,
    ) -> crate::MaterializeRequest {
        crate::MaterializeRequest {
            meta: request_meta_with_workspace(),
            target: crate::MaterializeTarget {
                kind,
                name: Some(name.to_owned()),
                commit: None,
            },
        }
    }

    pub(crate) struct SnapshotFixture {
        pub(crate) first: String,
        pub(crate) second: String,
    }

    pub(crate) fn materialize_snapshot_fixture(root: &Path, backend: &Git2Backend) -> SnapshotFixture {
        handle_create_workspace(create_workspace_request(root), "op_create").unwrap();
        let fixture = RemoteFixture::new("snapshot-source");
        let first = fixture.commit_and_push("README.md", "one", "initial", backend);
        let second = fixture.commit_and_push("README.md", "two", "second", backend);
        write_materialize_fixture(root, fixture.remote_url(), &second);
        backend
            .clone_repo(fixture.remote_url(), &root.join("repos/app"))
            .unwrap();
        let snapshot_members = std::collections::BTreeMap::from([(
            "mem_app".to_owned(),
            test_member_state("repos/app", Some(first.clone()), false),
        )]);
        crate::artifact::write_snapshot(
            root,
            &crate::artifact::SnapshotArtifact {
                schema: crate::artifact::SNAPSHOT_SCHEMA.to_owned(),
                workspace_id: "ws_ops".to_owned(),
                snapshot_id: "snap_first".to_owned(),
                created_at: "2026-06-15T00:00:00Z".to_owned(),
                created_by: crate::artifact::CreatedByArtifact {
                    actor_id: "agent://tester".to_owned(),
                },
                selected_members: vec!["mem_app".to_owned()],
                members: snapshot_members,
            },
        )
        .unwrap();
        SnapshotFixture {
            first,
            second,
        }
    }

    pub(crate) fn write_materialize_fixture(root: &Path, remote_url: &str, commit: &str) {
        crate::artifact::write_manifest(
            root,
            &crate::artifact::ManifestArtifact {
                schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
                workspace: crate::artifact::WorkspaceHeader {
                    id: "ws_ops".to_owned(),
                },
                members: vec![crate::artifact::ManifestMember {
                    id: "mem_app".to_owned(),
                    path: "repos/app".to_owned(),
                    source_kind: crate::artifact::ArtifactSourceKind::Git,
                    source_id: "src_app".to_owned(),
                    active: true,
                    desired: Some(crate::artifact::DesiredRefArtifact {
                        branch: Some("main".to_owned()),
                        ..Default::default()
                    }),
                    remotes: vec![crate::artifact::RemoteArtifact {
                        name: "origin".to_owned(),
                        url: remote_url.to_owned(),
                        fetch: true,
                        push: true,
                    }],
                }],
            },
        )
        .unwrap();
        crate::artifact::write_lock(
            root,
            &test_lock("mem_app", "repos/app", Some(commit.to_owned()), false),
        )
        .unwrap();
    }

    pub(crate) struct RemoteFixture {
        pub(crate) _temp: TempDir,
        pub(crate) source: PathBuf,
        pub(crate) remote: PathBuf,
    }

    