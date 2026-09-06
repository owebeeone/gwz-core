//! `local_clone::tests::family_merge`: `gwz merge --remote <name> [<ref>]`
//! end to end on real workspaces built with `gwz-local-testrepo` -- the
//! import through the retained ref and the one delegation to the engine
//! (LCM1.2; design §6, §6.1, §6.2, §12; plan §3 "MVP exit").

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use gwz_family_store_contract::{FamilyLocation, FamilyStore, StoreError};
use gwz_local_import::IMPORT_REF_NAMESPACE;

use super::fixture::meta;
use crate::git::{Git2Backend, GitBackend};
use crate::local_clone::family_merge::family_store;
use crate::model::ErrorCode;
use crate::operation::{EventSink, NullSink, WorkspaceMutatorLock};
use crate::workspace_ops::{
    handle_add_existing_repo, handle_clone_local_workspace, handle_create_workspace,
    handle_merge_with_events, handle_merge_with_local_family,
};

/// A real family: a workspace root with the given members, each with one
/// commit, registered through the public handlers, and its verbatim clone
/// `A` at the root's sibling `root-A`.
struct Family {
    _tree: gwz_local_testrepo::TempTree,
    root: PathBuf,
    clone: PathBuf,
    backend: Git2Backend,
}

impl Family {
    fn member(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn clone_member(&self, name: &str) -> PathBuf {
        self.clone.join(name)
    }
}

fn workspace_with_members(
    tree: &gwz_local_testrepo::TempTree,
    backend: &Git2Backend,
    members: &[&str],
) -> PathBuf {
    let workspace = tree.workspace("root", members);
    workspace.commit_all("init");
    let root = workspace.path().to_path_buf();
    handle_create_workspace(
        crate::CreateWorkspaceRequest {
            meta: meta("req-create"),
            workspace_root: root.to_string_lossy().into_owned(),
            workspace_id: None,
        },
        "op_create",
    )
    .expect("create the workspace over the fixture root repository");
    for member in members {
        handle_add_existing_repo(
            backend,
            &root,
            crate::AddExistingRepoRequest {
                meta: meta("req-add"),
                repository_path: root.join(member).to_string_lossy().into_owned(),
                member_path: Some((*member).to_owned()),
                member_id: Some(format!("mem_{member}")),
                source_id: Some(format!("src_{member}")),
            },
            "op_add",
        )
        .expect("register the member repository");
    }
    root
}

fn family(label: &str, members: &[&str]) -> Family {
    let tree = gwz_local_testrepo::TempTree::new(label);
    let backend = Git2Backend::without_credential_helpers();
    let root = workspace_with_members(&tree, &backend, members);
    handle_clone_local_workspace(
        &backend,
        &root,
        crate::CloneLocalWorkspaceRequest {
            meta: meta("req-clone-local"),
            name: "A".to_owned(),
            dest: None,
            mode: crate::LocalCloneMode::Verbatim,
            branch: None,
            copy_source: None,
        },
        "op_clone",
        &NullSink,
    )
    .expect("clone A");
    let clone = fs::canonicalize(tree.path().join("root-A")).unwrap();
    Family {
        _tree: tree,
        root,
        clone,
        backend,
    }
}

/// Commit `file` on top of `HEAD` in the repository at `path`, updating
/// `update_ref` (`HEAD` or a branch), with the fixture's fixed identity so
/// the id is deterministic. The worktree follows only when `HEAD` moved.
fn commit(path: &Path, file: &str, content: &str, message: &str, update_ref: &str) -> String {
    let repo = git2::Repository::open(path).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let parent = match update_ref {
        "HEAD" => head,
        branch => repo
            .find_reference(branch)
            .ok()
            .map_or(head, |reference| reference.peel_to_commit().unwrap()),
    };
    let blob = repo.blob(content.as_bytes()).unwrap();
    let tree_id = {
        let base = parent.tree().unwrap();
        let mut builder = repo.treebuilder(Some(&base)).unwrap();
        builder.insert(file, blob, 0o100644).unwrap();
        builder.write().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = gwz_local_testrepo::fixture_signature();
    let id = repo
        .commit(
            Some(update_ref),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent],
        )
        .unwrap();
    if update_ref == "HEAD" {
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
    }
    id.to_string()
}

fn head(backend: &Git2Backend, path: &Path) -> String {
    backend.head(path).unwrap().commit.unwrap()
}

fn read_ref(backend: &Git2Backend, path: &Path, name: &str) -> Option<String> {
    backend.read_ref(path, name).unwrap()
}

/// Every retained import ref in the repository at `path`, sorted.
fn import_refs(path: &Path) -> Vec<String> {
    let repo = git2::Repository::open(path).unwrap();
    let mut names: Vec<String> = repo
        .references_glob(&format!("{IMPORT_REF_NAMESPACE}*"))
        .unwrap()
        .map(|reference| reference.unwrap().name().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

fn family_merge_request(token: &str, source_ref: Option<&str>) -> crate::MergeRequest {
    crate::MergeRequest {
        meta: meta("req-family-merge"),
        op: crate::MergeOp::Start,
        source_ref: source_ref.map(str::to_owned),
        local_source_name: Some(token.to_owned()),
        ..Default::default()
    }
}

fn lifecycle_request(op: crate::MergeOp, merge_id: Option<String>) -> crate::MergeRequest {
    crate::MergeRequest {
        meta: meta("req-merge-lifecycle"),
        op,
        merge_id,
        ..Default::default()
    }
}

fn repo<'a>(response: &'a crate::MergeResponse, target_id: &str) -> &'a crate::MergeRepoSummary {
    response
        .repos
        .iter()
        .find(|repo| repo.target_id == target_id)
        .unwrap_or_else(|| panic!("no participant {target_id}: {:?}", response.repos))
}

/// Collects every event; on the engine's first member event it probes the
/// family lock, so the test can assert what the lock covers.
#[derive(Default)]
struct ProbingSink {
    events: Mutex<Vec<crate::OperationEvent>>,
    family_root: Option<PathBuf>,
    family_lock_busy_during_engine: Mutex<Option<bool>>,
}

impl EventSink for ProbingSink {
    fn deliver(&self, event: crate::OperationEvent) {
        if event.kind == crate::EventKind::MemberStarted
            && let Some(root) = &self.family_root
        {
            let busy = matches!(
                family_store().try_lock(&FamilyLocation::new(root)),
                Err(StoreError::Busy { .. })
            );
            self.family_lock_busy_during_engine
                .lock()
                .unwrap()
                .get_or_insert(busy);
        }
        self.events.lock().unwrap().push(event);
    }
}

fn family_lock_is_free(root: &Path) -> bool {
    family_store()
        .try_lock(&FamilyLocation::new(root))
        .map(drop)
        .is_ok()
}

/// Plan §3 MVP exit, "commit and merge by family name"; design §12 "family
/// merge delegates to existing engine". Work committed in the clone's
/// member reaches the root's member through `gwz merge --remote A`: the
/// import ref `refs/gwz/local-imports/<transfer-id>` is fetched into the
/// receiver under the anonymous transport (no remote persisted), the engine
/// is entered once with that ref as `source_ref` and fast-forwards, and the
/// ref is retained afterwards. The family lock is held through the
/// delegation and released after it; no receiver workspace lock is preheld
/// (the engine took its own, or it could not have merged), and the driver
/// sees one operation lifecycle.
#[test]
fn a_family_merge_by_name_integrates_the_clones_commits_through_a_retained_import() {
    let family = family("family-merge-name", &["app"]);
    let backend = &family.backend;
    let before = head(backend, &family.member("app"));
    let in_clone = commit(
        &family.clone_member("app"),
        "feature.txt",
        "from A\n",
        "work in A",
        "HEAD",
    );
    assert_ne!(in_clone, before);

    let sink = ProbingSink {
        family_root: Some(family.root.clone()),
        ..Default::default()
    };
    let response = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", None),
        "op_family_merge",
        &sink,
    )
    .expect("the family merge integrates");

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert!(!response.open);
    let app = repo(&response, "mem_app");
    assert_eq!(app.state, crate::MergeParticipantState::FastForwarded);
    assert!(
        app.source_ref.starts_with("refs/gwz/local-imports/xfer_"),
        "{}",
        app.source_ref
    );
    assert_eq!(app.source_ref.len(), IMPORT_REF_NAMESPACE.len() + 5 + 32);
    assert_eq!(app.source_commit, in_clone);
    assert_eq!(app.resulting_commit.as_deref(), Some(in_clone.as_str()));
    let message = response.response.meta.message.as_deref().unwrap_or("");
    assert!(
        message.contains(&format!(
            "imported HEAD of family member `A` as {} (mem_app={in_clone})",
            app.source_ref
        )),
        "{message}"
    );
    assert!(message.contains("never pruned"), "{message}");

