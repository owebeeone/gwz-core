//! A host key the endpoint does not trust is worded by what the trust check found (decision D12): the host is not
//! listed, or `known_hosts` lists it with a different key. A real server runs on every platform, with its own key
//! missing from the file and with another key of the same kind in its place. Nothing crosses the wire but `Trust`: the
//! endpoint records what it found, and the driver, in the same process, words the failure from the record.

use super::driver_tests::{block_on, commit, common, endpoint_home, fixture_url, local_meta};
use super::*;
use crate::git::GitBackend;
use crate::git::endpoint::ssh_fixture::run_fixture_key;
use std::fs;

/// What a clone through a runtime whose `known_hosts` holds `lines` (a function of the fixture's own host key line and
/// a foreign key's public line; `None` for no file) reports.
fn clone_error(lines: impl FnOnce(&str, &str) -> Option<String>) -> (String, std::path::PathBuf) {
    let fixture = common::SshdFixture::new();
    commit(
        &git2::Repository::open_bare(&fixture.repository).unwrap(),
        "first",
    );
    let foreign = fixture.temp.path().join("foreign");
    run_fixture_key(&foreign);
    let home = endpoint_home(&fixture);
    let known = home.join(".ssh/known_hosts");
    let own = fixture.known_host("127.0.0.1", fixture.port, false);
    let other = format!(
        "[127.0.0.1]:{} {}",
        fixture.port,
        fs::read_to_string(foreign.with_extension("pub")).unwrap()
    );
    match lines(&own, &other) {
        Some(text) => fs::write(&known, text).unwrap(),
        None => fs::remove_file(&known).unwrap(),
    }
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let meta = local_meta("host-key", &home);
    let request = block_on(runtime.request(meta.clone(), "clone".into())).unwrap();
    let backend = request
        .backend()
        .with_transport(fixture.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let error = backend
        .clone_repo(&fixture_url(&fixture), &fixture.temp.path().join("clone"))
        .err()
        .expect("the host key is not trusted");
    block_on(request.finish());
    block_on(runtime.shutdown());
    (error.message, known)
}

/// `path` as the words show it: the endpoint's home joins `.ssh\known_hosts` with the platform's separator, where the
/// test wrote the file with a `/`.
fn shown(path: &std::path::Path) -> String {
    path.display()
        .to_string()
        .replace('/', std::path::MAIN_SEPARATOR_STR)
}

#[test]
fn a_host_that_is_not_listed_gets_the_keyscan_guidance_and_no_claim_of_a_changed_key() {
    // The file lists another host only.
    let (message, known) = clone_error(|own, _| Some(own.replace("127.0.0.1", "other.invalid")));
    let path = shown(&known);
    for needle in [
        "does not list it",
        "ssh-keyscan",
        "ssh-keygen -lf",
        "fingerprint",
        &path,
    ] {
        assert!(message.contains(needle), "{needle}: {message}");
    }
    for claim in ["differs", "different key", "attack", "re-keyed"] {
        assert!(!message.contains(claim), "{claim}: {message}");
    }
}

#[test]
fn a_missing_file_is_a_host_that_is_not_listed() {
    let (message, _) = clone_error(|_, _| None);
    assert!(message.contains("does not list it"), "{message}");
    assert!(!message.contains("differs"), "{message}");
}

#[test]
fn a_changed_key_says_so_plainly_with_its_line_and_prints_no_fix() {
    // Line 1 lists another host; line 2 lists this host with another key of the same kind; line 3 lists it again.
    let (message, known) = clone_error(|own, other| {
        Some(format!(
            "{}{other}{other}",
            own.replace("127.0.0.1", "other.invalid")
        ))
    });
    let path = shown(&known);
    for needle in [
        "differs from the one recorded in",
        &path,
        "line 2",
        "may be an attack",
        "re-keyed",
        "fingerprint",
        "out of band",
        "only after",
    ] {
        assert!(message.contains(needle), "{needle}: {message}");
    }
    // Neither a command that adds a key nor one that removes an entry is printed.
    for fix in ["ssh-keyscan", "ssh-keygen -R", "Add-Content", ">>"] {
        assert!(!message.contains(fix), "{fix}: {message}");
    }
    assert!(!message.contains("does not list"), "{message}");
}
