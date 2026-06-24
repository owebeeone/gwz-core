use super::*;

// GWZTagPlan Phase 1: local git-tag primitives — create (lightweight / annotated), list, delete.

fn repo_with_commit(temp: &TempDir, backend: &Git2Backend) -> (std::path::PathBuf, String) {
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    run_git(&repo, &["config", "user.name", "GWZ"]);
    run_git(&repo, &["config", "user.email", "gwz@example.invalid"]);
    let base = commit_file(&repo, "f.txt", "x\n", "base", &[]).unwrap();
    (repo, base)
}

#[test]
fn tag_create_lightweight_then_list_and_delete() {
    let temp = TempDir::new("tag-prim");
    let backend = Git2Backend::new();
    let (repo, base) = repo_with_commit(&temp, &backend);

    assert!(backend.tag_list(&repo).unwrap().is_empty());

    let result = backend.tag_create(&repo, "v1", None, false).unwrap();
    assert_eq!(result.name, "v1");
    assert_eq!(result.commit, base, "lightweight tag points at HEAD commit");
    assert_eq!(backend.tag_list(&repo).unwrap(), vec!["v1".to_owned()]);

    backend.tag_delete(&repo, "v1").unwrap();
    assert!(backend.tag_list(&repo).unwrap().is_empty(), "tag gone after delete");
}

#[test]
fn tag_create_annotated_carries_message() {
    let temp = TempDir::new("tag-annot");
    let backend = Git2Backend::new();
    let (repo, base) = repo_with_commit(&temp, &backend);

    let result = backend
        .tag_create(&repo, "rel", Some("release one"), false)
        .unwrap();
    assert!(backend.tag_list(&repo).unwrap().contains(&"rel".to_owned()));
    assert_eq!(result.commit, base, "result commit is the peeled target");

    // An annotated tag creates a real tag object (a lightweight tag would resolve straight
    // to a commit instead).
    let opened = git2::Repository::open(&repo).unwrap();
    let object = opened.revparse_single("refs/tags/rel").unwrap();
    let tag = object.as_tag().expect("annotated tag is a tag object");
    assert_eq!(
        tag.message().unwrap().map(str::trim),
        Some("release one"),
        "annotation message round-trips"
    );
}

#[test]
fn tag_create_errors_on_duplicate() {
    let temp = TempDir::new("tag-dup");
    let backend = Git2Backend::new();
    let (repo, _base) = repo_with_commit(&temp, &backend);

    backend.tag_create(&repo, "v1", None, false).unwrap();
    assert!(
        backend.tag_create(&repo, "v1", None, false).is_err(),
        "creating a duplicate tag must fail"
    );
}

#[test]
fn tag_list_is_sorted() {
    let temp = TempDir::new("tag-sort");
    let backend = Git2Backend::new();
    let (repo, _base) = repo_with_commit(&temp, &backend);

    backend.tag_create(&repo, "b", None, false).unwrap();
    backend.tag_create(&repo, "a", None, false).unwrap();
    assert_eq!(
        backend.tag_list(&repo).unwrap(),
        vec!["a".to_owned(), "b".to_owned()]
    );
}

#[test]
fn tag_fetch_brings_remote_tags_local() {
    let temp = TempDir::new("tag-fetch");
    let backend = Git2Backend::new();

    // A bare remote, plus repo A that publishes main to it.
    let bare = temp.path().join("remote.git");
    init_bare_main(&bare);
    let repo_a = temp.path().join("a");
    backend.create_repo(&repo_a).unwrap();
    run_git(&repo_a, &["config", "user.name", "GWZ"]);
    run_git(&repo_a, &["config", "user.email", "gwz@example.invalid"]);
    commit_file(&repo_a, "f.txt", "x\n", "base", &[]).unwrap();
    backend.add_remote(&repo_a, "origin", bare.to_str().unwrap()).unwrap();
    backend
        .push(&repo_a, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();

    // Repo B clones before the tag exists, so it has no tags yet.
    let repo_b = temp.path().join("b");
    backend.clone_repo(bare.to_str().unwrap(), &repo_b).unwrap();
    assert!(backend.tag_list(&repo_b).unwrap().is_empty());

    // Repo A creates a tag and pushes it (push reused for tags).
    backend.tag_create(&repo_a, "v1", None, false).unwrap();
    backend
        .push(&repo_a, "origin", "refs/tags/v1:refs/tags/v1")
        .unwrap();

    // Repo B fetches it.
    backend.tag_fetch(&repo_b, "origin").unwrap();
    assert!(
        backend.tag_list(&repo_b).unwrap().contains(&"v1".to_owned()),
        "remote tag fetched into the local repo"
    );
}
