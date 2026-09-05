//! Every constructor is exercised by a test that reads back, from disk, the
//! hazard or work state it claims to make — with `git2` where Git itself is
//! the authority on the answer, and with plain file reads where a hazard has
//! deliberately left a layout `git2` will not open.
//!
//! Tier A: tiny fixtures, no sleeps, no process spawn, no network.

use std::path::Path;

use git2::{IndexEntryExtendedFlag, IndexEntryFlag, RepositoryState, Status};
use gwz_repo_contract::{
    HeadState, NativeOperation, ObjectFormat, ObjectKind, RepoKey, RootSource, SuppressionFlag,
};

use crate::{
    FIXTURE_EMAIL, FIXTURE_NAME, FIXTURE_TIME_SECONDS, IMPORT_REF_PREFIX, RepoSpec, TempTree,
    TestRepo, modes,
};

/// One commit holding one file, the shape most fixtures start from.
fn seeded(temp: &TempTree, name: &str) -> TestRepo {
    let repo = temp.repo(name);
    repo.commit_files("first", &[("README", b"hello\n")]);
    repo
}

// ---- construction and identity -----------------------------------------

#[test]
fn init_builds_both_object_formats_and_reports_them() {
    let temp = TempTree::new("formats");
    for (name, format) in [("one", ObjectFormat::Sha1), ("two", ObjectFormat::Sha256)] {
        let repo = temp.repo_with(name, &RepoSpec::new().format(format));
        assert_eq!(repo.object_format(), format);
        assert_eq!(repo.spec().format, format);
        let commit = repo.commit_files("first", &[("README", b"hello\n")]);
        assert_eq!(commit.format(), format);
        assert_eq!(commit.as_bytes().len(), format.digest_len());
        assert_eq!(commit.to_hex().len(), format.digest_len() * 2);
        assert!(repo.contains_object(&commit));
    }
}

#[test]
fn bare_and_non_bare_layouts_report_their_own_paths() {
    let temp = TempTree::new("layouts");
    let checkout = seeded(&temp, "checkout");
    assert!(!checkout.is_bare());
    assert_eq!(checkout.workdir().as_deref(), Some(checkout.path()));
    assert_eq!(checkout.git_dir(), checkout.path().join(".git"));
    assert_eq!(checkout.common_dir(), checkout.git_dir());
    assert!(checkout.path().join("README").is_file());

    let hub = temp.bare_repo("hub");
    hub.commit_files("first", &[("README", b"hub\n")]);
    assert!(hub.is_bare());
    assert_eq!(hub.workdir(), None);
    assert_eq!(hub.git_dir(), hub.path());
    assert!(
        !hub.path().join("README").exists(),
        "a bare hub has no worktree"
    );
}

#[test]
fn the_fixture_identity_and_time_are_the_documented_ones() {
    let temp = TempTree::new("identity");
    let repo = seeded(&temp, "one");
    let repository = repo.open();
    let commit = repository.head().unwrap().peel_to_commit().unwrap();
    for signature in [commit.author(), commit.committer()] {
        assert_eq!(signature.name(), Ok(FIXTURE_NAME));
        assert_eq!(signature.email(), Ok(FIXTURE_EMAIL));
        assert_eq!(signature.when().seconds(), FIXTURE_TIME_SECONDS);
        assert_eq!(signature.when().offset_minutes(), 0);
    }
}

// ---- determinism --------------------------------------------------------

#[test]
fn the_same_fixture_built_twice_yields_the_same_ids() {
    fn build(label: &str, format: ObjectFormat) -> Vec<String> {
        let temp = TempTree::new(label);
        let repo = temp.repo_with("work", &RepoSpec::new().format(format));
        let first = repo.commit_files("first", &[("README", b"hello\n")]);
        let second = repo.commit_files("second", &[("src/main.rs", b"fn main() {}\n")]);
        let tag = repo.annotated_tag("v1", &second);
        repo.work_untracked("scratch.txt", b"scratch\n");
        repo.work_unstaged("README", b"changed\n");
        let stash = repo.work_stash("fixture stash");
        // The tree is dropped with `temp`, so the ids can only come from the
        // fixed identity, time and content.
        vec![
            first.to_hex(),
            second.to_hex(),
            tag.to_hex(),
            stash.to_hex(),
        ]
    }

    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        let once = build("determinism-a", format);
        let twice = build("determinism-b", format);
        assert_eq!(once, twice, "{format:?} fixture ids must not vary");
        assert!(once.iter().all(|id| id.len() == format.digest_len() * 2));
    }
}

