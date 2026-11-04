//! Identical assertions against physical and in-memory repositories.
use super::*;
use crate::filesystem::{FileSystem, TestFsWorkspace};

#[test]
fn committed_index_is_independent_of_worktree() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let filesystem = context.filesystem();
    let backend = world.repository();
    let root = fixture(filesystem, &backend);
    assert_eq!(
        exists(filesystem, &root.path.join(".git/config")),
        std::env::var("GWZ_TEST_GIT").as_deref() == Ok("real")
    );
    write(filesystem, &root.path.join("file.txt"), b"staged\n");
    backend.stage_paths(&root.path, &["file.txt"]).unwrap();
    write(filesystem, &root.path.join("file.txt"), b"unstaged\n");
    let commit = backend
        .commit(&root.path, "snapshot index", false)
        .unwrap()
        .commit;
    assert_eq!(
        backend
            .read_file_at_commit(&root.path, &commit, "file.txt")
            .unwrap(),
        Some(b"staged\n".to_vec())
    );
    assert_eq!(
        backend
            .read_file_at_commit(&root.path, &commit, "absent")
            .unwrap(),
        None
    );
    assert!(backend.commit_exists(&root.path, &commit).unwrap());
    assert!(
        !backend
            .commit_exists(&root.path, "not-an-object-id")
            .unwrap()
    );
    assert_eq!(
        backend.read_ref(&root.path, "refs/heads/main").unwrap(),
        Some(commit.clone())
    );
    assert_eq!(
        backend.head(&root.path).unwrap().commit,
        Some(commit.clone())
    );
    // Reopening the same world must observe the same repository.
    assert_eq!(
        world.repository().head(&root.path).unwrap().commit,
        Some(commit)
    );
    let status = backend.status(&root.path).unwrap();
    assert_eq!(
        (status.staged, status.unstaged, status.untracked),
        (0, 1, 0)
    );
    assert!(status.is_dirty);
}

struct Fixture {
    _directory: TestFsWorkspace,
    path: PathBuf,
}

fn fixture<B: GitRepository>(filesystem: &dyn FileSystem, backend: &B) -> Fixture {
    let directory = filesystem.test_workspace().unwrap();
    let root = Fixture {
        path: directory.path().to_path_buf(),
        _directory: directory,
    };
    backend
        .test_init_repo(&root.path, &TestRepoSpec::default())
        .unwrap();
    root
}

#[test]
fn checked_backup_ref_rejects_drift_and_is_idempotent() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let filesystem = context.filesystem();
    let backend = world.repository();
    let root = fixture(filesystem, &backend);
    assert_eq!(
        exists(filesystem, &root.path.join(".git/config")),
        std::env::var("GWZ_TEST_GIT").as_deref() == Ok("real")
    );
    write(filesystem, &root.path.join("file.txt"), b"first");
    backend.stage_paths(&root.path, &["file.txt"]).unwrap();
    let first = backend.commit(&root.path, "first", false).unwrap().commit;
    let name = "refs/gwz/merge/contract/root/head";
    let created = backend
        .create_backup_ref_checked(&root.path, "main", &first, name, &first)
        .unwrap();
    assert_eq!(
        backend
            .create_backup_ref_checked(&root.path, "main", &first, name, &first)
            .unwrap(),
        created
    );
    write(filesystem, &root.path.join("file.txt"), b"second");
    backend.stage_paths(&root.path, &["file.txt"]).unwrap();
    let second = backend.commit(&root.path, "second", false).unwrap().commit;
    assert_eq!(
        backend
            .create_backup_ref_checked(&root.path, "main", &second, name, &second)
            .unwrap_err()
            .code,
        ErrorCode::PreservationEvidenceMismatch
    );
    assert_eq!(
        backend.read_ref(&root.path, name).unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        backend
            .delete_backup_ref_checked(&root.path, name, &second)
            .unwrap_err()
            .code,
        ErrorCode::MergeDrift
    );
    assert_eq!(
        backend.read_ref(&root.path, name).unwrap(),
        Some(first.clone())
    );
    backend
        .delete_backup_ref_checked(&root.path, name, &first)
        .unwrap();
    backend
        .delete_backup_ref_checked(&root.path, name, &first)
        .unwrap();
    assert_eq!(
        backend.observe_direct_ref(&root.path, name).unwrap(),
        GitDirectRefObservation::Absent
    );
}

