//! Push plan step 3.6 (gwz-dev `dev-docs/GwzUrlSchemePushPlan.md`): last-known
//! refs and the root proof on the native backend, against local bare remotes.
//! A tracking ref that no longer stands for its remote's repository (a branch
//! deleted there, a push URL since removed, a remote since renamed) still
//! leaves its member uncontacted, the risk §3.7 accepts, and the dependency
//! read then refuses the root (D10). A layout whose tracking ref may not stand
//! for the remote gives no last-known ref, so the member is contacted (§3.5).
use std::path::{Path, PathBuf};

use crate::git::{Git2Backend, GitBackend};

use super::*;

const UP_TO_DATE: &str = "up to date with origin/main as of the last fetch or push";

/// A workspace whose root is published to a local bare remote, and whose member
/// `mem_app` at `repos/app` is a clone of a published upstream: its `origin`
/// fetches with `+refs/heads/*:refs/remotes/origin/*`, and `origin/main` is at
/// the upstream commit.
struct ClonedMember {
    temp: TempDir,
    backend: Git2Backend,
    root_remote: PathBuf,
    published_root: String,
    upstream: RemoteFixture,
    app: PathBuf,
    initial: String,
}

impl ClonedMember {
    fn new(name: &str) -> Self {
        let temp = TempDir::new(name);
        let backend = Git2Backend::without_credential_helpers();
        handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
        let root_remote = temp.path().join("root.git");
        init_bare_main(&root_remote);
        backend
            .add_remote(temp.path(), "origin", root_remote.to_str().unwrap())
            .unwrap();
        set_identity(temp.path());
        backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
        let published_root = backend.commit(temp.path(), "base", false).unwrap().commit;
        backend
            .push(temp.path(), "origin", "refs/heads/main:refs/heads/main")
            .unwrap();
        let upstream = RemoteFixture::new(&format!("{name}-upstream"));
        let initial = upstream.commit_and_push("README.md", "one", "initial", &backend);
        let app = temp.path().join("repos/app");
        backend.clone_repo(upstream.remote_url(), &app).unwrap();
        Self {
            temp,
            backend,
            root_remote,
            published_root,
            upstream,
            app,
            initial,
        }
    }

    fn repo(&self) -> git2::Repository {
        git2::Repository::open(&self.app).unwrap()
    }

    /// A commit on the member's `main` that upstream does not hold.
    fn commit_work(&self) -> String {
        let parent = git2::Oid::from_str(&self.initial).unwrap();
        commit_file(&self.app, "README.md", "work", "work", &[parent]).unwrap()
    }

    /// The Git2 answer that a push classifies the member's `main` with.
    fn last_known_main(&self) -> Option<String> {
        self.backend
            .last_known_ref(&self.app, "origin", "refs/heads/main")
            .unwrap()
    }

    /// Record `commit` for the member, at its upstream URL, in the worktree lock.
    fn lock(&self, commit: &str) {
        write_pull_fixture(
            self.temp.path(),
            vec![("mem_app", "repos/app", self.upstream.remote_url(), commit)],
        );
    }

    /// A default push of the members alone.
    fn push_members(&self) -> crate::PushResponse {
        handle_push(
            &self.backend,
            self.temp.path(),
            push_request(None, None),
            "members",
        )
        .unwrap()
    }

    /// Commit a root lock that names `commit`, and return the root's commit.
    fn commit_root_lock(&self, commit: &str) -> String {
        self.lock(commit);
        self.backend
            .stage_paths(self.temp.path(), &["gwz.conf"])
            .unwrap();
        self.backend
            .commit(self.temp.path(), "lock", false)
            .unwrap()
            .commit
    }

    /// Commit a root lock that names `commit`, and push the whole workspace by
    /// default.
    fn push_root_naming(&self, commit: &str) -> crate::PushResponse {
        self.commit_root_lock(commit);
        let request = crate::PushRequest {
            meta: request_meta_with_workspace(),
            ..Default::default()
        };
        handle_push(&self.backend, self.temp.path(), request, "push").unwrap()
    }

