//! The words of the ambient (all-keys) agent route, from a real failed open (the diagnosability parity fix, finding F4
//! of run `2026-10-18-gapsb`): the agent answers and the server takes no key, on Unix and with the Windows pipe agent.
//! The driver's wiring of the words is in `transport_host::session::driver`'s rows.
use super::local_endpoint::{endpoint, exchange, failure, finish};
use super::login_fixture as support;
use gwz_transport::protocol::{AuthMethod, ErrorCode, Failure};
use std::fs;

fn failed_open(f: &support::Fixture) -> Failure {
    let e = endpoint(&f.ssh, f.source());
    let ended = failure(&exchange(&e, &f.ssh, None).unwrap_err());
    finish(&e);
    ended
}

#[test]
fn an_agent_whose_keys_the_server_does_not_authorize_is_worded_as_offered_and_not_accepted() {
    let f = support::Fixture::new(support::METHOD, false);
    // The server trusts a key the agent does not hold.
    fs::copy(
        f.ssh.temp.path().join("client_ed25519.pub"),
        f.ssh.temp.path().join("authorized_keys"),
    )
    .unwrap();
    let ended = failed_open(&f);
    assert_eq!(ended.code, ErrorCode::Authentication, "{ended:?}");
    let facts = ended.facts.clone().expect("the step's facts");
    assert_eq!(facts.method, AuthMethod::SshAgent);
    assert!(facts.credential_offered);
    assert_eq!(facts.authenticated, Some(false));
    let said = f.source().ambient_reason(&ended, "127.0.0.1", None);
    let said = said.expect("the open's own agent route has words");
    assert!(
        said.starts_with("GWZ offered 127.0.0.1 every key the "),
        "{said}"
    );
    assert!(
        said.contains("and 127.0.0.1 accepted none of them."),
        "{said}"
    );
}

#[test]
fn an_agent_that_holds_no_key_is_worded_as_nothing_offered() {
    let f = support::Fixture::without_keys(support::METHOD);
    let ended = failed_open(&f);
    assert_eq!(ended.code, ErrorCode::Authentication, "{ended:?}");
    let facts = ended.facts.clone().expect("the step's facts");
    assert!(!facts.credential_offered, "{facts:?}");
    assert_eq!(
        facts.method,
        AuthMethod::SshAgent,
        "the failure says it was the agent step"
    );
    let said = f.source().ambient_reason(&ended, "127.0.0.1", None);
    let said = said.expect("the open's own agent route has words");
    assert!(
        said.contains(" has no key GWZ can offer 127.0.0.1: it holds no key"),
        "{said}"
    );
    assert!(said.contains("GWZ offered 127.0.0.1 nothing."), "{said}");
}