#[test]
fn the_two_object_formats_do_not_share_ids() {
    let temp = TempTree::new("formats-differ");
    let sha1 = temp.repo_with("one", &RepoSpec::new());
    let sha256 = temp.repo_with("two", &RepoSpec::new().sha256());
    let first = sha1.commit_files("first", &[("README", b"hello\n")]);
    let second = sha256.commit_files("first", &[("README", b"hello\n")]);
    assert_ne!(first.to_hex(), second.to_hex());
    assert_ne!(first.format(), second.format());
}

// ---- index, commits, refs ----------------------------------------------

#[test]
fn committing_the_index_keeps_deliberate_unstaged_work() {
    let temp = TempTree::new("index");
    let repo = seeded(&temp, "one");
    repo.work_staged("staged.txt", b"staged\n");
    repo.write_file("README", b"unstaged\n");
    repo.commit_index("second");
    assert!(repo.is_tracked("staged.txt"));
    assert_eq!(repo.read_file("README"), b"unstaged\n");
    assert!(repo.status_of("README").contains(Status::WT_MODIFIED));
}

#[test]
fn commit_removing_drops_a_path_from_the_tree() {
    let temp = TempTree::new("remove");
    let repo = seeded(&temp, "one");
    repo.commit_files("second", &[("doomed.txt", b"bye\n")]);
    assert!(repo.is_tracked("doomed.txt"));
    repo.commit_removing("third", &["doomed.txt"]);
    assert!(!repo.is_tracked("doomed.txt"));
    assert!(!repo.path().join("doomed.txt").exists());
}

#[test]
fn branches_detached_and_unborn_heads_are_all_constructible() {
    let temp = TempTree::new("heads");
    let repo = seeded(&temp, "one");
    let first = repo.head_id();
    assert_eq!(
        repo.head_state(),
        HeadState::Attached {
            branch: "main".to_owned(),
            target: first.clone()
        }
    );

    let name = repo.branch("lane/agent-17", &first);
    assert_eq!(name, "refs/heads/lane/agent-17");
    assert_eq!(repo.ref_target(&name), Some(first.clone()));
    repo.checkout_branch("lane/agent-17");
    assert!(matches!(
        repo.head_state(),
        HeadState::Attached { ref branch, .. } if branch == "lane/agent-17"
    ));

    repo.detach_head(&first);
    assert_eq!(
        repo.head_state(),
        HeadState::Detached {
            target: first.clone()
        }
    );

    repo.unborn_head("fresh");
    assert_eq!(
        repo.head_state(),
        HeadState::Unborn {
            branch: "fresh".to_owned()
        }
    );
}

#[test]
fn lightweight_and_annotated_tags_differ_by_one_real_object() {
    let temp = TempTree::new("tags");
    let repo = seeded(&temp, "one");
    let commit = repo.head_id();

    let light = repo.lightweight_tag("light", &commit);
    assert_eq!(light, commit, "a lightweight tag is not its own object");
    assert_eq!(repo.ref_target("refs/tags/light"), Some(commit.clone()));

    let annotated = repo.annotated_tag("v1", &commit);
    assert_ne!(annotated, commit, "an annotated tag is its own object");
    assert_eq!(repo.ref_target("refs/tags/v1"), Some(annotated.clone()));
    let record = repo.object_record(&annotated);
    assert_eq!(record.kind, ObjectKind::Tag);
    assert_eq!(record.edges, vec![commit]);
}