    /// A default push of the root alone, as `gwz --target @root push` makes.
    fn push_root_only(&self) -> crate::PushResponse {
        let request = crate::PushRequest {
            meta: crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: vec!["@root".to_owned()],
                    ..Default::default()
                }),
                ..request_meta_with_workspace()
            },
            ..Default::default()
        };
        handle_push(&self.backend, self.temp.path(), request, "root").unwrap()
    }

    /// The push took the member as up to date and did not contact it. The read
    /// of the committed URL, the only URL the refusal names, did not show
    /// `commit`, so the root was refused with the §3.6 remedies and not pushed.
    fn assert_root_refused(&self, response: &crate::PushResponse, commit: &str) {
        let rows = &response.response.members;
        let row = |id: &str| rows.iter().find(|row| row.member_id == id).unwrap();
        let member = row("mem_app");
        let reason = member
            .planned
            .as_ref()
            .and_then(|planned| planned.message.as_deref());
        assert_eq!(
            (member.status, reason),
            (crate::MemberStatus::Noop, Some(UP_TO_DATE))
        );
        let root = row("@root");
        assert_eq!(root.status, crate::MemberStatus::Rejected);
        let error = root.error.as_ref().unwrap();
        assert_eq!(error.code, crate::GwzErrorCode::RemoteRejected);
        let proof = format!(
            "cannot prove member mem_app commit {commit} is available at its committed fetch remote origin;"
        );
        assert!(error.message.contains(&proof), "{}", error.message);
        assert!(
            error.message.contains("or run gwz push --check-remotes"),
            "{}",
            error.message
        );
        // The root's own read, then the dependency's read before the transfers
        // and its re-read after them.
        let operations: Vec<_> = response
            .response
            .meta
            .transport
            .iter()
            .flatten()
            .map(|row| row.operation)
            .collect();
        assert_eq!(
            operations,
            [crate::TransportOperation::ReadAdvertisement; 3]
        );
        assert_eq!(
            read_repo_ref(&self.root_remote, "refs/heads/main"),
            Some(self.published_root.clone())
        );
    }
}

