//! The fixture's dialer counts the connections it abandons and replaces, so that a row that counts the connections a
//! proxy accepted can subtract them. libssh2 on Windows CNG ends about one key exchange in a hundred on its own side
//! and `handshaken` connects afresh; a server has refused the fixture's own key about one dial in six thousand and
//! the dial connects afresh: CI and host runs of the `idle_loss` rows saw the proxy count one connection more than
//! the sessions dialed, and a dial fail. `DialRetries` makes either the case of every dial, which a Unix host never
//! produces by itself.
use super::{DialRetries, SshdFixture, dialer_counting, handshaken_counting};
use crate::git::endpoint::cut_proxy::CutProxy;

#[test]
fn a_key_exchange_that_fails_is_a_connection_and_not_a_session() {
    let fixture = SshdFixture::new();
    let proxy = CutProxy::start(fixture.port);
    let retries = DialRetries::failing_key_exchanges();
    handshaken_counting(proxy.port, 5_000, &retries, true).unwrap();
    assert_eq!(proxy.connections(), 2);
    assert_eq!(retries.sessions_in(proxy.connections()), 1);
    handshaken_counting(proxy.port, 5_000, &retries, true).unwrap();
    assert_eq!(proxy.connections(), 4);
    assert_eq!(retries.sessions_in(proxy.connections()), 2);
}

#[test]
fn a_dial_that_works_abandons_no_connection() {
    let fixture = SshdFixture::new();
    let proxy = CutProxy::start(fixture.port);
    let retries = DialRetries::default();
    let dial = fixture.dialer_counting(proxy.port, retries.clone());
    dial().unwrap();
    assert_eq!(proxy.connections(), 1);
    assert_eq!(retries.sessions_in(proxy.connections()), 1);
}

#[test]
fn a_dial_whose_key_exchange_fails_connects_again() {
    let fixture = SshdFixture::new();
    let proxy = CutProxy::start(fixture.port);
    let retries = DialRetries::failing_key_exchanges();
    let dial = fixture.dialer_counting(proxy.port, retries.clone());
    dial().unwrap();
    assert_eq!(proxy.connections(), 2);
    assert_eq!(retries.sessions_in(proxy.connections()), 1);
}

#[test]
fn a_dial_whose_key_the_server_refuses_connects_again() {
    let fixture = SshdFixture::new();
    let proxy = CutProxy::start(fixture.port);
    let retries = DialRetries::refusing_keys();
    let dial = fixture.dialer_counting(proxy.port, retries.clone());
    dial().unwrap();
    assert_eq!(proxy.connections(), 2);
    assert_eq!(retries.sessions_in(proxy.connections()), 1);
    dial().unwrap();
    assert_eq!(proxy.connections(), 4);
    assert_eq!(retries.sessions_in(proxy.connections()), 2);
}

#[test]
fn a_dial_that_is_retried_for_its_key_exchange_and_for_its_key_is_three_connections_and_one_session()
 {
    let fixture = SshdFixture::new();
    let proxy = CutProxy::start(fixture.port);
    let retries = DialRetries::failing_key_exchanges().and_refusing_keys();
    let dial = fixture.dialer_counting(proxy.port, retries.clone());
    dial().unwrap();
    assert_eq!(proxy.connections(), 3);
    assert_eq!(retries.sessions_in(proxy.connections()), 1);
}

#[test]
fn a_key_the_server_does_not_authorize_fails_the_dial_at_its_first_connection() {
    let fixture = SshdFixture::new();
    let proxy = CutProxy::start(fixture.port);
    let retries = DialRetries::default();
    // The fixture's server does not authorize this key.
    let unauthorized = fixture.temp.path().join("unauthorized_ed25519");
    super::run_fixture_key(&unauthorized);
    let dial = dialer_counting(
        proxy.port,
        fixture.port,
        fixture.user.clone(),
        fixture.known_hosts.clone(),
        unauthorized,
        retries.clone(),
    );
    assert!(dial().is_err());
    // A refusal that the fixture's own key never gets is the server's answer: it is final at the first connection.
    assert_eq!(proxy.connections(), 1);
}