#[test]
fn a_reflog_entry_retains_a_commit_no_ref_names() {
    let temp = TempTree::new("reflog");
    let repo = seeded(&temp, "one");
    let first = repo.head_id();
    let second = repo.commit_files("second", &[("second.txt", b"second\n")]);

    repo.reset_branch("main", &first);
    assert_eq!(repo.ref_target("refs/heads/main"), Some(first.clone()));
    assert!(
        !repo
            .ref_names()
            .iter()
            .any(|name| repo.ref_target(name) == Some(second.clone())),
        "the dropped tip must be named by no ref"
    );
    assert!(repo.contains_object(&second), "its objects are still there");

    let entries = repo.reflog_entry("refs/heads/main", &second, "fixture retained root");
    assert!(entries > 1);
    let repository = repo.open();
    let reflog = repository.reflog("refs/heads/main").unwrap();
    let newest = reflog.get(0).expect("the appended entry");
    assert_eq!(repo.oid(newest.id_new()), second);
    assert_eq!(newest.message(), Ok(Some("fixture retained root")));
}

// ---- remotes and transfer ----------------------------------------------

#[test]
fn a_repository_can_have_a_named_remote_and_none() {
    let temp = TempTree::new("remotes");
    let repo = seeded(&temp, "one");
    let hub = temp.bare_repo("hub");
    assert_eq!(repo.remote_names(), Vec::<String>::new());
    repo.remote("hub", &hub);
    assert_eq!(repo.remote_names(), vec!["hub".to_owned()]);
    repo.remove_remote("hub");
    assert_eq!(repo.remote_names(), Vec::<String>::new());
}

#[test]
fn import_ref_fetches_into_the_retained_import_namespace() {
    let temp = TempTree::new("import");
    let source = temp.repo("source");
    let lane = source.commit_files("lane work", &[("LANE", b"lane\n")]);
    let receiver = seeded(&temp, "receiver");
    assert!(!receiver.contains_object(&lane));

    let received = receiver.import_ref(&source, "t1", "refs/heads/main");
    assert_eq!(received, lane);
    let name = format!("{IMPORT_REF_PREFIX}/t1");
    assert_eq!(receiver.ref_target(&name), Some(lane.clone()));
    assert!(receiver.ref_names().contains(&name));
    assert!(
        receiver.remote_names().is_empty(),
        "an anonymous fetch persists no remote (design §6.2)"
    );
    assert!(receiver.contains_object(&lane));
}

#[test]
fn push_publishes_a_branch_into_a_bare_hub() {
    let temp = TempTree::new("push");
    let source = seeded(&temp, "source");
    let head = source.head_id();
    let hub = temp.bare_repo("hub");
    source.push(&hub, &["+refs/heads/main:refs/heads/lane/from-a"]);
    assert_eq!(hub.ref_target("refs/heads/lane/from-a"), Some(head));
}

// ---- observation --------------------------------------------------------

#[test]
fn object_records_carry_kind_size_and_the_contract_s_edges() {
    let temp = TempTree::new("records");
    let repo = seeded(&temp, "one");
    let first = repo.head_id();
    let second = repo.commit_files("second", &[("src/main.rs", b"fn main() {}\n")]);

    let commit = repo.object_record(&second);
    assert_eq!(commit.kind, ObjectKind::Commit);
    assert!(commit.size > 0);
    assert_eq!(commit.edges.len(), 2, "tree then parent");
    assert_eq!(commit.edges[1], first);

    let tree = repo.object_record(&commit.edges[0]);
    assert_eq!(tree.kind, ObjectKind::Tree);
    assert_eq!(tree.edges.len(), 2, "README and src/");

    let blob = repo.object_record(
        &repo
            .reachable_objects()
            .into_iter()
            .find(|record| record.kind == ObjectKind::Blob && record.size == 6)
            .expect("the README blob")
            .oid,
    );
    assert_eq!(blob.kind, ObjectKind::Blob);
    assert_eq!(blob.size, 6);
    assert!(blob.edges.is_empty());
}