/// Run the `git` command line in `repo`, as an operator would.
fn git(repo: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Step 3.6, a deleted remote branch (§3.7): a member push through gwz writes
/// `origin/main`, another clone deletes upstream's `main`, and a fetch that
/// does not prune keeps that ref. A default push takes the member as up to
/// date and does not contact it, and the dependency read refuses the root that
/// names the member's commit.
#[test]
fn a_branch_deleted_upstream_leaves_the_member_uncontacted_and_refuses_the_root() {
    let fixture = ClonedMember::new("push-deleted-branch");
    let work = fixture.commit_work();
    fixture.lock(&work);
    let published = fixture.push_members();
    let member = published.response.members.single();
    assert_eq!(member.status, crate::MemberStatus::Ok, "{:?}", member.error);
    assert_eq!(fixture.last_known_main(), Some(work.clone()));
    let other = TempDir::new("push-deleted-branch-other");
    let clone = other.path().join("clone");
    let backend = &fixture.backend;
    backend
        .clone_repo(fixture.upstream.remote_url(), &clone)
        .unwrap();
    backend.push(&clone, "origin", ":refs/heads/main").unwrap();
    assert_eq!(
        read_repo_ref(&fixture.upstream.remote, "refs/heads/main"),
        None
    );
    // A `fetch.prune` setting would make the fetch prune; the remote's own
    // setting overrides it.
    fixture
        .repo()
        .config()
        .unwrap()
        .set_bool("remote.origin.prune", false)
        .unwrap();
    backend.fetch(&fixture.app, "origin").unwrap();
    assert_eq!(fixture.last_known_main(), Some(work.clone()));

    let response = fixture.push_root_naming(&work);

    fixture.assert_root_refused(&response, &work);
    assert_eq!(
        read_repo_ref(&fixture.upstream.remote, "refs/heads/main"),
        None
    );
}

/// Step 3.6, a fork push URL, later removed (§3.7): a gwz push through a push
/// URL naming the fork lands in the fork, not in upstream, and writes
/// `origin/main` at a commit only the fork holds. Once that URL is unset, the
/// ref passes for upstream's, so a default push does not contact the member,
/// and the dependency read of upstream refuses the root.
#[test]
fn a_commit_pushed_through_a_since_removed_fork_push_url_refuses_the_root() {
    let fixture = ClonedMember::new("push-removed-fork-url");
    let fork = fixture.temp.path().join("fork.git");
    init_bare_main(&fork);
    let work = fixture.commit_work();
    fixture
        .repo()
        .remote_set_pushurl("origin", Some(fork.to_str().unwrap()))
        .unwrap();
    fixture.lock(&work);
    let published = fixture.push_members();
    let member = published.response.members.single();
    assert_eq!(member.status, crate::MemberStatus::Ok, "{:?}", member.error);
    assert_eq!(read_repo_ref(&fork, "refs/heads/main"), Some(work.clone()));
    assert_eq!(
        read_repo_ref(&fixture.upstream.remote, "refs/heads/main"),
        Some(fixture.initial.clone())
    );
    fixture.repo().remote_set_pushurl("origin", None).unwrap();
    assert_eq!(fixture.last_known_main(), Some(work.clone()));

    let response = fixture.push_root_naming(&work);

    fixture.assert_root_refused(&response, &work);
    assert_eq!(
        read_repo_ref(&fixture.upstream.remote, "refs/heads/main"),
        Some(fixture.initial.clone())
    );
}

/// Step 3.6, a renamed remote (§3.7): `origin` is removed first, because
/// `git remote rename` refuses an existing name, and then a fork remote whose
/// fetched tracking refs hold a commit only the fork has is renamed to
/// `origin`, which moves those refs. The member's `main` equals the moved ref,
/// so a default push does not contact the member, and the dependency read of
/// the committed upstream URL refuses the root.
#[test]
fn a_fork_remote_renamed_to_origin_leaves_the_member_uncontacted_and_refuses_the_root() {
    let fixture = ClonedMember::new("push-renamed-remote");
    let fork = fixture.temp.path().join("fork.git");
    init_bare_main(&fork);
    let work = fixture.commit_work();
    let backend = &fixture.backend;
    backend
        .add_remote(&fixture.app, "fork", fork.to_str().unwrap())
        .unwrap();
    backend
        .push(&fixture.app, "fork", "refs/heads/main:refs/heads/main")
        .unwrap();
    backend.fetch(&fixture.app, "fork").unwrap();
    assert_eq!(
        read_repo_ref(&fixture.app, "refs/remotes/fork/main"),
        Some(work.clone())
    );
    git(&fixture.app, &["remote", "remove", "origin"]);
    git(&fixture.app, &["remote", "rename", "fork", "origin"]);
    assert_eq!(fixture.last_known_main(), Some(work.clone()));

    let response = fixture.push_root_naming(&work);

    fixture.assert_root_refused(&response, &work);
    assert_eq!(
        read_repo_ref(&fixture.upstream.remote, "refs/heads/main"),
        Some(fixture.initial.clone())
    );
}

/// Step 3.6, layouts whose tracking ref may not stand for `origin` (§3.5).
/// Each case starts from a clone whose `origin/main` equals the member's
/// branch and is its last-known ref, and changes one setting: a second remote
/// fetching into `refs/remotes/origin/*`, a `+refs/heads/*:refs/heads/*` fetch
/// refspec, a push URL naming another, empty repository, or a non-forced fetch
/// refspec. The Git2 query then gives no last-known ref, and a default push
/// contacts the member with one read. Upstream already holds the commit; the
/// push URL's repository does not, so that case pushes the commit there and
/// leaves upstream as it was. Core refuses the push URL case before it asks the
/// backend, so only the query shows that the backend refuses it too.
#[test]
fn a_layout_whose_tracking_ref_may_not_stand_for_origin_is_contacted() {
    fn fetch_only(repo: &git2::Repository, refspec: &str) {
        let mut config = repo.config().unwrap();
        config.remove_multivar("remote.origin.fetch", ".*").unwrap();
        repo.remote_add_fetch("origin", refspec).unwrap();
    }
    /// A named change to the member's remote configuration, given another
    /// repository's path, and whether that repository becomes the member's
    /// push destination.
    type Layout = (&'static str, fn(&git2::Repository, &str), bool);
    let layouts: [Layout; 4] = [
        (
            "a second remote fetching into refs/remotes/origin/*",
            |repo, other| {
                repo.remote_with_fetch("mirror", other, "+refs/heads/*:refs/remotes/origin/*")
                    .unwrap();
            },
            false,
        ),
        (
            "a +refs/heads/*:refs/heads/* fetch refspec",
            |repo, _| {
                fetch_only(repo, "+refs/heads/*:refs/heads/*");
            },
            false,
        ),
        (
            "a push URL naming another repository",
            |repo, other| {
                repo.remote_set_pushurl("origin", Some(other)).unwrap();
            },
            true,
        ),
        (
            "a non-forced fetch refspec",
            |repo, _| {
                fetch_only(repo, "refs/heads/*:refs/remotes/origin/*");
            },
            false,
        ),
    ];
    for (layout, change, pushed_to_other) in layouts {
        let fixture = ClonedMember::new("push-no-last-known-ref");
        // Another repository, which holds no commit.
        let other = fixture.temp.path().join("other.git");
        init_bare_main(&other);
        fixture.lock(&fixture.initial);
        assert_eq!(
            fixture.last_known_main(),
            Some(fixture.initial.clone()),
            "{layout}"
        );
        change(&fixture.repo(), other.to_str().unwrap());
        assert_eq!(fixture.last_known_main(), None, "{layout}");

        let response = fixture.push_members();

        let row = response.response.members.single();
        let reason = row
            .planned
            .as_ref()
            .and_then(|planned| planned.message.as_deref());
        let operations: Vec<_> = response
            .response
            .meta
            .transport
            .iter()
            .flatten()
            .map(|row| row.operation)
            .collect();
        let read = crate::TransportOperation::ReadAdvertisement;
        // The push URL case reads its empty destination and pushes the commit
        // there; every other case reads upstream, which already holds it.
        let (expected_row, expected_operations) = if pushed_to_other {
            (
                (crate::MemberStatus::Ok, None),
                vec![read, crate::TransportOperation::Push],
            )
        } else {
            (
                (crate::MemberStatus::Noop, Some("already on origin")),
                vec![read],
            )
        };
        assert_eq!(
            (row.status, reason),
            expected_row,
            "{layout}: {:?}",
            row.error
        );
        assert_eq!(operations, expected_operations, "{layout}");
        assert_eq!(
            read_repo_ref(&other, "refs/heads/main"),
            pushed_to_other.then(|| fixture.initial.clone()),
            "{layout}"
        );
        assert_eq!(
            read_repo_ref(&fixture.upstream.remote, "refs/heads/main"),
            Some(fixture.initial.clone()),
            "{layout}"
        );
    }
}

/// Create the annotated tag `name` on `commit` in `repo` with the `git` command
/// line, pinning the settings a developer's global configuration may change
/// (a signing default would fail or prompt).
fn annotated_tag(repo: &Path, name: &str, commit: &str) {
    git(
        repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "tag.gpgSign=false",
            "-c",
            "tag.forceSignAnnotated=false",
            "tag",
            "-a",
            name,
            "-m",
            name,
            commit,
        ],
    );
}

/// The root row of a root-only push: refused with the §3.6 remedies for
/// `commit`, not with a Git error, and the root remote left at `published`.
/// `fetch_only` is a refusal that comes from objects the member does not hold,
/// which only a fetch repairs, so it names no other remedy.
fn assert_root_only_refused(
    fixture: &ClonedMember,
    response: &crate::PushResponse,
    commit: &str,
    fetch_only: bool,
) {
    let root = response.response.members.single();
    assert_eq!(root.member_id, "@root");
    assert_eq!(root.status, crate::MemberStatus::Rejected);
    let error = root.error.as_ref().unwrap();
    assert_eq!(
        error.code,
        crate::GwzErrorCode::RemoteRejected,
        "{}",
        error.message
    );
    let proof = format!(
        "cannot prove member mem_app commit {commit} is available at its committed fetch remote origin;"
    );
    assert!(error.message.contains(&proof), "{}", error.message);
    assert!(
        error
            .message
            .contains("fetch its advertised history and retry"),
        "{}",
        error.message
    );
    for other in ["publish the member", "--check-remotes"] {
        assert_eq!(
            error.message.contains(other),
            !fetch_only,
            "{other}: {}",
            error.message
        );
    }
    assert_eq!(
        read_repo_ref(&fixture.root_remote, "refs/heads/main"),
        Some(fixture.published_root.clone())
    );
}

/// After the member fetches, the root-only push publishes the root at the
/// commit a plain `git push` sets.
fn assert_root_published_after_fetch(fixture: &ClonedMember, root_head: String) {
    fixture.backend.fetch(&fixture.app, "origin").unwrap();
    let response = fixture.push_root_only();

    let root = response.response.members.single();
    assert_eq!(root.status, crate::MemberStatus::Ok, "{:?}", root.error);
    assert_eq!(
        read_repo_ref(&fixture.root_remote, "refs/heads/main"),
        Some(root_head)
    );
}

/// Hotfix 1.0.18: another clone moved the member's remote on to a commit the
/// member has never fetched. The remote advertises only that commit, so the
/// proof asks the member's repository whether the locked commit is its
/// ancestor, and the repository does not hold it. That object was read as a
/// local Git failure (libgit2's "object not found - no match for id"), which
/// refused the root on every push, naming an id found in no local repository,
/// while a plain `git push` of the root succeeded. An advertised object the
/// member does not hold proves nothing here: the root is refused naming the
/// fetch, the only remedy that repairs it, and once the member fetches, the
/// root is published at the commit a plain `git push` sets.
#[test]
fn a_remote_commit_the_member_never_fetched_refuses_the_root_until_a_fetch() {
    let fixture = ClonedMember::new("push-unfetched-remote-commit");
    let backend = &fixture.backend;
    let root_head = fixture.commit_root_lock(&fixture.initial);
    let later = fixture
        .upstream
        .commit_and_push("README.md", "two", "later", backend);
    assert!(!backend.commit_exists(&fixture.app, &later).unwrap());

    let response = fixture.push_root_only();

    assert_root_only_refused(&fixture, &response, &fixture.initial, true);
    let message = &response
        .response
        .members
        .single()
        .error
        .as_ref()
        .unwrap()
        .message;
    assert!(!message.contains(&later), "{message}");

    assert_root_published_after_fetch(&fixture, root_head);
}

/// Hotfix 1.0.18: an annotated tag is advertised as its tag object, which is
/// not a commit, and again as the commit it peels to (`refs/tags/v1^{}`).
/// Asking whether the locked commit is an ancestor of the tag object failed
/// (libgit2's "object is no commit object") and that failure replaced the
/// refusal of a commit the remote does not hold. A held tag object proves
/// nothing itself; the peeled row decides. So an unpublished commit is refused
/// with the §3.6 remedies, and a commit published only under an annotated tag
/// is proven by that row.
#[test]
fn an_annotated_tag_on_the_remote_is_decided_by_its_peeled_row() {
    let fixture = ClonedMember::new("push-annotated-tag");
    let backend = &fixture.backend;
    annotated_tag(&fixture.upstream.source, "v1", &fixture.initial);
    git(
        &fixture.upstream.source,
        &["push", "origin", "refs/tags/v1"],
    );
    // The member holds the tag object, so only its kind can fail the query.
    git(
        &fixture.app,
        &["fetch", "origin", "refs/tags/v1:refs/tags/v1"],
    );
    let v1 = read_repo_ref(&fixture.app, "refs/tags/v1").unwrap();
    assert_ne!(v1, fixture.initial);
    let work = fixture.commit_work();
    let root_head = fixture.commit_root_lock(&work);

    let response = fixture.push_root_only();

    assert_root_only_refused(&fixture, &response, &work, false);

    annotated_tag(&fixture.app, "v2", &work);
    backend
        .push(&fixture.app, "origin", "refs/tags/v2:refs/tags/v2")
        .unwrap();
    let response = fixture.push_root_only();

    let root = response.response.members.single();
    assert_eq!(root.status, crate::MemberStatus::Ok, "{:?}", root.error);
    assert_eq!(
        read_repo_ref(&fixture.root_remote, "refs/heads/main"),
        Some(root_head)
    );
}

/// Another clone pushed the member's commit L (the one the root lock names)
/// and M on top, and published `refs/heads/old` or an annotated tag at the
/// initial commit. This clone fetched none of it, then pushed the root alone
/// (`gwz --target @root push`). `hold` makes the member hold that older
/// advertised object.
fn lock_a_commit_the_member_never_fetched(
    fixture: &ClonedMember,
    hold: impl FnOnce(&ClonedMember),
) -> (String, String) {
    hold(fixture);
    let locked = fixture
        .upstream
        .commit_and_push("README.md", "two", "locked", &fixture.backend);
    fixture
        .upstream
        .commit_and_push("README.md", "three", "later", &fixture.backend);
    assert!(
        !fixture
            .backend
            .commit_exists(&fixture.app, &locked)
            .unwrap()
    );
    let root_head = fixture.commit_root_lock(&locked);
    (locked, root_head)
}

/// Hotfix 1.0.18 State P2-1: the member lacks the locked commit but holds an
/// unrelated advertised branch. Asking ancestry of the held branch fails on the
/// locked commit, the member's missing object, which was reported as the
/// root's Git error on every try (while a fetch repaired it). The proof
/// refuses, naming the fetch alone, and after the fetch publishes the root.
#[test]
fn a_locked_commit_the_member_never_fetched_is_refused_though_it_holds_a_branch() {
    let fixture = ClonedMember::new("push-unfetched-locked-branch");
    let (locked, root_head) = lock_a_commit_the_member_never_fetched(&fixture, |fixture| {
        git(
            &fixture.upstream.source,
            &[
                "push",
                "origin",
                &format!("{}:refs/heads/old", fixture.initial),
            ],
        );
        git(
            &fixture.app,
            &["fetch", "origin", "refs/heads/old:refs/heads/old"],
        );
    });

    let response = fixture.push_root_only();

    assert_root_only_refused(&fixture, &response, &locked, true);
    assert_root_published_after_fetch(&fixture, root_head);
}

/// As the row above, with the older advertised object an annotated tag the
/// member holds instead of a branch.
#[test]
fn a_locked_commit_the_member_never_fetched_is_refused_though_it_holds_a_tag() {
    let fixture = ClonedMember::new("push-unfetched-locked-tag");
    let (locked, root_head) = lock_a_commit_the_member_never_fetched(&fixture, |fixture| {
        annotated_tag(&fixture.upstream.source, "v1", &fixture.initial);
        git(
            &fixture.upstream.source,
            &["push", "origin", "refs/tags/v1"],
        );
        git(
            &fixture.app,
            &["fetch", "origin", "refs/tags/v1:refs/tags/v1"],
        );
    });

    let response = fixture.push_root_only();

    assert_root_only_refused(&fixture, &response, &locked, true);
    assert_root_published_after_fetch(&fixture, root_head);
}

/// Hotfix 1.0.18 (Code OOS-1): a member tag push the remote accepted proves the
/// root's locked commit (D8), but the pushed source of an annotated tag is the
/// tag object, which is not a commit. Asking ancestry of it failed (libgit2's
/// "object is no commit object") and rejected the root although the member's
/// push was accepted and the remote holds the commit. The tag object peels to
/// its commit, which the pushed history proves; the root tag is then pushed.
#[test]
fn a_pushed_annotated_tag_proves_the_root_dependency_through_its_commit() {
    let fixture = ClonedMember::new("push-annotated-tag-refspec");
    let work = fixture.commit_work();
    let root_head = fixture.commit_root_lock(&work);
    annotated_tag(&fixture.app, "v9", &work);
    annotated_tag(fixture.temp.path(), "v9", &root_head);
    let request = crate::PushRequest {
        meta: request_meta_with_workspace(),
        refspec: Some("refs/tags/v9:refs/tags/v9".to_owned()),
        ..Default::default()
    };

    let response = handle_push(&fixture.backend, fixture.temp.path(), request, "tag").unwrap();

    let rows = &response.response.members;
    let row = |id: &str| rows.iter().find(|row| row.member_id == id).unwrap();
    for id in ["mem_app", "@root"] {
        assert_eq!(
            row(id).status,
            crate::MemberStatus::Ok,
            "{id}: {:?}",
            row(id).error
        );
    }
    let v9 = read_repo_ref(&fixture.app, "refs/tags/v9").unwrap();
    assert_eq!(
        read_repo_ref(&fixture.upstream.remote, "refs/tags/v9"),
        Some(v9)
    );
    let root_v9 = read_repo_ref(fixture.temp.path(), "refs/tags/v9");
    assert!(root_v9.is_some());
    assert_eq!(read_repo_ref(&fixture.root_remote, "refs/tags/v9"), root_v9);
}

/// The row above must not prove more than the push did: an annotated tag on an
/// older commit than the locked one is accepted for the member, but the
/// locked commit is not under it, so the root is refused with the remedies and
/// its tag is not pushed.
#[test]
fn a_pushed_annotated_tag_below_the_locked_commit_does_not_prove_the_root() {
    let fixture = ClonedMember::new("push-annotated-tag-below");
    let work = fixture.commit_work();
    let root_head = fixture.commit_root_lock(&work);
    annotated_tag(&fixture.app, "v9", &fixture.initial);
    annotated_tag(fixture.temp.path(), "v9", &root_head);
    let request = crate::PushRequest {
        meta: request_meta_with_workspace(),
        refspec: Some("refs/tags/v9:refs/tags/v9".to_owned()),
        ..Default::default()
    };

    let response = handle_push(&fixture.backend, fixture.temp.path(), request, "tag").unwrap();

    let rows = &response.response.members;
    let row = |id: &str| rows.iter().find(|row| row.member_id == id).unwrap();
    assert_eq!(row("mem_app").status, crate::MemberStatus::Ok);
    let root = row("@root");
    assert_eq!(root.status, crate::MemberStatus::Rejected);
    let error = root.error.as_ref().unwrap();
    assert_eq!(
        error.code,
        crate::GwzErrorCode::RemoteRejected,
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .contains(&format!("commit {work} is available"))
    );
    assert_eq!(read_repo_ref(&fixture.root_remote, "refs/tags/v9"), None);
}