    // The receiver holds the work, the retained ref and no remote.
    assert_eq!(head(backend, &family.member("app")), in_clone);
    assert_eq!(
        read_ref(backend, &family.member("app"), &app.source_ref).as_deref(),
        Some(in_clone.as_str()),
        "the import ref is retained after the merge"
    );
    assert_eq!(
        import_refs(&family.member("app")),
        vec![app.source_ref.clone()]
    );
    assert!(backend.remotes(&family.member("app")).unwrap().is_empty());
    // The root repository was not a receiver (not selected), and the source
    // is untouched: A's member still holds its own commit and no import ref.
    assert!(import_refs(&family.root).is_empty());
    assert_eq!(head(backend, &family.clone_member("app")), in_clone);
    assert!(import_refs(&family.clone_member("app")).is_empty());
    let lock = crate::artifact::read_lock(&family.root).unwrap();
    assert_eq!(
        lock.members["mem_app"].commit.as_deref(),
        Some(in_clone.as_str()),
        "the engine published the lock"
    );

    // What the lock covers (design §3.2): the family lock was held while
    // the engine ran and is free now; the receiver's mutator lock is free
    // too -- nothing leaked, and nothing was preheld or the engine's own
    // acquisition would have refused.
    assert_eq!(
        *sink.family_lock_busy_during_engine.lock().unwrap(),
        Some(true),
        "the family lock is held across the delegation"
    );
    assert!(family_lock_is_free(&family.root));
    assert!(
        WorkspaceMutatorLock::try_acquire(&family.root)
            .unwrap()
            .is_some()
    );