#[test]
fn reachable_objects_are_children_first_and_the_absent_id_is_absent() {
    let temp = TempTree::new("reachable");
    let repo = seeded(&temp, "one");
    let records = repo.reachable_objects();
    assert_eq!(records.len(), 3, "blob, tree, commit");
    assert_eq!(
        records.iter().map(|record| record.kind).collect::<Vec<_>>(),
        vec![ObjectKind::Blob, ObjectKind::Tree, ObjectKind::Commit]
    );
    assert_eq!(records, repo.reachable_objects(), "the order is stable");

    let missing = repo.absent_object_id();
    assert_eq!(missing.format(), ObjectFormat::Sha1);
    assert!(!repo.contains_object(&missing));
}

#[test]
fn protected_roots_report_head_then_every_ref_in_name_order() {
    let temp = TempTree::new("roots");
    let repo = seeded(&temp, "one");
    let head = repo.head_id();
    let tag = repo.annotated_tag("v1", &head);

    let roots = repo.protected_roots();
    assert_eq!(roots.roots[0].source, RootSource::Head);
    assert_eq!(roots.roots[0].oid, head);
    let named: Vec<(String, String)> = roots.roots[1..]
        .iter()
        .map(|root| match &root.source {
            RootSource::Ref { name } => (name.clone(), root.oid.to_hex()),
            other => panic!("unexpected root {other:?}"),
        })
        .collect();
    assert_eq!(
        named,
        vec![
            ("refs/heads/main".to_owned(), head.to_hex()),
            ("refs/tags/v1".to_owned(), tag.to_hex()),
        ]
    );
}

#[cfg(any(test, feature = "contract-tests"))]
#[test]
fn the_graph_fixture_shape_holds_for_a_real_repository() {
    use gwz_repo_contract::contract_tests::GraphFixture;

    let temp = TempTree::new("graph");
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        let repo = temp.repo_with(&format!("{format:?}"), &RepoSpec::new().format(format));
        repo.commit_files("first", &[("README", b"hello\n")]);
        let fixture: GraphFixture = repo.graph_fixture();
        assert_eq!(fixture.objects.len(), 3);
        assert!(fixture.objects.iter().all(|record| record.size > 0));
        assert_eq!(fixture.roots, repo.protected_roots());
        assert_eq!(fixture.missing.format(), format);
        assert!(!repo.contains_object(&fixture.missing));
        // Every object the fixture claims really is served by the repository
        // with those exact edges: the invariant `object_reader_conformance`
        // holds a real reader to.
        for record in &fixture.objects {
            assert_eq!(&repo.object_record(&record.oid), record);
        }
    }
}

// ---- design §4.0 layout hazards -----------------------------------------

#[test]
fn hazard_git_file_replaces_the_git_directory_with_a_gitfile() {
    let temp = TempTree::new("gitfile");
    let repo = seeded(&temp, "one");
    let relocated = repo.hazard_git_file();

    let dot_git = repo.path().join(".git");
    assert!(dot_git.is_file(), ".git must be a file, not a directory");
    let pointer = String::from_utf8(crate::read_file(&dot_git)).unwrap();
    assert_eq!(pointer, format!("gitdir: {}\n", relocated.display()));
    assert!(relocated.join("HEAD").is_file());
    assert_eq!(repo.git_dir(), relocated);
}

#[test]
fn hazard_external_common_dir_points_the_commondir_outside_the_member() {
    let temp = TempTree::new("commondir");
    let repo = seeded(&temp, "one");
    let outside = temp.join("outside/common");
    let marker = repo.hazard_external_common_dir(&outside);
    assert_eq!(marker, repo.layout_git_dir().join("commondir"));
    let recorded = String::from_utf8(crate::read_file(&marker)).unwrap();
    assert_eq!(recorded.trim_end(), outside.to_string_lossy());
    assert!(
        !outside.starts_with(repo.path()),
        "the common directory must be outside the member"
    );
}

#[test]
fn hazard_alternates_and_http_alternates_name_a_borrowed_object_store() {
    let temp = TempTree::new("alternates");
    let repo = seeded(&temp, "one");
    let borrowed = temp.join("outside/objects");

    let alternates = repo.hazard_alternates(&borrowed);
    assert_eq!(
        alternates,
        repo.layout_git_dir().join("objects/info/alternates")
    );
    let recorded = String::from_utf8(crate::read_file(&alternates)).unwrap();
    assert_eq!(recorded.trim_end(), borrowed.to_string_lossy());
    assert!(borrowed.is_dir());

    let http = repo.hazard_http_alternates("https://example.invalid/objects/");
    assert_eq!(
        http,
        repo.layout_git_dir().join("objects/info/http-alternates")
    );
    assert_eq!(
        String::from_utf8(crate::read_file(&http)).unwrap(),
        "https://example.invalid/objects/\n"
    );
}

