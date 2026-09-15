//! A push through a named remote whose push URL is a local path other than its
//! fetch URL (gwz-dev `dev-docs/GwzUrlSchemePushPlan.md`, step 3.6, "Defect
//! found"). libgit2 1.9.7's local transport reads the push URL's advertisement
//! but writes the pack and the refs into the fetch URL's repository
//! (`transports/local.c`, `local_push`). The cases push the two ways gwz pushes
//! through a named remote, `Git2Backend::push` and `prepare_push` with
//! `push_prepared`, against local bare remotes, and check where the commit
//! landed.
use std::path::{Path, PathBuf};

use crate::git::{Git2Backend, GitBackend};

use super::*;

/// A repository at `work` whose `origin` fetches from the bare `upstream` and
/// pushes to the bare `fork`, with `main` at `base`, which neither holds.
struct SplitRemote {
    temp: TempDir,
    backend: Git2Backend,
    work: PathBuf,
    upstream: PathBuf,
    fork: PathBuf,
    base: String,
}

impl SplitRemote {
    fn new(name: &str) -> Self {
        let temp = TempDir::new(name);
        let backend = Git2Backend::without_credential_helpers();
        let work = temp.path().join("work");
        let upstream = temp.path().join("upstream.git");
        let fork = temp.path().join("fork.git");
        init_bare_main(&upstream);
        init_bare_main(&fork);
        backend.create_repo(&work).unwrap();
        let base = commit_file(&work, "README.md", "base", "base", &[]).unwrap();
        backend
            .add_remote(&work, "origin", upstream.to_str().unwrap())
            .unwrap();
        git2::Repository::open(&work)
            .unwrap()
            .remote_set_pushurl("origin", Some(fork.to_str().unwrap()))
            .unwrap();
        Self {
            temp,
            backend,
            work,
            upstream,
            fork,
            base,
        }
    }

    fn repo(&self) -> git2::Repository {
        git2::Repository::open(&self.work).unwrap()
    }

    /// Publish `main` as it stands to the bare repository at `bare`, through no
    /// named remote, so that no remote-tracking ref is written.
    fn publish(&self, bare: &Path) {
        self.backend
            .push_anonymous(
                &self.work,
                bare.to_str().unwrap(),
                "refs/heads/main:refs/heads/main",
            )
            .unwrap();
    }

    /// Publish a child of `base` as upstream's `main`, then return `main` to
    /// `base`. Gives the child.
    fn publish_upstream_child(&self) -> String {
        let base = git2::Oid::from_str(&self.base).unwrap();
        let child = commit_file(&self.work, "README.md", "upstream", "upstream", &[base]).unwrap();
        self.publish(&self.upstream);
        self.repo()
            .reference("refs/heads/main", base, true, "return to base")
            .unwrap();
        child
    }

    /// A child of `base` on `main` that neither remote holds.
    fn commit_work(&self) -> String {
        let base = git2::Oid::from_str(&self.base).unwrap();
        commit_file(&self.work, "work.txt", "work", "work", &[base]).unwrap()
    }

    /// Push `main` through `origin`, directly or through a captured plan.
    fn push(&self, via_plan: bool) -> crate::model::ModelResult<()> {
        let refspec = "refs/heads/main:refs/heads/main";
        if via_plan {
            let plan = self.backend.prepare_push(&self.work, "origin", refspec)?;
            self.backend.push_prepared(&self.work, &plan).map(drop)
        } else {
            self.backend.push(&self.work, "origin", refspec).map(drop)
        }
    }

