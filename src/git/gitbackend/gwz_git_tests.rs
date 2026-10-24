//! Step 5.2: what core delegates to gwz-git, and that the old paths are gone.

use crate::git::{git_error, model_error};
use crate::model::ErrorCode;

#[test]
fn a_gwz_git_error_translates_like_the_native_error_it_carries() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing = dir.path().join("missing");
    let native = git2::Repository::open(&missing).err().unwrap();
    let library = gwz_git::Repository::open_exact(&missing).err().unwrap();
    let old = git_error(native);
    let new = model_error(library);
    assert_eq!(new.code, old.code);
    assert_eq!(new.code, ErrorCode::GitCommandFailed);
    assert_eq!(new.message, old.message);
    assert!(!new.message.is_empty());

    let plain = gwz_git::ObjectId::parse_hex(gwz_git::ObjectFormat::Sha1, "zz")
        .err()
        .unwrap();
    let translated = model_error(plain);
    assert_eq!(translated.code, ErrorCode::GitCommandFailed);
    assert!(!translated.message.is_empty());
}

#[test]
fn is_repository_asks_gwz_git_and_core_no_longer_opens_the_repository_for_it() {
    // A Windows checkout may convert the line endings, so the function's closing
    // brace is found in the source with the ones the repository stores.
    let source = include_str!("repository.rs").replace("\r\n", "\n");
    let body = source
        .split("fn is_repository")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("is_repository is in repository.rs");
    assert!(
        body.contains("gwz_git::Repository::is_repository"),
        "{body}"
    );
    assert!(!body.contains("git2::"), "{body}");
}

fn source_file(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/git/gitbackend")
        .join(relative)
}

#[test]
fn native_commit_and_tag_live_in_gwz_git_and_core_keeps_no_copy() {
    // G1 moved to gwz-git: the module, its two characterization files and the
    // three Git CLI fallbacks are gone from core, not kept beside the new path.
    for gone in [
        "native_commit",
        "native_commit.rs",
        "commit_tag_characterization.rs",
        "commit_tag_identity_characterization.rs",
    ] {
        assert!(!source_file(gone).exists(), "{gone} is still in core");
    }
    for (file, forbidden) in [
        ("repository.rs", "Command::new(\"git\")"),
        ("refs.rs", "Command::new(\"git\")"),
        ("repository.rs", "fn commit_cli"),
        ("refs.rs", "fn tag_create_cli"),
        ("refs.rs", "fn tag_delete_cli"),
        ("refs.rs", "fn verify_tag_created"),
        ("refs.rs", "fn verify_tag_deleted"),
    ] {
        let text = std::fs::read_to_string(source_file(file)).unwrap();
        assert!(!text.contains(forbidden), "{file} still has {forbidden}");
    }
    let backend = std::fs::read_to_string(source_file("commit_tag.rs")).unwrap();
    for needed in [
        "use gwz_git::native_commit::{self, Services}",
        "native_commit::commit(",
        "native_commit::tag_create(",
        "native_commit::tag_delete(",
    ] {
        assert!(
            backend.contains(needed),
            "commit_tag.rs does not contain {needed}"
        );
    }
}

#[test]
fn refs_and_reads_live_in_gwz_git_and_core_keeps_no_copy() {
    // G2 moved to gwz-git (step 5.3): head, status, tag names, ref and revision
    // reads, commit existence, committed file content, the commit-graph reads
    // and the configured remotes. Core's old implementations are gone, not kept
    // beside the new path, and the helpers only they used went with them.
    for (file, forbidden) in [
        ("refs.rs", "fn tag_list("),
        ("refs.rs", "fn read_ref("),
        ("refs.rs", "fn ahead_behind("),
        ("refs.rs", "fn is_ancestor("),
        ("repository.rs", "fn commit_exists("),
        ("repository.rs", "fn read_file_at_commit("),
        ("repository.rs", "fn status("),
        ("repository.rs", "fn status_with_options("),
        ("repository.rs", "fn head("),
        ("comparison.rs", "fn merge_base("),
        ("comparison.rs", "fn changed_paths_between("),
        ("transport.rs", "fn remotes("),
        ("repository_support.rs", "fn git_file_status("),
        ("repository_support.rs", "fn rename_delta("),
    ] {
        let text = std::fs::read_to_string(source_file(file)).unwrap();
        assert!(!text.contains(forbidden), "{file} still has {forbidden}");
    }
    let git_dir = source_file("..");
    for gone in ["index_status_char.rs", "worktree_status_char.rs"] {
        assert!(!git_dir.join(gone).exists(), "{gone} is still in core");
    }
    let checkout = std::fs::read_to_string(git_dir.join("verify_checkout_state.rs")).unwrap();
    assert!(
        !checkout.contains("fn unborn_head("),
        "unborn_head is still in core"
    );
    assert!(
        !checkout.contains("repo.head()"),
        "repo_head still reads HEAD itself"
    );

    let reads = std::fs::read_to_string(source_file("reads.rs")).unwrap();
    for needed in [
        "Repository::open_exact(path)",
        ".head()",
        ".tag_names()",
        ".resolve(",
        ".commit_exists(",
        ".blob_at(",
        ".is_ancestor(",
        ".ahead_behind(",
        ".merge_base(",
        ".changed_paths(",
        ".status(",
        ".remotes()",
    ] {
        assert!(reads.contains(needed), "reads.rs does not contain {needed}");
    }
    assert!(!reads.contains("git2::"), "reads.rs names git2");
}