#[cfg(unix)]
#[test]
fn hazard_escaping_object_store_link_moves_objects_out_through_a_symlink() {
    let temp = TempTree::new("objects-link");
    let repo = seeded(&temp, "one");
    let head = repo.head_id();
    let outside = temp.join("outside/objects");
    repo.hazard_escaping_object_store_link(&outside);

    let objects = repo.layout_git_dir().join("objects");
    assert!(objects.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(&objects).unwrap(), outside);
    assert!(!outside.starts_with(repo.path()));
    assert!(
        repo.contains_object(&head),
        "the repository still resolves through the link, which is the hazard"
    );
}

#[cfg(unix)]
#[test]
fn hazard_escaping_metadata_link_moves_the_git_directory_out_through_a_symlink() {
    let temp = TempTree::new("metadata-link");
    let repo = seeded(&temp, "one");
    let outside = temp.join("outside/metadata");
    repo.hazard_escaping_metadata_link(&outside);

    let dot_git = repo.path().join(".git");
    assert!(dot_git.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(&dot_git).unwrap(), outside);
    assert!(outside.join("HEAD").is_file());
    assert!(!outside.starts_with(repo.path()));
}

#[test]
fn hazard_core_worktree_names_a_directory_outside_the_destination() {
    let temp = TempTree::new("core-worktree");
    let repo = seeded(&temp, "one");
    let outside = temp.join("outside/tree");
    repo.hazard_core_worktree(&outside);
    assert_eq!(
        repo.config_value("core.worktree").as_deref(),
        Some(outside.to_string_lossy().as_ref())
    );
    assert!(!outside.starts_with(repo.path()));
}

#[test]
fn hazard_hooks_paths_cover_absolute_escaping_internal_and_unresolvable() {
    let temp = TempTree::new("hooks");

    let escaping_absolute = seeded(&temp, "absolute");
    let outside = temp.join("outside/hooks");
    escaping_absolute.hazard_absolute_hooks_path(&outside);
    assert_eq!(
        escaping_absolute.config_value("core.hooksPath").as_deref(),
        Some(outside.to_string_lossy().as_ref())
    );
    assert!(outside.is_dir());
    assert!(!outside.starts_with(escaping_absolute.path()));

    let escaping_relative = seeded(&temp, "relative-out");
    let resolved = escaping_relative.hazard_relative_hooks_path("../shared-hooks");
    assert_eq!(
        escaping_relative.config_value("core.hooksPath").as_deref(),
        Some("../shared-hooks")
    );
    assert!(resolved.is_dir());
    assert_eq!(
        resolved.canonicalize().unwrap(),
        temp.path().canonicalize().unwrap().join("shared-hooks"),
        "the relative path resolves outside the repository"
    );

    let internal = seeded(&temp, "relative-in");
    let inside = internal.hazard_relative_hooks_path("hooks");
    assert_eq!(
        internal.config_value("core.hooksPath").as_deref(),
        Some("hooks")
    );
    assert!(inside.starts_with(internal.path()), "this one stays inside");

    let unresolvable = seeded(&temp, "unresolvable");
    let blocker = unresolvable.hazard_unresolvable_hooks_path();
    assert!(blocker.is_file(), "a regular file blocks the path");
    let configured = unresolvable.config_value("core.hooksPath").expect("set");
    assert!(configured.starts_with(blocker.to_string_lossy().as_ref()));
    assert!(!Path::new(&configured).exists());
}