    /// The push succeeded and landed in the fork alone: the fork's `main` is
    /// `tip`, upstream's `main` is still `upstream_main`, and upstream never
    /// received `tip`. `origin/main` records `tip`, as it does after any push
    /// through the named remote, and after `git push`.
    fn assert_pushed_to_fork(
        &self,
        via_plan: bool,
        pushed: crate::model::ModelResult<()>,
        tip: &str,
        upstream_main: Option<&str>,
    ) {
        let entry = if via_plan { "push_prepared" } else { "push" };
        assert!(pushed.is_ok(), "{entry}: {pushed:?}");
        let tip_id = git2::Oid::from_str(tip).unwrap();
        let upstream_received_tip = git2::Repository::open(&self.upstream)
            .unwrap()
            .find_commit(tip_id)
            .is_ok();
        assert_eq!(
            (
                read_repo_ref(&self.fork, "refs/heads/main"),
                read_repo_ref(&self.upstream, "refs/heads/main"),
                upstream_received_tip,
                read_repo_ref(&self.work, "refs/remotes/origin/main"),
            ),
            (
                Some(tip.to_owned()),
                upstream_main.map(ToOwned::to_owned),
                false,
                Some(tip.to_owned()),
            ),
            "{entry}: the fork's main, upstream's main, whether upstream received the tip, and origin/main"
        );
    }
}

/// Neither remote has `main`. libgit2 alone wrote `main` into upstream,
/// reported success and left the fork empty.
#[test]
fn a_push_through_a_local_push_url_to_empty_remotes_lands_in_the_push_url_repository() {
    for via_plan in [false, true] {
        let remotes = SplitRemote::new("push-url-empty");

        let pushed = remotes.push(via_plan);

        remotes.assert_pushed_to_fork(via_plan, pushed, &remotes.base, None);
    }
}

/// Upstream already has `main` and the fork is empty. libgit2 alone saw no
/// `main` in the fork, created upstream's `main` without forcing, and failed
/// with "a reference with that name already exists" after writing its pack
/// into upstream.
#[test]
fn a_push_through_a_local_push_url_is_not_refused_for_the_fetch_url_repositorys_branch() {
    for via_plan in [false, true] {
        let remotes = SplitRemote::new("push-url-upstream-branch");
        let upstream = remotes.publish_upstream_child();
        let work = remotes.commit_work();

        let pushed = remotes.push(via_plan);

        remotes.assert_pushed_to_fork(via_plan, pushed, &work, Some(&upstream));
    }
}

/// The fork holds `base`, upstream holds a child of it, and `main` is another
/// child. libgit2 alone saw `base` in the fork, forced upstream's `main` to the
/// new commit and lost upstream's own. Upstream now keeps its commit, and the
/// fork fast-forwards.
#[test]
fn a_push_through_a_local_push_url_never_overwrites_the_fetch_url_repositorys_branch() {
    for via_plan in [false, true] {
        let remotes = SplitRemote::new("push-url-data-loss");
        remotes.publish(&remotes.fork);
        let upstream = remotes.publish_upstream_child();
        let work = remotes.commit_work();

        let pushed = remotes.push(via_plan);

        remotes.assert_pushed_to_fork(via_plan, pushed, &work, Some(&upstream));
    }
}

/// A `url.<base>.pushInsteadOf` rule matching the push URL gives an anonymous
/// remote for it one URL to read and another to write, which is the defect
/// again. The push is refused and writes nothing anywhere.
#[test]
fn a_local_push_url_that_push_rewriting_would_split_again_is_refused() {
    let remotes = SplitRemote::new("push-url-rewritten");
    let decoy = remotes.temp.path().join("decoy.git");
    init_bare_main(&decoy);
    remotes
        .repo()
        .config()
        .unwrap()
        .set_str(
            &format!("url.{}.pushInsteadOf", decoy.to_str().unwrap()),
            remotes.fork.to_str().unwrap(),
        )
        .unwrap();

    for via_plan in [false, true] {
        let error = remotes.push(via_plan).unwrap_err();
        assert!(
            error.message.contains("pushInsteadOf") && error.message.contains("nothing was pushed"),
            "{}",
            error.message
        );
    }

    for bare in [&remotes.upstream, &remotes.fork, &decoy] {
        assert_eq!(
            read_repo_ref(bare, "refs/heads/main"),
            None,
            "{}",
            bare.display()
        );
    }
    assert_eq!(
        read_repo_ref(&remotes.work, "refs/remotes/origin/main"),
        None
    );
}

/// One ref of a repository: its name, its commit's summary and its reflog
/// messages.
type RefRow = (String, String, Vec<String>);

