#![allow(dead_code, unused_imports)]
mod common;
use common::{SshdFixture, ssh_channel, ssh_connection};
#[path = "common/pooled.rs"]
mod pooled;
#[path = "../../../src/git/endpoint/ssh_pool.rs"]
mod ssh_pool;
#[path = "../../../src/git/endpoint/ssh_pump.rs"]
mod ssh_pump;
#[path = "../../../src/git/endpoint/ssh_remote.rs"]
mod ssh_remote;
#[path = "../../../src/git/endpoint/stream_io.rs"]
mod stream_io;

use git2::{FetchOptions, PushOptions, Repository, Signature, build::RepoBuilder};
use std::{path::Path, sync::atomic::Ordering};

fn commit(repo: &Repository, seed: u32) -> git2::Oid {
    let mut state = seed;
    let data: Vec<_> = (0..70_000)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 24) as u8
        })
        .collect();
    let blob = repo.blob(&data).unwrap();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert("payload", blob, 0o100644).unwrap();
    let tree = repo.find_tree(builder.write().unwrap()).unwrap();
    let signature = Signature::now("fixture", "fixture@example.invalid").unwrap();
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    repo.set_head("refs/heads/main").unwrap();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "payload",
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .unwrap()
}
fn fetch_options(host: &pooled::Harness) -> FetchOptions<'static> {
    let mut options = FetchOptions::new();
    options.remote_callbacks(ssh_remote::callbacks(host.endpoint.clone()));
    options
}
fn push(repo: &Repository, url: &str, host: &pooled::Harness) {
    let mut remote = repo.remote_anonymous(url).unwrap();
    let mut options = PushOptions::new();
    options.remote_callbacks(ssh_remote::callbacks(host.endpoint.clone()));
    remote
        .push(&["refs/heads/main:refs/heads/main"], Some(&mut options))
        .unwrap();
    remote.disconnect().unwrap();
}
fn clone_repo(host: &pooled::Harness, url: &str, path: &Path) -> Repository {
    RepoBuilder::new()
        .fetch_options(fetch_options(host))
        .clone(url, path)
        .unwrap()
}

#[test]
fn native_git_clone_push_fetch_share_one_ssh_connection_across_remote_repositories() {
    let mut fixture = SshdFixture::new();
    let root = fixture.temp.path();
    let source = Repository::init(root.join("source")).unwrap();
    let first = commit(&source, 17);
    source
        .remote_anonymous(fixture.repository.to_str().unwrap())
        .unwrap()
        .push(&["refs/heads/main:refs/heads/main"], None)
        .unwrap();
    let second_path = root.join("second.git");
    Repository::init_bare(&second_path)
        .unwrap()
        .set_head("refs/heads/main")
        .unwrap();
    let url1 = format!(
        "ssh://{}@127.0.0.1:{}/first.git",
        fixture.user, fixture.port
    );
    let url2 = format!(
        "ssh://{}@127.0.0.1:{}/second.git",
        fixture.user, fixture.port
    );
    let routes = [
        (url1.clone(), fixture.repository.clone()),
        (url2.clone(), second_path.clone()),
    ];
    let session = fixture.session();
    let host = pooled::Harness::new(
        session,
        fixture.user.clone(),
        fixture.port,
        routes.into_iter().collect(),
    );
    let clone1 = clone_repo(&host, &url1, &fixture.temp.path().join("clone-one"));
    assert_eq!(clone1.head().unwrap().target(), Some(first));
    push(&clone1, &url2, &host);
    let clone2 = clone_repo(&host, &url2, &fixture.temp.path().join("clone-two"));
    assert_eq!(clone2.head().unwrap().target(), Some(first));
    let second = commit(&source, 23);
    push(&source, &url1, &host);
    clone1
        .find_remote("origin")
        .unwrap()
        .fetch(
            &["refs/heads/main:refs/remotes/origin/main"],
            Some(&mut fetch_options(&host)),
            None,
        )
        .unwrap();
    assert_eq!(
        clone1
            .find_reference("refs/remotes/origin/main")
            .unwrap()
            .target(),
        Some(second)
    );
    assert_eq!(
        host.opens.load(Ordering::SeqCst),
        1,
        "a second physical connection was opened"
    );
    assert!(host.commands.load(Ordering::SeqCst) >= 5);
    assert_eq!(fixture.authenticated_sessions, 1);
    println!(
        "physical opens={}, Git service channels={}, authenticated sessions={}; clone/push/fetch object IDs verified",
        host.opens.load(Ordering::SeqCst),
        host.commands.load(Ordering::SeqCst),
        fixture.authenticated_sessions
    );
    host.finish();
    assert!(
        !fixture.marker.exists(),
        "repository operand was shell-injected"
    );
}
