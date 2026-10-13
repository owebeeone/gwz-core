//! The guard's own promise: when it is dropped, nothing it started is still
//! listening on its port.
//!
//! Before it started `git-daemon` itself, dropping it killed only the `git`
//! wrapper, and the real daemon went on listening as an orphan.

use std::net::TcpStream;

use super::{GitDaemon, TempDir};

/// The ports to try: `taken` for `times` attempts, as when another daemon won the race for it, then fresh ones.
fn after(taken: u16, times: usize) -> impl FnMut() -> u16 {
    let mut left = times;
    move || {
        if left > 0 {
            left -= 1;
            return taken;
        }
        super::fixture_port()
    }
}

#[test]
fn dropping_the_guard_leaves_nothing_listening_on_its_port() {
    let temp = TempDir::new("git-daemon-guard");
    let daemon = GitDaemon::start(temp.path());
    let port = daemon.port();
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_ok(),
        "the daemon listens while the guard is held"
    );
    drop(daemon);
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "a daemon outlived its guard on port {port}"
    );
}

#[test]
fn stopping_the_guard_closes_the_port_and_dropping_it_after_is_harmless() {
    let temp = TempDir::new("git-daemon-stop");
    let mut daemon = GitDaemon::start(temp.path());
    let port = daemon.port();
    daemon.stop();
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    drop(daemon);
}

/// A daemon on a port that another daemon already serves exits at once, and a readiness check that went by what
/// answers would take the other daemon's answer for its own.
#[test]
fn a_daemon_whose_port_another_daemon_took_moves_to_a_port_of_its_own() {
    let temp = TempDir::new("git-daemon-collision");
    let first = GitDaemon::start(temp.path());
    let mut second = GitDaemon::start_on_ports(temp.path(), after(first.port(), 1));
    assert_ne!(second.port(), first.port(), "both daemons claim one port");
    assert!(second.running(), "the second daemon is not running");
}

#[test]
#[should_panic(expected = "git-daemon did not become ready")]
fn a_daemon_that_finds_no_port_to_serve_on_says_so() {
    let temp = TempDir::new("git-daemon-no-port");
    let first = GitDaemon::start(temp.path());
    let _second = GitDaemon::start_on_ports(temp.path(), after(first.port(), usize::MAX));
}