#[test]
fn checked_reset_restores_content_and_rejects_dirty_work() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let filesystem = context.filesystem();
    let backend = world.repository();
    let root = fixture(filesystem, &backend);
    write(filesystem, &root.path.join("file.txt"), b"first");
    backend.stage_paths(&root.path, &["file.txt"]).unwrap();
    let first = backend.commit(&root.path, "first", false).unwrap().commit;
    write(filesystem, &root.path.join("file.txt"), b"second");
    backend.stage_paths(&root.path, &["file.txt"]).unwrap();
    let second = backend.commit(&root.path, "second", false).unwrap().commit;
    assert!(backend.is_ancestor(&root.path, &first, &second).unwrap());
    assert!(!backend.is_ancestor(&root.path, &second, &first).unwrap());
    write(filesystem, &root.path.join("file.txt"), b"user work");
    assert_eq!(
        backend
            .set_branch_target_checked(&root.path, "main", &second, &first)
            .unwrap_err()
            .code,
        ErrorCode::DirtyMember
    );
    assert_eq!(
        backend.head(&root.path).unwrap().commit,
        Some(second.clone())
    );
    assert_eq!(
        filesystem.read(&root.path.join("file.txt")).unwrap(),
        b"user work"
    );
    write(filesystem, &root.path.join("file.txt"), b"second");
    let changed = backend
        .set_branch_target_checked(&root.path, "main", &second, &first)
        .unwrap();
    assert!(changed.updated);
    assert_eq!(
        filesystem.read(&root.path.join("file.txt")).unwrap(),
        b"first"
    );
    assert!(!backend.status(&root.path).unwrap().is_dirty);
    assert!(
        !backend
            .set_branch_target_checked(&root.path, "main", &second, &first)
            .unwrap()
            .updated
    );
    assert!(backend.commit_exists(&root.path, &second).unwrap());
}

#[test]
fn preservation_stash_captures_dirty_work_once() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let filesystem = context.filesystem();
    let backend = world.repository();
    let root = fixture(filesystem, &backend);
    write(filesystem, &root.path.join("file.txt"), b"base");
    backend.stage_paths(&root.path, &["file.txt"]).unwrap();
    let head = backend.commit(&root.path, "base", false).unwrap().commit;
    write(filesystem, &root.path.join("file.txt"), b"staged");
    backend.stage_paths(&root.path, &["file.txt"]).unwrap();
    write(filesystem, &root.path.join("file.txt"), b"unstaged");
    write(filesystem, &root.path.join("new.txt"), b"untracked");
    let image = backend.preservation_image(&root.path, true).unwrap();
    assert_eq!(
        image.dirty,
        GitPreservationDirtySummary {
            staged: true,
            unstaged: true,
            untracked: true
        }
    );
    let stash = backend
        .stash_for_merge_preservation_checked(
            &root.path,
            "main",
            &head,
            &image.preimage_sha256,
            "contract",
            true,
        )
        .unwrap();
    assert!(!backend.status(&root.path).unwrap().is_dirty);
    assert_eq!(
        filesystem.read(&root.path.join("file.txt")).unwrap(),
        b"base"
    );
    assert!(!exists(filesystem, &root.path.join("new.txt")));
    let evidence = backend
        .preservation_stashes(&root.path, "contract")
        .unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].image, image);
    assert_eq!(evidence[0].object_id, stash.object_id);
    assert_eq!(
        backend
            .stash_for_merge_preservation_checked(
                &root.path,
                "main",
                &head,
                &image.preimage_sha256,
                "contract",
                true
            )
            .unwrap(),
        stash
    );
    write(filesystem, &root.path.join("new.txt"), b"new user work");
    assert_eq!(
        backend
            .stash_for_merge_preservation_checked(
                &root.path,
                "main",
                &head,
                &image.preimage_sha256,
                "contract",
                true
            )
            .unwrap_err()
            .code,
        ErrorCode::PreservationEvidenceMismatch
    );
    assert_eq!(
        filesystem.read(&root.path.join("new.txt")).unwrap(),
        b"new user work"
    );
    assert_eq!(
        backend
            .preservation_stashes(&root.path, "contract")
            .unwrap(),
        evidence
    );
    filesystem.remove_file(&root.path.join("new.txt")).unwrap();
    backend
        .stash_apply(
            &root.path,
            &GitStashTarget::object_id(&stash.object_id),
            GitStashRestoreOptions::default(),
        )
        .unwrap();
    assert_eq!(
        filesystem.read(&root.path.join("file.txt")).unwrap(),
        b"unstaged"
    );
    assert_eq!(
        filesystem.read(&root.path.join("new.txt")).unwrap(),
        b"untracked"
    );
    assert_eq!(backend.preservation_image(&root.path, true).unwrap(), image);
    assert_eq!(
        backend
            .preservation_stashes(&root.path, "contract")
            .unwrap(),
        evidence
    );
}

