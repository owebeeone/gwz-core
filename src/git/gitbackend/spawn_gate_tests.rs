//! The backend's std spawns hold its host's spawn gate (WH2 contract C9; operator decision D25): the Git CLI
//! routes of `tag` and `commit`, and the call into libgit2's credential helper. Each row holds an excluding gate
//! itself, starts the site on another thread, and sees that the site does not finish until the hold is dropped.
use super::*;
use crate::git::spawn_gate::SpawnGate;
use std::{sync::mpsc, thread, time::Duration};

/// How long a site that must be blocked is given to prove it is not.
const BLOCKED: Duration = Duration::from_millis(500);

/// A repository with one commit, and a backend whose spawn gate is `gate`.
fn repository(gate: &SpawnGate) -> (tempfile::TempDir, Git2Backend) {
    let directory = tempfile::tempdir().unwrap();
    let repository = git2::Repository::init(directory.path()).unwrap();
    let signature = git2::Signature::now("Gate", "gate@example.invalid").unwrap();
    let tree = repository
        .find_tree(repository.treebuilder(None).unwrap().write().unwrap())
        .unwrap();
    repository
        .commit(Some("HEAD"), &signature, &signature, "one", &tree, &[])
        .unwrap();
    let mut backend = Git2Backend::new();
    backend.spawn_gate = gate.clone();
    (directory, backend)
}

/// Runs `site` on another thread while the test holds `gate`: it must not finish until the hold is dropped.
fn blocked_until_released(gate: &SpawnGate, what: &str, site: impl FnOnce() + Send) {
    let hold = gate.hold();
    let (finished, done) = mpsc::channel();
    thread::scope(|scope| {
        scope.spawn(|| {
            site();
            finished.send(()).unwrap();
        });
        assert!(
            done.recv_timeout(BLOCKED).is_err(),
            "{what} ran while another hold on the gate existed"
        );
        drop(hold);
        done.recv_timeout(Duration::from_secs(60))
            .unwrap_or_else(|_| panic!("{what} did not go on once the gate was released"));
    });
}

#[test]
fn git_tag_waits_for_the_backends_gate() {
    let gate = SpawnGate::with_exclusion(true);
    let (directory, backend) = repository(&gate);
    blocked_until_released(&gate, "git tag", || {
        refs::tag_create_cli(&backend, directory.path(), "gated", None, false).unwrap();
    });
    assert!(
        backend
            .tag_list(directory.path())
            .unwrap()
            .contains(&"gated".to_owned())
    );
}

#[test]
fn git_tag_delete_waits_for_the_backends_gate() {
    let gate = SpawnGate::with_exclusion(true);
    let (directory, backend) = repository(&gate);
    refs::tag_create_cli(&backend, directory.path(), "gated", None, false).unwrap();
    blocked_until_released(&gate, "git tag -d", || {
        refs::tag_delete_cli(&backend, directory.path(), "gated").unwrap();
    });
    assert!(backend.tag_list(directory.path()).unwrap().is_empty());
}

#[test]
fn git_commit_waits_for_the_backends_gate() {
    let gate = SpawnGate::with_exclusion(true);
    let (directory, backend) = repository(&gate);
    blocked_until_released(&gate, "git commit", || {
        // Whether the commit succeeds depends on the machine's identity; the row is about when it starts.
        let _ = repository::commit_cli(&backend, directory.path(), "two", true);
    });
}

#[test]
fn the_credential_helper_call_holds_the_gate_for_the_whole_call() {
    let gate = SpawnGate::with_exclusion(true);
    blocked_until_released(&gate, "libgit2's credential helper", || {
        // No level, so no helper is configured and nothing is spawned: the row is about the hold.
        let config = git2::Config::new().unwrap();
        let _ = transport_support::helper_credential(
            &config,
            "https://gate.example.invalid/x.git",
            None,
            &gate,
        );
    });
    assert!(gate.try_hold().is_some(), "the call released the gate");
}

#[test]
fn the_credentials_callback_holds_the_backends_gate_around_the_helper_call() {
    let gate = SpawnGate::with_exclusion(true);
    let mut backend = Git2Backend::new();
    backend.spawn_gate = gate.clone();
    blocked_until_released(&gate, "the credentials callback's helper call", || {
        // No level, so no helper is configured and nothing is spawned: the row is about the gate.
        let mut callback =
            transport_support::credential_callback(&backend, None, None, git2::Config::new);
        let credential = callback(
            "https://gate.example.invalid/x.git",
            None,
            git2::CredentialType::USER_PASS_PLAINTEXT,
        );
        assert!(credential.is_err());
    });
}
