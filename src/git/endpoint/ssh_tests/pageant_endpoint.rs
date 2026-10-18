//! Plan step 3.6's rows through the whole endpoint, with the loopback `sshd.exe` and a synthetic Pageant: a window of
//! this process that answers `WM_COPYDATA` as Pageant does, with the keys of the fixture's agent (so it signs for real,
//! and the server's acceptance is the proof). Selection is the product's rule (`select`) over a pin taken from the
//! synthetic window; the probe that finds a window by class and title is `agent_pageant`'s own row. **No parity claim
//! rests on these**: PuTTY's own `pageant.exe` is `real_pageant.rs`'s, and owed to the console session.
use super::local_endpoint::{endpoint, exchange, failure, finish};
use super::{agent_fixture_pipe::Fixture as Keys, login_fixture as support};
use crate::git::endpoint::{
    agent_pageant::pin_of,
    agent_source::{AgentSource, select},
    pageant_exchange::receiver::{Receiver, Reply},
};
use gwz_transport::protocol::{AuthMethod, ErrorCode, SetupFailureCause};
use std::ffi::OsStr;

const NO_KEYS: [u8; 5] = [12, 0, 0, 0, 0];

/// The source a runtime would choose with Pageant at `pageant` and `sock` as its `SSH_AUTH_SOCK`.
fn chosen(pageant: &Receiver, sock: &str) -> AgentSource {
    let pin = pin_of(pageant.window).expect("the synthetic window has an owner");
    select(Some(OsStr::new(sock)), || Some(pin))
}

#[test]
fn a_visible_pageant_with_keys_authenticates_and_the_pipe_sees_no_request() {
    // B06: both agents are there. The server trusts the key Pageant holds; the pipe holds others, and records.
    let f = support::Fixture::new(support::METHOD, false);
    let pipe = Keys::new();
    let pageant = Receiver::start(Reply::Handler(Box::new(f.responder())));
    let source = chosen(&pageant, pipe.agent.name());
    assert!(matches!(
        source,
        AgentSource::Pageant {
            sock_ignored: true,
            ..
        }
    ));
    let e = endpoint(&f.ssh, source);
    let opened = exchange(&e, &f.ssh, None).unwrap();
    assert_eq!(opened.facts.method, AuthMethod::SshAgent);
    assert!(opened.facts.credential_offered);
    assert_eq!(opened.facts.authenticated, Some(true));
    // Pageant listed its keys and signed once, with the flag libssh2 on Windows offers.
    f.assert_requests(support::METHOD, 1);
    assert!(pipe.requests().is_empty(), "the pipe was never tried");
    finish(&e);
}

#[test]
fn a_visible_pageant_with_no_keys_refuses_and_the_pipe_that_would_authenticate_is_not_tried() {
    // X9: Pageant has no key, and the pipe the variable names holds the one the server trusts.
    let f = support::Fixture::new(support::METHOD, false);
    let pageant = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
    let e = endpoint(&f.ssh, chosen(&pageant, &f.path));
    let refused = failure(&exchange(&e, &f.ssh, None).unwrap_err());
    assert_eq!(refused.code, ErrorCode::Authentication, "{refused:?}");
    let facts = refused.facts.expect("the step's facts");
    assert_eq!(facts.method, AuthMethod::SshAgent);
    assert!(!facts.credential_offered, "no key was offered");
    f.no_requests();
    finish(&e);
}

#[test]
fn a_pageant_that_went_away_is_unavailable_and_the_pipe_is_not_tried() {
    let f = support::Fixture::new(support::METHOD, false);
    let pageant = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
    let source = chosen(&pageant, &f.path);
    drop(pageant);
    let e = endpoint(&f.ssh, source);
    let refused = failure(&exchange(&e, &f.ssh, None).unwrap_err());
    assert_eq!(refused.code, ErrorCode::Unavailable, "{refused:?}");
    assert_eq!(refused.setup_cause, Some(SetupFailureCause::NotFound));
    assert_eq!(refused.facts.unwrap().method, AuthMethod::SshAgent);
    f.no_requests();
    finish(&e);
}

#[test]
fn a_visible_pageant_whose_rejected_keys_are_worded_as_offered_and_not_accepted() {
    // The ambient route's words for Pageant: it holds keys, the server trusts a key it does not hold, and the pipe that
    // the variable names is never tried.
    let f = support::Fixture::new(support::METHOD, false);
    std::fs::copy(
        f.ssh.temp.path().join("client_ed25519.pub"),
        f.ssh.temp.path().join("authorized_keys"),
    )
    .unwrap();
    let pageant = Receiver::start(Reply::Handler(Box::new(f.responder())));
    let source = chosen(&pageant, &f.path);
    let e = endpoint(&f.ssh, source.clone());
    let refused = failure(&exchange(&e, &f.ssh, None).unwrap_err());
    assert_eq!(refused.code, ErrorCode::Authentication, "{refused:?}");
    let facts = refused.facts.clone().expect("the step's facts");
    assert_eq!(facts.method, AuthMethod::SshAgent);
    assert!(facts.credential_offered);
    assert_eq!(facts.authenticated, Some(false));
    let said = source
        .ambient_reason(&refused, "127.0.0.1", None)
        .expect("Pageant's keys that the server rejected have words");
    assert!(
        said.starts_with("GWZ offered 127.0.0.1 every key Pageant holds that it can use"),
        "{said}"
    );
    assert!(said.contains("Add Key"), "{said}");
    finish(&e);
}
