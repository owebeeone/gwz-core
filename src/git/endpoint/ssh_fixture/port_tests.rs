//! A fixture server is ready only when it is its own server that answers.
//!
//! The fixture picks its port by binding and releasing one, so another test's server can take the port before this
//! one binds it. The second `sshd` then exits, and a probe that only connects would go on to talk to the first one,
//! whose host key the second fixture's `known_hosts` does not trust (CI run 38427769224, `temporary host key did not
//! match`). These rows put two fixtures on one port on purpose, through the constructors that name the ports to try.
use super::SshdFixture;
use crate::test_support::fixture_port;
use std::net::TcpListener;

/// The ports to try: `taken` first, as when another server won the race for it, then fresh ones.
fn after(taken: u16, times: usize) -> impl FnMut() -> u16 {
    let mut left = times;
    move || {
        if left > 0 {
            left -= 1;
            taken
        } else {
            fixture_port()
        }
    }
}

#[test]
fn a_fixture_whose_port_another_server_took_moves_to_a_port_of_its_own() {
    let mut first = SshdFixture::new();
    let mut second = SshdFixture::new_on_ports(after(first.port, 1));
    assert_ne!(second.port, first.port, "both fixtures claim one port");
    assert!(
        second.child.try_wait().unwrap().is_none(),
        "the second fixture's server is not running"
    );
    // Each fixture's session meets its own host key.
    drop(second.session());
    drop(first.session());
    assert_eq!(second.authenticated_sessions, 1);
}

#[test]
fn a_debug_fixture_whose_port_another_server_took_moves_to_a_port_of_its_own() {
    // A debugging server serves one connection, so the session's own handshake tells whether it is the fixture's own.
    let first = SshdFixture::new();
    let mut second = SshdFixture::new_debug_on_ports(after(first.port, 1));
    drop(second.debug_session());
    assert_ne!(second.port, first.port, "both fixtures claim one port");
    assert_eq!(second.authenticated_sessions, 1);
}

#[test]
fn a_port_held_by_something_that_is_not_sshd_is_not_taken_for_the_server() {
    let holder = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let taken = holder.local_addr().unwrap().port();
    let mut fixture = SshdFixture::new_on_ports(after(taken, 2));
    assert_ne!(fixture.port, taken);
    drop(fixture.session());
}

#[test]
#[should_panic(expected = "temporary sshd did not start")]
fn a_fixture_that_finds_no_port_to_serve_on_says_so() {
    let first = SshdFixture::new();
    let _second = SshdFixture::new_on_ports(after(first.port, usize::MAX));
}