#[test]
fn fixture_commits_and_ref_changes_do_not_rewrite_the_checkout() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let filesystem = context.filesystem();
    let git = world.repository();
    let temp = filesystem.test_workspace().unwrap();
    let root = temp.path();
    git.test_init_repo(root, &TestRepoSpec::default()).unwrap();
    write(filesystem, &root.join("file.txt"), b"index");
    git.stage_paths(root, &["file.txt"]).unwrap();
    let first = git
        .test_create_commit(root, &TestCommitSpec::from_index("first", vec![]))
        .unwrap();
    assert_eq!(git.head(root).unwrap().commit, None);
    git.test_set_ref(
        root,
        "refs/heads/main",
        Some(&TestRefTarget::Direct(first.clone())),
    )
    .unwrap();
    git.test_set_head(root, &TestHead::Attached("refs/heads/main".into()))
        .unwrap();
    write(filesystem, &root.join("file.txt"), b"user work");
    let second = git
        .test_create_commit(
            root,
            &TestCommitSpec::from_index("second", vec![first.clone()]),
        )
        .unwrap();
    assert_eq!(
        git.test_read_commit(root, &second).unwrap().parents,
        vec![first]
    );
    git.test_set_ref(
        root,
        "refs/heads/main",
        Some(&TestRefTarget::Direct(second.clone())),
    )
    .unwrap();
    assert_eq!(git.head(root).unwrap().commit, Some(second));
    assert_eq!(
        filesystem.read(&root.join("file.txt")).unwrap(),
        b"user work"
    );
    let index = git.test_read_index(root).unwrap();
    git.test_replace_index(root, &[]).unwrap();
    assert!(git.test_read_index(root).unwrap().is_empty());
    git.test_replace_index(root, &index).unwrap();
    assert_eq!(git.test_read_index(root).unwrap(), index);
}

fn exists(filesystem: &dyn FileSystem, path: &Path) -> bool {
    filesystem.metadata(path).is_ok()
}

fn write(filesystem: &dyn FileSystem, path: &Path, bytes: &[u8]) {
    if exists(filesystem, path) {
        filesystem.remove_file(path).unwrap();
    }
    let file = filesystem.create_file(path).unwrap();
    filesystem.write_all(&file, bytes).unwrap();
}