#[test]
fn the_reads_delegate_to_gwz_git_and_project_its_records_onto_the_model() {
    use crate::git::tests::commit_file;
    use crate::git::{GitBackend, GitStatusOptions};

    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("repo");
    let backend = crate::git::Git2Backend::new();
    backend.create_repo(&path).unwrap();

    // Unborn: the branch HEAD names and no commit; no ref reads as absent.
    let unborn = backend.head(&path).unwrap();
    assert_eq!(unborn.branch.as_deref(), Some("main"));
    assert_eq!((unborn.commit, unborn.is_detached), (None, false));
    assert_eq!(backend.read_ref(&path, "HEAD").unwrap(), None);
    assert!(backend.tag_list(&path).unwrap().is_empty());

    let first = commit_file(&path, "a.txt", "one", "first", &[]).unwrap();
    let first_oid = git2::Oid::from_str(&first).unwrap();
    let second = commit_file(&path, "b.txt", "two", "second", &[first_oid]).unwrap();

    // Ids come back as the full lowercase text the model carries.
    assert_eq!(
        backend.head(&path).unwrap().commit.as_deref(),
        Some(second.as_str())
    );
    assert_eq!(
        backend
            .read_ref(&path, "refs/heads/main")
            .unwrap()
            .as_deref(),
        Some(second.as_str())
    );
    assert_eq!(backend.read_ref(&path, "refs/heads/absent").unwrap(), None);
    let native = git2::Repository::open(&path).unwrap();
    let object = native.find_object(first_oid, None).unwrap();
    native.tag_lightweight("v2", &object, false).unwrap();
    native.tag_lightweight("v1", &object, false).unwrap();
    assert_eq!(backend.tag_list(&path).unwrap(), vec!["v1", "v2"]);

    // Commit existence: text that is not an id is a plain `false`.
    assert!(backend.commit_exists(&path, &first).unwrap());
    assert!(!backend.commit_exists(&path, "not-an-object-id").unwrap());
    assert!(!backend.commit_exists(&path, &"0".repeat(40)).unwrap());

    // Committed content; the path refusal keeps core's InvalidRequest code.
    assert_eq!(
        backend.read_file_at_commit(&path, &first, "a.txt").unwrap(),
        Some(b"one".to_vec())
    );
    assert_eq!(
        backend.read_file_at_commit(&path, &first, "b.txt").unwrap(),
        None
    );
    assert_eq!(
        backend
            .read_file_at_commit(&path, &first, "../outside")
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        backend
            .read_file_at_commit(&path, "not-an-object-id", "a.txt")
            .unwrap_err()
            .code,
        ErrorCode::GitCommandFailed
    );

    // The commit graph.
    assert!(backend.is_ancestor(&path, &first, &second).unwrap());
    assert!(!backend.is_ancestor(&path, &second, &first).unwrap());
    let counts = backend.ahead_behind(&path, &second, &first).unwrap();
    assert_eq!((counts.ahead, counts.behind), (1, 0));
    assert_eq!(
        backend
            .merge_base(&path, &first, &second)
            .unwrap()
            .as_deref(),
        Some(first.as_str())
    );
    assert_eq!(
        backend
            .changed_paths_between(&path, &first, &second)
            .unwrap(),
        vec!["b.txt"]
    );

    // Status: the characters are model strings, the counts and files carry over.
    std::fs::write(path.join("c.txt"), "new").unwrap();
    std::fs::create_dir_all(path.join("ignored")).unwrap();
    std::fs::write(path.join(".gitignore"), "ignored/\n").unwrap();
    std::fs::write(path.join("ignored/x"), "x").unwrap();
    let status = backend.status(&path).unwrap();
    assert!(status.is_dirty);
    assert_eq!(status.untracked, 2);
    assert!(status.files.iter().any(|file| {
        file.path == "c.txt" && file.index_status == " " && file.worktree_status == "?"
    }));
    assert_eq!(status.ignored, 0);
    let with_ignored = backend
        .status_with_options(&path, GitStatusOptions::include_ignored())
        .unwrap();
    assert_eq!(with_ignored.ignored, 1);

    // Remotes: the url the model wraps in Some, the push url as is.
    backend
        .add_remote(&path, "origin", "file:///tmp/origin.git")
        .unwrap();
    assert_eq!(
        backend.remotes(&path).unwrap(),
        vec![crate::git::GitRemote {
            name: "origin".to_owned(),
            url: Some("file:///tmp/origin.git".to_owned()),
            push_url: None,
        }]
    );
}