#[test]
fn hazard_include_path_and_include_if_name_configuration_outside() {
    let temp = TempTree::new("includes");

    let plain = seeded(&temp, "plain");
    let included = temp.join("outside/extra.gitconfig");
    plain.hazard_include_path(&included);
    assert!(included.is_file());
    assert!(
        plain
            .config_text()
            .contains(&format!("path = {}", included.display())),
        "{}",
        plain.config_text()
    );
    assert_eq!(
        plain.config_value("gwz.fixture").as_deref(),
        Some("included"),
        "the include really takes effect"
    );

    let conditional = seeded(&temp, "conditional");
    let other = temp.join("outside/conditional.gitconfig");
    conditional.hazard_include_if("gitdir:/nowhere/", &other);
    assert!(other.is_file());
    let text = conditional.config_text();
    assert!(text.contains("[includeIf \"gitdir:/nowhere/\"]"), "{text}");
    assert!(
        text.contains(&format!("path = {}", other.display())),
        "{text}"
    );
}

#[test]
fn hazard_url_insteadof_rewrites_a_remote_prefix() {
    let temp = TempTree::new("insteadof");
    let repo = seeded(&temp, "one");
    let outside = temp.join("outside/mirror/");
    repo.hazard_url_insteadof(
        "https://example.invalid/",
        outside.to_string_lossy().as_ref(),
    );
    let key = format!("url.{}.insteadOf", outside.to_string_lossy());
    assert_eq!(
        repo.config_value(&key).as_deref(),
        Some("https://example.invalid/")
    );
}

#[test]
fn hazard_partial_clone_declares_a_promisor_remote_and_marker() {
    let temp = TempTree::new("promisor");
    let repo = seeded(&temp, "one");
    let marker = repo.hazard_partial_clone("origin");
    assert!(marker.is_file());
    assert_eq!(
        marker.extension().and_then(|e| e.to_str()),
        Some("promisor")
    );
    let text = repo.config_text();
    assert!(text.contains("partialClone = origin"), "{text}");
    assert!(text.contains("promisor = true"), "{text}");
    assert!(text.contains("partialclonefilter = blob:none"), "{text}");
    assert!(text.contains("repositoryformatversion = 1"), "{text}");
}

// ---- design §5.1 work states -------------------------------------------

#[test]
fn staged_unstaged_untracked_and_ignored_are_each_distinguishable() {
    let temp = TempTree::new("work");
    let repo = seeded(&temp, "one");

    repo.work_staged("staged.txt", b"staged\n");
    assert!(repo.status_of("staged.txt").contains(Status::INDEX_NEW));

    repo.work_unstaged("README", b"changed\n");
    assert!(repo.status_of("README").contains(Status::WT_MODIFIED));

    repo.work_untracked("untracked.txt", b"untracked\n");
    assert!(repo.status_of("untracked.txt").contains(Status::WT_NEW));

    repo.work_ignored("ignored.log", b"ignored\n");
    assert!(
        repo.path().join("ignored.log").is_file(),
        "the data is there"
    );
    assert!(repo.status_of("ignored.log").contains(Status::IGNORED));
    assert!(
        !repo.path().join(".gitignore").exists(),
        "no extra untracked file"
    );
}

#[test]
fn work_conflict_leaves_three_index_stages() {
    let temp = TempTree::new("conflict");
    let repo = seeded(&temp, "one");
    repo.work_conflict("README");

    assert!(repo.has_conflicts());
    assert!(repo.index_entry_at("README", 0).is_none());
    let mut ids = Vec::new();
    for stage in 1..=3 {
        let entry = repo
            .index_entry_at("README", stage)
            .unwrap_or_else(|| panic!("stage {stage}"));
        assert_eq!(entry.mode, modes::FILE);
        ids.push(entry.id);
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3, "each stage holds a different blob");
    assert!(repo.status_of("README").contains(Status::CONFLICTED));
    assert!(repo.read_file("README").starts_with(b"<<<<<<<"));
}