    // One operation lifecycle, numbered from one origin.
    let events = sink.events.lock().unwrap();
    let kinds: Vec<crate::EventKind> = events.iter().map(|event| event.kind).collect();
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == crate::EventKind::OperationStarted)
            .count(),
        1,
        "{kinds:?}"
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == crate::EventKind::OperationFinished)
            .count(),
        1,
        "{kinds:?}"
    );
    assert_eq!(kinds.first(), Some(&crate::EventKind::OperationStarted));
    assert_eq!(kinds.last(), Some(&crate::EventKind::OperationFinished));
    let sequences: Vec<i64> = events.iter().map(|event| event.sequence).collect();
    assert_eq!(sequences[0], 0);
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "{sequences:?}"
    );
    assert!(
        events
            .iter()
            .all(|event| event.operation_id == "op_family_merge")
    );
}

/// Design §6.1: `merge --remote A <ref>` integrates `<ref>` resolved in A --
/// a branch A's HEAD is not on -- and the import ref holds that commit, not
/// A's HEAD. From inside the clone, `merge --remote root` integrates the
/// root's HEAD the same way, through the pointer to the family index.
#[test]
fn an_explicit_source_ref_and_the_root_as_source_integrate_through_the_same_import() {
    let family = family("family-merge-ref", &["app"]);
    let backend = &family.backend;
    let clone_head = head(backend, &family.clone_member("app"));
    let lane = commit(
        &family.clone_member("app"),
        "lane.txt",
        "lane\n",
        "lane work",
        "refs/heads/lane/agent-17",
    );
    assert_eq!(head(backend, &family.clone_member("app")), clone_head);

    let response = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", Some("lane/agent-17")),
        "op_family_merge_ref",
        &NullSink,
    )
    .expect("the explicit ref integrates");
    let app = repo(&response, "mem_app");
    assert_eq!(app.state, crate::MergeParticipantState::FastForwarded);
    assert_eq!(app.source_commit, lane);
    assert_eq!(head(backend, &family.member("app")), lane);
    assert_eq!(
        read_ref(backend, &family.member("app"), &app.source_ref).as_deref(),
        Some(lane.as_str())
    );
    assert!(
        response
            .response
            .meta
            .message
            .as_deref()
            .unwrap_or("")
            .contains("imported refs/heads/lane/agent-17 of family member `A`")
    );

    // A ref that resolves nowhere in A refuses before any fetch, with the
    // engine's own start-validation code, and leaves nothing behind.
    let missing = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", Some("lane/absent")),
        "op_family_merge_missing",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(missing.code, ErrorCode::MergeValidationFailed);
    assert!(
        missing.message.contains("mem_app: ") && missing.message.contains("refs/heads/lane/absent"),
        "{}",
        missing.message
    );
    assert!(
        missing.message.contains("nothing was written"),
        "{}",
        missing.message
    );
    assert_eq!(import_refs(&family.member("app")).len(), 1);

    // The other direction: the root commits, the clone integrates it.
    let in_root = commit(
        &family.member("app"),
        "root.txt",
        "from root\n",
        "work at root",
        "HEAD",
    );
    let response = handle_merge_with_local_family(
        backend,
        &family.clone,
        family_merge_request("root", None),
        "op_family_merge_root",
        &NullSink,
    )
    .expect("the clone integrates the root");
    let app = repo(&response, "mem_app");
    assert_eq!(app.state, crate::MergeParticipantState::FastForwarded);
    assert_eq!(app.source_commit, in_root);
    assert_eq!(head(backend, &family.clone_member("app")), in_root);
    assert_eq!(
        read_ref(backend, &family.clone_member("app"), &app.source_ref).as_deref(),
        Some(in_root.as_str())
    );
    assert!(family_lock_is_free(&family.root));
}