#[test]
fn a_read_of_a_path_that_is_not_a_repository_translates_like_the_native_open_did() {
    use crate::git::GitBackend;

    let dir = tempfile::TempDir::new().unwrap();
    let missing = dir.path().join("missing");
    let backend = crate::git::Git2Backend::new();
    let native = git_error(git2::Repository::open(&missing).err().unwrap());
    for error in [
        backend.head(&missing).unwrap_err(),
        backend.status(&missing).unwrap_err(),
        backend.tag_list(&missing).unwrap_err(),
        backend.remotes(&missing).unwrap_err(),
        backend.read_ref(&missing, "HEAD").unwrap_err(),
    ] {
        assert_eq!(error.code, native.code);
        assert_eq!(error.message, native.message);
    }
}

#[test]
fn repository_paths_and_index_live_in_gwz_git_and_core_keeps_no_copy() {
    // Step 5.5 moved the last two reads of row G2 that were still inline in
    // `gitbackend.rs`: the storage locations and the index snapshot.
    let backend = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/git/gitbackend.rs"),
    )
    .unwrap();
    for forbidden in [
        "repo.index()",
        "repo.commondir()",
        "repo.workdir()",
        "index.iter()",
    ] {
        assert!(
            !backend.contains(forbidden),
            "gitbackend.rs still has {forbidden}"
        );
    }
    let reads = std::fs::read_to_string(source_file("reads.rs")).unwrap();
    for needed in [
        "fn repository_paths(",
        "fn repository_index(",
        ".index_snapshot()",
        ".git_dir()",
        ".common_dir()",
        ".work_dir()",
    ] {
        assert!(reads.contains(needed), "reads.rs does not contain {needed}");
    }
    assert!(!reads.contains("git2::"), "reads.rs names git2");
}

#[test]
fn repository_paths_and_index_delegate_to_gwz_git_and_match_the_native_values() {
    use crate::git::GitBackend;
    use crate::git::tests::commit_file;

    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("repo");
    let backend = crate::git::Git2Backend::new();
    backend.create_repo(&path).unwrap();
    let first = commit_file(&path, "a.txt", "one", "first", &[]).unwrap();
    std::fs::write(path.join("staged.txt"), "staged\n").unwrap();
    let native = git2::Repository::open(&path).unwrap();
    let mut index = native.index().unwrap();
    index.add_path(std::path::Path::new("staged.txt")).unwrap();
    index.write().unwrap();

    // Storage locations: the main worktree, then a linked worktree whose
    // common directory is the main repository's.
    let paths = backend.repository_paths(&path).unwrap();
    assert_eq!(paths.worktree.as_deref(), native.workdir());
    assert_eq!(paths.git_dir, native.path());
    assert_eq!(paths.common_dir, native.commondir());
    let linked_path = dir.path().join("linked");
    let object = native
        .find_commit(git2::Oid::from_str(&first).unwrap())
        .unwrap();
    let branch = native.branch("side", &object, false).unwrap();
    let mut options = git2::WorktreeAddOptions::new();
    options.reference(Some(branch.get()));
    native
        .worktree("side", &linked_path, Some(&options))
        .unwrap();
    let linked = backend.repository_paths(&linked_path).unwrap();
    let linked_native = git2::Repository::open(&linked_path).unwrap();
    assert_eq!(linked.git_dir, linked_native.path());
    assert_eq!(linked.common_dir, native.commondir());
    assert_ne!(linked.git_dir, linked.common_dir);
    let bare_path = dir.path().join("bare.git");
    git2::Repository::init_bare(&bare_path).unwrap();
    assert_eq!(backend.repository_paths(&bare_path).unwrap().worktree, None);

    // The index: every entry the native index holds, ids as raw bytes.
    let snapshot = backend.repository_index(&path).unwrap();
    let native_index = native.index().unwrap();
    assert_eq!(snapshot.path.as_deref(), native_index.path());
    assert_eq!(snapshot.entries.len(), 2);
    for (entry, expected) in snapshot.entries.iter().zip(native_index.iter()) {
        assert_eq!(entry.path, expected.path);
        assert_eq!(entry.object_id, expected.id.as_bytes());
        assert_eq!(entry.mode, expected.mode);
        assert_eq!(entry.flags, expected.flags);
        assert_eq!(entry.flags_extended, expected.flags_extended);
        assert_eq!(
            entry.ctime,
            (expected.ctime.seconds(), expected.ctime.nanoseconds())
        );
        assert_eq!(
            entry.mtime,
            (expected.mtime.seconds(), expected.mtime.nanoseconds())
        );
        assert_eq!(
            entry.stat,
            [
                expected.dev,
                expected.ino,
                expected.uid,
                expected.gid,
                expected.file_size
            ]
        );
    }

    // A path that is not a repository translates like the native open did.
    let missing = dir.path().join("missing");
    let expected = git_error(git2::Repository::open(&missing).err().unwrap());
    for error in [
        backend.repository_paths(&missing).unwrap_err(),
        backend.repository_index(&missing).unwrap_err(),
    ] {
        assert_eq!(error.code, expected.code);
        assert_eq!(error.message, expected.message);
    }
}