#[test]
fn fixture_config_preserves_repeated_and_empty_values() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let filesystem = context.filesystem();
    let git = world.repository();
    let root = fixture(filesystem, &git);
    let values = vec![String::new(), "one".into(), String::new(), "one".into()];
    git.test_set_config(&root.path, "fixture.values", &values)
        .unwrap();
    assert_eq!(
        git.test_read_config(&root.path, "fixture.values").unwrap(),
        values
    );
    git.test_set_config(&root.path, "fixture.values", &[])
        .unwrap();
    assert!(
        git.test_read_config(&root.path, "fixture.values")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn fixture_detached_head_and_symbolic_refs_resolve_independently() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let filesystem = context.filesystem();
    let git = world.repository();
    let root = fixture(filesystem, &git);
    let first = git
        .test_create_commit(&root.path, &TestCommitSpec::from_index("first", vec![]))
        .unwrap();
    let second = git
        .test_create_commit(
            &root.path,
            &TestCommitSpec::from_index("second", vec![first.clone()]),
        )
        .unwrap();
    git.test_set_ref(
        &root.path,
        "refs/heads/main",
        Some(&TestRefTarget::Direct(first.clone())),
    )
    .unwrap();
    git.test_set_ref(
        &root.path,
        "refs/heads/alias",
        Some(&TestRefTarget::Symbolic("refs/heads/main".into())),
    )
    .unwrap();
    assert_eq!(
        git.read_ref(&root.path, "refs/heads/alias").unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        git.observe_direct_ref(&root.path, "refs/heads/alias")
            .unwrap(),
        GitDirectRefObservation::NonDirect
    );
    git.test_set_head(&root.path, &TestHead::Detached(second.clone()))
        .unwrap();
    assert_eq!(
        git.read_ref(&root.path, "refs/heads/main").unwrap(),
        Some(first)
    );
    assert_eq!(git.read_ref(&root.path, "HEAD").unwrap(), Some(second));
    assert!(git.head(&root.path).unwrap().is_detached);
    git.test_set_ref(&root.path, "refs/heads/main", None)
        .unwrap();
    assert_eq!(git.read_ref(&root.path, "refs/heads/alias").unwrap(), None);
    git.test_set_head(&root.path, &TestHead::Attached("refs/heads/unborn".into()))
        .unwrap();
    assert_eq!(git.head(&root.path).unwrap().commit, None);
}

#[test]
fn fixture_tree_edits_mixed_reset_and_merge_conflict_match_native_semantics() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let filesystem = context.filesystem();
    let git = world.repository();
    let root = fixture(filesystem, &git);
    let base = git
        .test_create_commit_from_parent(
            &root.path,
            &git.test_create_commit(&root.path, &TestCommitSpec::from_index("empty", vec![]))
                .unwrap(),
            "base",
            &[TestCommitFileEdit {
                path: "file.txt".into(),
                bytes: Some(b"base\n".to_vec()),
            }],
        )
        .unwrap();
    let ours = git
        .test_create_commit_from_parent(
            &root.path,
            &base,
            "ours",
            &[TestCommitFileEdit {
                path: "file.txt".into(),
                bytes: Some(b"ours\n".to_vec()),
            }],
        )
        .unwrap();
    let theirs = git
        .test_create_commit_from_parent(
            &root.path,
            &base,
            "theirs",
            &[TestCommitFileEdit {
                path: "file.txt".into(),
                bytes: Some(b"theirs\n".to_vec()),
            }],
        )
        .unwrap();
    git.test_set_ref(
        &root.path,
        "refs/heads/main",
        Some(&TestRefTarget::Direct(ours.clone())),
    )
    .unwrap();
    git.test_set_head(&root.path, &TestHead::Attached("refs/heads/main".into()))
        .unwrap();
    git.test_force_checkout(&root.path, &ours).unwrap();
    write(filesystem, &root.path.join("file.txt"), b"kept worktree\n");
    git.test_reset_mixed(&root.path, &base).unwrap();
    assert_eq!(
        git.head(&root.path).unwrap().commit.as_deref(),
        Some(base.as_str())
    );
    assert_eq!(
        filesystem.read(&root.path.join("file.txt")).unwrap(),
        b"kept worktree\n"
    );

    git.test_force_checkout(&root.path, &ours).unwrap();
    let snapshot = git
        .test_seed_merge_conflict(&root.path, &ours, &theirs)
        .unwrap();
    assert_eq!(snapshot.files.len(), 1);
    assert_eq!(
        git.repository_state(&root.path).unwrap(),
        GitRepositoryState::Merge
    );
    git.abort_merge(&root.path, &ours, &theirs).unwrap();
    assert_eq!(
        git.repository_state(&root.path).unwrap(),
        GitRepositoryState::Clean
    );
    assert_eq!(
        filesystem.read(&root.path.join("file.txt")).unwrap(),
        b"ours\n"
    );
}