/// One push's outcome, then the refs of the pushing repository and of the
/// destination after it.
type PushStep = (Result<(), String>, Vec<RefRow>, Vec<RefRow>);

fn refs_and_reflogs(path: &Path) -> Vec<RefRow> {
    let repo = git2::Repository::open(path).unwrap();
    let mut rows: Vec<RefRow> = repo
        .references()
        .unwrap()
        .map(|reference| {
            let reference = reference.unwrap();
            let name = reference.name().unwrap().to_owned();
            let commit = reference.peel_to_commit().unwrap();
            let messages = repo
                .reflog(&name)
                .unwrap()
                .iter()
                .map(|entry| entry.message().unwrap().unwrap_or_default().to_owned())
                .collect();
            let summary = commit.summary().unwrap().unwrap_or_default().to_owned();
            (name, summary, messages)
        })
        .collect();
    rows.sort();
    rows
}

/// Push `main`, then `main` as `topic`, then the deletion of `main` into the
/// fork, through an `origin` whose fetch refspecs are `layout` and which names
/// the fork by its URL or, with `push_url`, by its push URL next to upstream as
/// its URL. Upstream stays empty either way.
fn pushes_with_fetch_refspecs(layout: &[&str], push_url: bool) -> Vec<PushStep> {
    let remotes = SplitRemote::new("push-url-tracking-refs");
    let repo = remotes.repo();
    if !push_url {
        repo.remote_set_url("origin", remotes.fork.to_str().unwrap())
            .unwrap();
        repo.remote_set_pushurl("origin", None).unwrap();
    }
    let mut config = repo.config().unwrap();
    config.remove_multivar("remote.origin.fetch", ".*").unwrap();
    for refspec in layout {
        repo.remote_add_fetch("origin", refspec).unwrap();
    }
    let steps = [
        "refs/heads/main:refs/heads/main",
        "refs/heads/main:refs/heads/topic",
        ":refs/heads/main",
    ]
    .into_iter()
    .map(|refspec| {
        let pushed = remotes
            .backend
            .push(&remotes.work, "origin", refspec)
            .map(drop)
            .map_err(|error| error.message);
        (
            pushed,
            refs_and_reflogs(&remotes.work),
            refs_and_reflogs(&remotes.fork),
        )
    })
    .collect();
    assert!(
        refs_and_reflogs(&remotes.upstream).is_empty(),
        "{layout:?}, push URL: {push_url}"
    );
    steps
}

/// libgit2 updates remote-tracking refs itself after a push through a named
/// remote; after a push through a local push URL, gwz updates them. For each
/// layout of fetch refspecs, pushing into the fork through `origin`'s push URL
/// leaves every ref and reflog message, in the pushing repository and in the
/// fork, as pushing through `origin`'s URL does. The layouts: the default, a
/// non-forced mapping, one branch, destinations libgit2 expands from
/// `remotes/` and from a bare name, a negative refspec, a mirror into
/// `refs/heads/`, two mappings of one branch where the first wins, and a
/// refspec without a destination, whose empty tracking ref name fails the push
/// after the transfer.
#[test]
fn a_push_through_a_local_push_url_updates_tracking_refs_as_libgit2_does() {
    let layouts: [&[&str]; 9] = [
        &["+refs/heads/*:refs/remotes/origin/*"],
        &["refs/heads/*:refs/remotes/origin/*"],
        &["+refs/heads/main:refs/remotes/origin/main"],
        &["+refs/heads/*:remotes/origin/*"],
        &["+refs/heads/*:origin/*"],
        &["+refs/heads/*:refs/remotes/origin/*", "^refs/heads/main"],
        &["+refs/heads/*:refs/heads/*"],
        &[
            "+refs/heads/main:refs/remotes/origin/first",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
        &["refs/heads/main"],
    ];
    for layout in layouts {
        assert_eq!(
            pushes_with_fetch_refspecs(layout, true),
            pushes_with_fetch_refspecs(layout, false),
            "{layout:?}"
        );
    }
}
