//! D17 end to end: a clone whose selected key file is protected by a passphrase ends with words that name the file,
//! the cause and the two ways out (`ssh_limits`), through the endpoint's admission, the open's facts and the host's
//! wording, not with `ssh setup failed: InvalidRequest`. The key is a copy of the fixture's, given a passphrase with
//! `ssh-keygen`; the server is never asked to authenticate.
use super::*;
use crate::git::endpoint::fixture_host;
use std::process::Command;

#[test]
fn candidate_clone_with_a_passphrase_protected_key_file_says_what_is_wrong_and_what_to_do() {
    let f = common::SshdFixture::new();
    commit(
        &git2::Repository::open_bare(&f.repository).unwrap(),
        "first",
    );
    let locked = f.temp.path().join("client_locked");
    std::fs::copy(f.temp.path().join("client_ed25519"), &locked).unwrap();
    common::run(
        Command::new(fixture_host::programs().keygen)
            .args(["-q", "-p", "-P", "", "-N", "fixture-passphrase", "-f"])
            .arg(&locked),
    );
    let host = Host::new(&f);
    let mut meta = request_meta(&f, "locked");
    meta.transport.as_mut().unwrap().default_identity = Some(locked.display().to_string());
    let request = host.request(meta.clone(), "locked");
    let backend = request
        .backend()
        .with_transport(f.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let error = backend
        .clone_repo(&url(&f), &f.temp.path().join("clone-locked"))
        .expect_err("a key file with a passphrase cannot be used");
    let words = error.to_string();
    for needle in [
        &locked.display().to_string(),
        "is protected by a passphrase",
        "gwz cannot ask for a passphrase",
        "ssh-add",
        "ssh-keygen -p -f <copy of the key>",
    ] {
        assert!(words.contains(needle), "{needle}: {words}");
    }
    assert!(!words.contains("InvalidRequest"), "{words}");
    // A refusal of the file, not of the server: nothing authenticated, and the server was not asked to.
    assert_eq!(f.authenticated_sessions, 0);
    host.finish(request);
    host.shutdown();
}