#[cfg(unix)]
#[test]
fn mode_and_link_changes_are_visible_without_touching_the_index() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempTree::new("modes");
    let repo = seeded(&temp, "one");
    repo.work_mode_change("README");
    assert_eq!(repo.index_entry_at("README", 0).unwrap().mode, modes::FILE);
    let mode = std::fs::metadata(repo.path().join("README"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    assert!(repo.status_of("README").contains(Status::WT_MODIFIED));

    let linked = seeded(&temp, "two");
    linked.work_link_change("README", Path::new("/etc/hosts"));
    let entry = linked.index_entry_at("README", 0).unwrap();
    assert_eq!(entry.mode, modes::FILE, "the index still records a file");
    assert!(
        linked
            .path()
            .join("README")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(linked.status_of("README").contains(Status::WT_TYPECHANGE));
    assert_ne!(modes::LINK, modes::FILE);
    assert_ne!(modes::EXECUTABLE, modes::FILE);
}

#[test]
fn deletions_and_renames_are_constructible() {
    let temp = TempTree::new("delete-rename");
    let repo = seeded(&temp, "one");
    repo.work_deleted("README");
    assert!(!repo.path().join("README").exists());
    assert!(repo.is_tracked("README"), "the index still has it");
    assert!(repo.status_of("README").contains(Status::WT_DELETED));

    let renamed = seeded(&temp, "two");
    renamed.work_renamed("README", "docs/README");
    assert!(!renamed.path().join("README").exists());
    assert_eq!(renamed.read_file("docs/README"), b"hello\n");
    assert!(!renamed.is_tracked("README"));
    assert!(renamed.is_tracked("docs/README"));
}

#[test]
fn suppression_flags_are_written_and_read_back_from_the_index() {
    let temp = TempTree::new("suppressed");

    let assume = seeded(&temp, "assume");
    assume.work_suppressed("README", SuppressionFlag::AssumeUnchanged);
    let entry = assume.index_entry_at("README", 0).unwrap();
    assert_ne!(entry.flags & IndexEntryFlag::VALID.bits(), 0);

    let skip = seeded(&temp, "skip");
    skip.work_suppressed("README", SuppressionFlag::SkipWorktree);
    let entry = skip.index_entry_at("README", 0).unwrap();
    assert_ne!(
        entry.flags_extended & IndexEntryExtendedFlag::SKIP_WORKTREE.bits(),
        0
    );

    let other = seeded(&temp, "other");
    other.work_suppressed("README", SuppressionFlag::Other);
    let entry = other.index_entry_at("README", 0).unwrap();
    assert_ne!(
        entry.flags_extended & IndexEntryExtendedFlag::INTENT_TO_ADD.bits(),
        0
    );

    // The physical state of a suppressed path is the caller's to choose, and
    // §5.1 requires all three to be reachable.
    assume.write_file("README", b"differs from the index\n");
    assert_eq!(assume.read_file("README"), b"differs from the index\n");
    skip.remove_file("README");
    assert!(!skip.path().join("README").exists());
    assert_eq!(other.read_file("README"), b"hello\n");
}

#[test]
fn sparse_absence_is_skip_worktree_plus_a_missing_file() {
    let temp = TempTree::new("sparse");
    let repo = seeded(&temp, "one");
    repo.commit_files("second", &[("docs/guide.md", b"guide\n")]);
    repo.work_sparse_absent("docs/guide.md");

    assert!(repo.is_tracked("docs/guide.md"), "still a tracked path");
    assert!(
        !repo.path().join("docs/guide.md").exists(),
        "validly absent"
    );
    let entry = repo.index_entry_at("docs/guide.md", 0).unwrap();
    assert_ne!(
        entry.flags_extended & IndexEntryExtendedFlag::SKIP_WORKTREE.bits(),
        0
    );
    assert_eq!(
        repo.config_value("core.sparseCheckout").as_deref(),
        Some("true")
    );
    let patterns = crate::read_file(&repo.layout_git_dir().join("info/sparse-checkout"));
    assert!(
        String::from_utf8(patterns)
            .unwrap()
            .contains("!/docs/guide.md")
    );
}

#[test]
fn every_native_operation_state_is_constructible() {
    let temp = TempTree::new("native");
    let cases = [
        (NativeOperation::Merge, RepositoryState::Merge),
        (NativeOperation::Rebase, RepositoryState::RebaseMerge),
        (NativeOperation::CherryPick, RepositoryState::CherryPick),
        (NativeOperation::Revert, RepositoryState::Revert),
        (NativeOperation::Bisect, RepositoryState::Bisect),
        (NativeOperation::ApplyMailbox, RepositoryState::ApplyMailbox),
        (
            NativeOperation::Other,
            RepositoryState::ApplyMailboxOrRebase,
        ),
    ];
    for (index, (operation, expected)) in cases.into_iter().enumerate() {
        let repo = seeded(&temp, &format!("case{index}"));
        assert_eq!(repo.repository_state(), RepositoryState::Clean);
        repo.work_native_operation(operation);
        assert_eq!(repo.repository_state(), expected, "{operation:?}");
    }
}

#[test]
fn stashes_accumulate_as_native_stash_entries() {
    let temp = TempTree::new("stash");
    let repo = seeded(&temp, "one");
    assert_eq!(repo.stash_count(), 0);

    repo.work_unstaged("README", b"first change\n");
    repo.work_untracked("scratch.txt", b"scratch\n");
    let first = repo.work_stash("first stash");
    assert_eq!(repo.stash_count(), 1);
    assert!(repo.contains_object(&first));
    assert_eq!(
        repo.read_file("README"),
        b"hello\n",
        "the worktree is clean"
    );
    assert!(
        !repo.path().join("scratch.txt").exists(),
        "untracked went too"
    );

    repo.work_unstaged("README", b"second change\n");
    let second = repo.work_stash("second stash");
    assert_eq!(repo.stash_count(), 2, "older entries are protected too");
    assert_ne!(first, second);
    assert!(repo.contains_object(&first));
}

// ---- workspace layout ---------------------------------------------------

#[test]
fn a_workspace_has_a_root_and_members_keyed_by_repo_key() {
    let temp = TempTree::new("workspace");
    let workspace = temp.workspace("ws", &["libs/a", "apps/b"]);
    assert_eq!(workspace.member_names(), vec!["apps/b", "libs/a"]);
    assert_eq!(workspace.root().path(), workspace.path());
    assert_eq!(
        workspace.member("libs/a").path(),
        workspace.path().join("libs/a")
    );
    assert!(workspace.get(&RepoKey::Root).is_some());
    assert!(
        workspace
            .get(&RepoKey::Member {
                id: "nope".to_owned()
            })
            .is_none()
    );

    let commits = workspace.commit_all("first");
    assert_eq!(commits.len(), 3);
    let distinct: std::collections::BTreeSet<_> = commits.values().collect();
    assert_eq!(distinct.len(), 3, "each repository has its own history");
    for (key, repo) in workspace.repos() {
        assert_eq!(repo.head_id(), commits[key]);
    }

    let target = workspace.dir("target/debug");
    assert!(target.is_dir());
    assert!(
        workspace
            .file("target/debug/artifact", b"build output\n")
            .is_file()
    );
}

#[test]
fn a_workspace_can_hold_an_unmanaged_nested_repository() {
    let temp = TempTree::new("nested");
    let mut workspace = temp.workspace("ws", &["libs/a"]);
    workspace.commit_all("first");
    let nested_path = {
        let nested = workspace.nested_repo("vendor/thing", &RepoSpec::new());
        nested.commit_files("vendor", &[("VENDOR", b"vendored\n")]);
        nested.path().to_path_buf()
    };
    assert_eq!(nested_path, workspace.path().join("vendor/thing"));
    assert_eq!(workspace.member_names(), vec!["libs/a"], "not a member");
    let listed: Vec<&str> = workspace.nested_repos().map(|(name, _)| name).collect();
    assert_eq!(listed, vec!["vendor/thing"]);
    assert!(nested_path.join(".git").is_dir());
}

#[test]
fn a_workspace_can_be_built_bare_for_the_hub_shape() {
    let temp = TempTree::new("bare-workspace");
    let workspace =
        crate::TestWorkspace::init(&temp.join("hub"), &["libs/a"], &RepoSpec::new().bare());
    workspace.commit_all("first");
    assert!(workspace.root().is_bare());
    assert!(workspace.member("libs/a").is_bare());
    assert_eq!(workspace.root().workdir(), None);
}