/// Design §6.2: one common import name in every paired receiver, each
/// holding its own captured id; design §12 "family merge delegates to
/// existing engine": both members fast-forward in one engine run.
#[test]
fn every_paired_receiver_holds_the_common_import_name_with_its_own_captured_id() {
    let family = family("family-merge-two", &["app", "lib"]);
    let backend = &family.backend;
    let app_commit = commit(
        &family.clone_member("app"),
        "app.txt",
        "app work\n",
        "app work",
        "HEAD",
    );
    let lib_commit = commit(
        &family.clone_member("lib"),
        "lib.txt",
        "lib work\n",
        "lib work",
        "HEAD",
    );
    assert_ne!(app_commit, lib_commit);

    let response = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", None),
        "op_family_merge_two",
        &NullSink,
    )
    .expect("both members integrate");
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    let app = repo(&response, "mem_app");
    let lib = repo(&response, "mem_lib");
    assert_eq!(app.source_ref, lib.source_ref, "one common import name");
    assert_eq!(app.source_commit, app_commit);
    assert_eq!(lib.source_commit, lib_commit);
    assert_eq!(app.state, crate::MergeParticipantState::FastForwarded);
    assert_eq!(lib.state, crate::MergeParticipantState::FastForwarded);
    assert_eq!(
        read_ref(backend, &family.member("app"), &app.source_ref).as_deref(),
        Some(app_commit.as_str())
    );
    assert_eq!(
        read_ref(backend, &family.member("lib"), &lib.source_ref).as_deref(),
        Some(lib_commit.as_str())
    );
    let message = response.response.meta.message.unwrap_or_default();
    assert!(
        message.contains(&format!("mem_app={app_commit}, mem_lib={lib_commit}")),
        "{message}"
    );
}

