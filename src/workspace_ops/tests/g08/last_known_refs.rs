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

    /// Commit a root lock that names `commit`, and push the whole workspace by
    /// default.
    fn push_root_naming(&self, commit: &str) -> crate::PushResponse {
        self.lock(commit);
        self.backend
            .stage_paths(self.temp.path(), &["gwz.conf"])
            .unwrap();
        self.backend
            .commit(self.temp.path(), "lock", false)
            .unwrap();
        let request = crate::PushRequest {
            meta: request_meta_with_workspace(),
            ..Default::default()
        };
        handle_push(&self.backend, self.temp.path(), request, "push").unwrap()
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