// gwz 1.0.19 (pull and untracked files): both backends name exactly the entries an
// update would overwrite -- a file at an added path, a file at a parent path of one, a
// directory with untracked content at an added file's path, untracked content in a
// tracked directory the update turns into a file -- leave an unrelated untracked file
// alone, and delete none of them; the refusal built from them tells the user to move
// or delete them and never mentions `--force`.
#[test]
fn pull_update_names_exactly_the_blocking_entries_and_removes_none() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let filesystem = context.filesystem();
    let git = world.repository();
    let root = fixture(filesystem, &git);
    write(filesystem, &root.path.join("base.txt"), b"base\n");
    filesystem
        .create_directories(&root.path.join("conf"))
        .unwrap();
    write(filesystem, &root.path.join("conf/a.txt"), b"a\n");
    git.stage_paths(&root.path, &["base.txt", "conf/a.txt"])
        .unwrap();
    let base = git.commit(&root.path, "base", false).unwrap().commit;
    let incoming = |path: &str| TestCommitFileEdit {
        path: path.into(),
        bytes: Some(b"incoming\n".to_vec()),
    };
    let target = git
        .test_create_commit_from_parent(
            &root.path,
            &base,
            "target",
            &[
                incoming("new.txt"),
                incoming("sub/inner.txt"),
                incoming("blocked.txt"),
                incoming("free.txt"),
                TestCommitFileEdit {
                    path: "conf/a.txt".into(),
                    bytes: None,
                },
                incoming("conf"),
            ],
        )
        .unwrap();
    write(filesystem, &root.path.join("new.txt"), b"local\n");
    write(
        filesystem,
        &root.path.join("sub"),
        b"a file where a directory comes\n",
    );
    filesystem
        .create_directories(&root.path.join("blocked.txt"))
        .unwrap();
    write(
        filesystem,
        &root.path.join("blocked.txt/mine.txt"),
        b"mine\n",
    );
    write(filesystem, &root.path.join("conf/local.txt"), b"mine\n");
    write(filesystem, &root.path.join("junk.txt"), b"junk\n");
    let status = git.status(&root.path).unwrap();
    assert!(!status.has_tracked_changes() && status.is_dirty);

    let collisions = git.update_collisions(&root.path, &target).unwrap();
    let paths = collisions
        .iter()
        .map(|collision| collision.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["blocked.txt", "conf", "new.txt", "sub/inner.txt"]);
    assert!(
        collisions
            .iter()
            .all(|collision| collision.reason.is_none()),
        "{collisions:?}"
    );
    assert_eq!(git.head(&root.path).unwrap().commit, Some(base.clone()));
    assert!(exists(filesystem, &root.path.join("new.txt")));

    // The refusal names every one, tells the user to move or delete them, and does not
    // advise `--force`, which deletes none of them. The query removed nothing.
    let refused =
        crate::git::refuse_collisions(&collisions, "would be overwritten by pull").unwrap_err();
    assert_eq!(refused.code, ErrorCode::DirtyMember, "{}", refused.message);
    assert!(
        refused.message.contains("move or delete") && !refused.message.contains("--force"),
        "{}",
        refused.message
    );
    for path in &paths {
        assert!(
            refused.message.contains(path.as_str()),
            "{}",
            refused.message
        );
    }
    for kept in ["new.txt", "sub", "blocked.txt/mine.txt", "conf/local.txt"] {
        assert!(exists(filesystem, &root.path.join(kept)), "{kept} removed");
    }
    assert_eq!(
        filesystem.read(&root.path.join("conf/a.txt")).unwrap(),
        b"a\n",
        "a tracked file was removed"
    );
    assert_eq!(
        filesystem.read(&root.path.join("junk.txt")).unwrap(),
        b"junk\n"
    );
    // Once the user has deleted the colliding entries there is nothing left to refuse,
    // and the query must still be a dry run: it writes none of the incoming files and
    // leaves HEAD where it was.
    for deleted in ["new.txt", "sub", "blocked.txt", "conf/local.txt"] {
        filesystem.remove_tree(&root.path.join(deleted)).unwrap();
    }
    assert!(
        git.update_collisions(&root.path, &target)
            .unwrap()
            .is_empty()
    );
    for incoming in ["free.txt", "new.txt", "sub", "blocked.txt"] {
        assert!(
            !exists(filesystem, &root.path.join(incoming)),
            "the collision query wrote {incoming}"
        );
    }
    assert_eq!(git.head(&root.path).unwrap().commit, Some(base.clone()));
}