/// Design §12 "Family merge delegates to existing engine: existing
/// lifecycle unchanged": a conflicting family merge stays open under the
/// engine's own record, and `--continue` completes it with the imported
/// commit as the merge's second parent -- after the source advanced and
/// detached its HEAD, which changed nothing about the imported commit
/// (design §6.2 "after merge start, source advancement ... does not change
/// its imported commits"). A second family start while the merge is open
/// refuses before any fetch.
#[test]
fn a_conflicting_family_merge_stays_open_and_continues_from_the_imported_commit() {
    let family = family("family-merge-conflict", &["app"]);
    let backend = &family.backend;
    let local = commit(
        &family.member("app"),
        "README",
        "local\n",
        "local edit",
        "HEAD",
    );
    let imported = commit(
        &family.clone_member("app"),
        "README",
        "clone\n",
        "clone edit",
        "HEAD",
    );

    let started = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", None),
        "op_family_conflict",
        &NullSink,
    )
    .expect("a conflicted start is a response, not an error");
    assert_eq!(
        started.response.meta.aggregate_status,
        crate::AggregateStatus::Conflicted
    );
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );
    assert!(started.open);
    let merge_id = started.merge_id.clone().expect("an open merge id");
    let import_ref = repo(&started, "mem_app").source_ref.clone();
    assert_eq!(repo(&started, "mem_app").source_commit, imported);
    assert_eq!(repo(&started, "mem_app").conflict_paths, ["README"]);
    assert!(
        family
            .root
            .join(format!(".gwz/merge/{merge_id}.yaml"))
            .is_file(),
        "the engine's own record is open"
    );
    assert!(
        family_lock_is_free(&family.root),
        "the family lock is released with the response"
    );

    // A second family start while the merge is open refuses before any
    // fetch: still exactly one import ref in the receiver.
    let refused = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", None),
        "op_family_conflict_again",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::OpenOperation);
    assert!(refused.message.contains(&merge_id), "{}", refused.message);
    assert!(
        refused.message.contains("nothing was imported"),
        "{}",
        refused.message
    );
    assert_eq!(import_refs(&family.member("app")), vec![import_ref.clone()]);

    // The source advances and detaches after the start.
    let advanced = commit(
        &family.clone_member("app"),
        "later.txt",
        "later\n",
        "later work in A",
        "HEAD",
    );
    assert_ne!(advanced, imported);
    let clone_repo = git2::Repository::open(family.clone_member("app")).unwrap();
    clone_repo
        .set_head_detached(git2::Oid::from_str(&advanced).unwrap())
        .unwrap();
    assert!(clone_repo.head_detached().unwrap());
    assert_eq!(
        read_ref(backend, &family.member("app"), &import_ref).as_deref(),
        Some(imported.as_str()),
        "the imported commit did not move with the source"
    );
    let status = handle_merge_with_local_family(
        backend,
        &family.root,
        lifecycle_request(crate::MergeOp::Status, None),
        "op_family_status",
        &NullSink,
    )
    .unwrap();
    assert_eq!(status.merge_id.as_deref(), Some(merge_id.as_str()));
    assert_eq!(repo(&status, "mem_app").source_commit, imported);

    // Resolve and continue, through the same entry every driver uses.
    fs::write(family.member("app").join("README"), "resolved\n").unwrap();
    backend
        .stage_paths_allowing_other_conflicts(&family.member("app"), &["README"])
        .unwrap();
    let continued = handle_merge_with_local_family(
        backend,
        &family.root,
        lifecycle_request(crate::MergeOp::Resume, Some(merge_id.clone())),
        "op_family_continue",
        &NullSink,
    )
    .expect("continue completes");
    assert_eq!(continued.state, crate::MergeOperationState::Completed);
    assert!(!continued.open);
    assert_eq!(
        repo(&continued, "mem_app").state,
        crate::MergeParticipantState::Continued
    );
    let result = git2::Oid::from_str(
        repo(&continued, "mem_app")
            .resulting_commit
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    let receiver = git2::Repository::open(family.member("app")).unwrap();
    let merge_commit = receiver.find_commit(result).unwrap();
    assert_eq!(merge_commit.parent_id(0).unwrap().to_string(), local);
    assert_eq!(
        merge_commit.parent_id(1).unwrap().to_string(),
        imported,
        "the merge's second parent is the imported commit, not the advanced source"
    );
    assert!(
        family
            .root
            .join(format!(".gwz/merge/done/{merge_id}.yaml"))
            .is_file()
    );
    assert_eq!(
        import_refs(&family.member("app")),
        vec![import_ref],
        "the import ref is retained after the continue"
    );
}

