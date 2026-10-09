//! The guard's own promise: when it is dropped, nothing it started is still
//! listening on its port.
//!
//! Before it started `git-daemon` itself, dropping it killed only the `git`
//! wrapper, and the real daemon went on listening as an orphan.

use std::net::TcpStream;

use super::{GitDaemon, TempDir};

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

/// The port the guard picks can be taken before the daemon binds it. The daemon
/// exits then, and a listener that merely accepts connections would pass for
/// it: the readiness check must see that no git daemon is answering.
#[test]
fn a_listener_that_is_not_a_git_daemon_is_not_ready() {
    use std::net::TcpListener;

    let foreign = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = foreign.local_addr().unwrap().port();
    let accepting = std::thread::spawn(move || {
        // Accepts, reads, never answers: what another test's server does.
        let (stream, _) = foreign.accept().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(3));
        drop(stream);
    });
    assert!(!super::git_daemon::answers_as_git_daemon(port));
    accepting.join().unwrap();
}