/// The other half of the lifecycle row: `--abort` restores the receiver
/// exactly and archives the record, and the import ref outlives it (design
/// §6.2: no pruning on success, error, cancellation or abort). Then design
/// §12 "open merge after source ... disposal or ordinary Git GC: retained
/// import ref keeps source objects local": with the source workspace gone
/// and `git gc --prune=now` run in the receiver, the imported commit is
/// still there, held by the retained ref alone.
#[test]
fn an_aborted_family_merge_keeps_its_import_ref_which_holds_the_objects_through_gc() {
    let family = family("family-merge-abort", &["app"]);
    let backend = &family.backend;
    let local = commit(
        &family.member("app"),
        "README",
        "local\n",
        "local edit",
        "HEAD",
    );
    let imported = commit(
        &family.clone_member("app"),
        "README",
        "clone\n",
        "clone edit",
        "HEAD",
    );
    let lock_before = fs::read(family.root.join(crate::artifact::LOCK_PATH)).unwrap();

    let started = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", None),
        "op_family_abort",
        &NullSink,
    )
    .unwrap();
    assert!(started.open);
    let merge_id = started.merge_id.clone().unwrap();
    let import_ref = repo(&started, "mem_app").source_ref.clone();

    let aborted = handle_merge_with_local_family(
        backend,
        &family.root,
        lifecycle_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_family_abort_abort",
        &NullSink,
    )
    .expect("abort restores");
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert!(!aborted.open);
    assert_eq!(head(backend, &family.member("app")), local);
    assert!(
        backend
            .merge_state(&family.member("app"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fs::read(family.root.join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert!(
        family
            .root
            .join(format!(".gwz/merge/done/{merge_id}.yaml"))
            .is_file()
    );
    assert_eq!(
        read_ref(backend, &family.member("app"), &import_ref).as_deref(),
        Some(imported.as_str()),
        "abort prunes nothing"
    );

    // The source is gone; ordinary GC in the receiver; the object survives.
    fs::remove_dir_all(&family.clone).unwrap();
    assert!(!family.clone.exists());
    let gc = Command::new("git")
        .arg("-C")
        .arg(family.member("app"))
        .args(["gc", "--prune=now", "--quiet"])
        .status()
        .expect("git is available");
    assert!(gc.success(), "git gc: {gc}");
    let receiver = git2::Repository::open(family.member("app")).unwrap();
    let odb = receiver.odb().unwrap();
    assert!(
        odb.exists(git2::Oid::from_str(&imported).unwrap()),
        "the retained import ref holds the imported commit through gc"
    );
    assert_eq!(
        read_ref(backend, &family.member("app"), &import_ref).as_deref(),
        Some(imported.as_str())
    );
    // And it is usable as an ordinary Git ref: a plain merge of it (no
    // family selector) plans against local objects only.
    let planned = handle_merge_with_local_family(
        backend,
        &family.root,
        crate::MergeRequest {
            meta: crate::RequestMeta {
                dry_run: Some(true),
                ..meta("req-plain-merge")
            },
            op: crate::MergeOp::Start,
            source_ref: Some(import_ref.clone()),
            ..Default::default()
        },
        "op_plain_dry_run",
        &NullSink,
    )
    .expect("the retained ref plans as an ordinary source");
    assert_eq!(repo(&planned, "mem_app").source_commit, imported);
}

/// Design §6 "set mismatch ... refuses aggregating; no fetch"; §12: a
/// pairing set mismatch is `pairing_mismatch`, refused before any fetch,
/// with nothing written -- no import ref anywhere, no merge record, the
/// family lock released.
#[test]
fn a_pairing_set_mismatch_refuses_before_any_fetch() {
    let family = family("family-merge-pairing", &["app"]);
    let backend = &family.backend;
    let before = head(backend, &family.member("app"));
    commit(
        &family.clone_member("app"),
        "feature.txt",
        "from A\n",
        "work in A",
        "HEAD",
    );
    // A grows a member the root does not have.
    let extra = gwz_local_testrepo::TestRepo::init(
        &family.clone.join("extra"),
        &gwz_local_testrepo::RepoSpec::new(),
    );
    extra.commit_files("extra", &[("README", b"extra\n")]);
    handle_add_existing_repo(
        backend,
        &family.clone,
        crate::AddExistingRepoRequest {
            meta: meta("req-add-extra"),
            repository_path: family.clone.join("extra").to_string_lossy().into_owned(),
            member_path: Some("extra".to_owned()),
            member_id: Some("mem_extra".to_owned()),
            source_id: Some("src_extra".to_owned()),
        },
        "op_add_extra",
    )
    .expect("A registers an extra member");

    let error = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", None),
        "op_family_pairing",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::PairingMismatch);
    assert!(
        error.message.contains("unpaired: mem_extra"),
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .contains("no import ref was created; nothing was written"),
        "{}",
        error.message
    );
    for path in [
        family.member("app"),
        family.root.clone(),
        family.clone_member("app"),
        family.clone.clone(),
    ] {
        assert!(import_refs(&path).is_empty(), "{}", path.display());
    }
    assert!(!family.root.join(".gwz/merge").exists());
    assert!(family_lock_is_free(&family.root));
    assert_eq!(head(backend, &family.member("app")), before);
}

/// Design §6.2 "a partial or mismatched fetch does not start a partly
/// sourced merge ... leaves the refs it created ... a retry uses a fresh
/// transfer id"; §12 "import crashes before engine entry: retained Git
/// refs may remain; no record repair or automatic pruning". The second
/// receiver cannot take a ref: the first receiver's import ref is created
/// and retained, the refusal is `import_incomplete` naming it, no record is
/// opened, and the retry -- under a fresh id -- succeeds beside the retained
/// one.
#[cfg(unix)]
#[test]
fn a_partial_import_leaves_its_refs_refuses_and_a_retry_succeeds_under_a_fresh_id() {
    use std::os::unix::fs::PermissionsExt;
    let family = family("family-merge-partial", &["app", "lib"]);
    let backend = &family.backend;
    let app_commit = commit(
        &family.clone_member("app"),
        "app.txt",
        "app work\n",
        "app work",
        "HEAD",
    );
    let lib_commit = commit(
        &family.clone_member("lib"),
        "lib.txt",
        "lib work\n",
        "lib work",
        "HEAD",
    );
    let lib_refs = family.member("lib").join(".git/refs");
    let writable = fs::metadata(&lib_refs).unwrap().permissions();
    fs::set_permissions(&lib_refs, fs::Permissions::from_mode(0o555)).unwrap();

    let error = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", None),
        "op_family_partial",
        &NullSink,
    )
    .unwrap_err();
    fs::set_permissions(&lib_refs, writable).unwrap();
    assert_eq!(error.code, ErrorCode::ImportIncomplete, "{}", error.message);
    let retained = import_refs(&family.member("app"));
    assert_eq!(retained.len(), 1, "{retained:?}");
    assert!(
        error.message.contains(&format!(
            "retained import refs (ordinary Git refs, never pruned by gwz): mem_app {} = \
             {app_commit}",
            retained[0]
        )),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("mem_lib: transfer failed"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("the merge engine was not entered"),
        "{}",
        error.message
    );
    assert!(import_refs(&family.member("lib")).is_empty());
    assert!(!family.root.join(".gwz/merge").exists());
    assert_ne!(head(backend, &family.member("app")), app_commit);
    assert!(family_lock_is_free(&family.root));

    // The retry: a fresh id, both receivers, the retained ref untouched.
    let response = handle_merge_with_local_family(
        backend,
        &family.root,
        family_merge_request("A", None),
        "op_family_partial_retry",
        &NullSink,
    )
    .expect("the retry integrates");
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    let fresh = repo(&response, "mem_app").source_ref.clone();
    assert_ne!(fresh, retained[0]);
    assert_eq!(repo(&response, "mem_lib").source_ref, fresh);
    let mut expected = vec![retained[0].clone(), fresh.clone()];
    expected.sort();
    assert_eq!(import_refs(&family.member("app")), expected);
    assert_eq!(import_refs(&family.member("lib")), vec![fresh]);
    assert_eq!(head(backend, &family.member("app")), app_commit);
    assert_eq!(head(backend, &family.member("lib")), lib_commit);
}

/// Plan §3 MVP exit: "ordinary non-family merges retain their behavior".
/// A merge without the selector never reaches the wrapper: it creates no
/// family file, no import ref and no family lock, and answers exactly what
/// the engine's public entry answers for the same request on a twin
/// workspace.
#[test]
fn an_ordinary_merge_is_the_engines_own_answer_and_touches_no_family_state() {
    let backend = Git2Backend::without_credential_helpers();
    let twins: Vec<(gwz_local_testrepo::TempTree, PathBuf)> = ["ordinary-a", "ordinary-b"]
        .into_iter()
        .map(|label| {
            let tree = gwz_local_testrepo::TempTree::new(label);
            let root = workspace_with_members(&tree, &backend, &["app"]);
            commit(
                &root.join("app"),
                "feature.txt",
                "feature\n",
                "feature work",
                "refs/heads/feature/x",
            );
            (tree, root)
        })
        .collect();
    let request = crate::MergeRequest {
        meta: meta("req-ordinary-merge"),
        op: crate::MergeOp::Start,
        source_ref: Some("feature/x".to_owned()),
        ..Default::default()
    };

    let through_wrapper_entry = handle_merge_with_local_family(
        &backend,
        &twins[0].1,
        request.clone(),
        "op_ordinary",
        &NullSink,
    )
    .expect("the ordinary merge completes");
    let through_engine_entry =
        handle_merge_with_events(&backend, &twins[1].1, request, "op_ordinary", &NullSink)
            .expect("the engine completes the twin");

    assert_eq!(
        through_wrapper_entry.state,
        crate::MergeOperationState::Completed
    );
    assert_eq!(
        through_wrapper_entry.merge_id,
        through_engine_entry.merge_id
    );
    assert_eq!(through_wrapper_entry.state, through_engine_entry.state);
    assert_eq!(through_wrapper_entry.open, through_engine_entry.open);
    assert_eq!(
        through_wrapper_entry.participant_counts,
        through_engine_entry.participant_counts
    );
    assert_eq!(through_wrapper_entry.repos, through_engine_entry.repos);
    assert_eq!(
        through_wrapper_entry.response.meta.aggregate_status,
        through_engine_entry.response.meta.aggregate_status
    );
    assert_eq!(
        through_wrapper_entry.response.meta.message,
        through_engine_entry.response.meta.message
    );
    assert_eq!(
        repo(&through_wrapper_entry, "mem_app").source_ref,
        "feature/x"
    );
    for (_, root) in &twins {
        assert!(
            super::fixture::family_files_absent(root),
            "{}",
            root.display()
        );
        assert!(import_refs(&root.join("app")).is_empty());
        assert!(import_refs(root).is_empty());
    }
}
