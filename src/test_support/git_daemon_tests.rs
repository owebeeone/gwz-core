//! The guard's own promise: when it is dropped, nothing it started is still
//! listening on its port.
//!
//! Before it started `git-daemon` itself, dropping it killed only the `git`
//! wrapper, and the real daemon went on listening as an orphan.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use std::path::Path;

use super::{GitDaemon, TempDir, retry_a_lost_tail};

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
    let daemon_port = daemon.daemon_port();
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_ok(),
        "the daemon listens while the guard is held"
    );
    drop(daemon);
    assert!(
        TcpStream::connect(("127.0.0.1", daemon_port)).is_err(),
        "a daemon outlived its guard on port {daemon_port}"
    );
}

#[test]
fn stopping_the_guard_closes_the_port_and_dropping_it_after_is_harmless() {
    let temp = TempDir::new("git-daemon-stop");
    let mut daemon = GitDaemon::start(temp.path());
    let daemon_port = daemon.daemon_port();
    daemon.stop();
    assert!(TcpStream::connect(("127.0.0.1", daemon_port)).is_err());
    drop(daemon);
}

/// A daemon on a port that another daemon already serves exits at once, and a readiness check that went by what
/// answers would take the other daemon's answer for its own.
#[test]
fn a_daemon_whose_port_another_daemon_took_moves_to_a_port_of_its_own() {
    let temp = TempDir::new("git-daemon-collision");
    let first = GitDaemon::start(temp.path());
    let mut second = GitDaemon::start_on_ports(temp.path(), after(first.daemon_port(), 1));
    assert_ne!(
        second.daemon_port(),
        first.daemon_port(),
        "both daemons claim one port"
    );
    assert!(second.running(), "the second daemon is not running");
}

#[test]
#[should_panic(expected = "git-daemon did not become ready")]
fn a_daemon_that_finds_no_port_to_serve_on_says_so() {
    let temp = TempDir::new("git-daemon-no-port");
    let first = GitDaemon::start(temp.path());
    let _second = GitDaemon::start_on_ports(temp.path(), after(first.daemon_port(), usize::MAX));
}

/// A bare repository `repo` with one commit under `base`.
fn repository_under(base: &Path) {
    let repository = git2::Repository::init_bare(base.join("repo")).unwrap();
    let blob = repository.blob(b"payload\n").unwrap();
    let mut tree = repository.treebuilder(None).unwrap();
    tree.insert("payload", blob, 0o100644).unwrap();
    let tree = repository.find_tree(tree.write().unwrap()).unwrap();
    let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    repository
        .commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            "fixture",
            &tree,
            &[],
        )
        .unwrap();
    repository.set_head("refs/heads/main").unwrap();
}

fn packet(payload: &[u8]) -> Vec<u8> {
    let mut line = format!("{:04x}", payload.len() + 4).into_bytes();
    line.extend_from_slice(payload);
    line
}

/// One pkt-line, read after `pause`: its payload, or None for a flush.
fn read_packet(stream: &mut TcpStream, pause: Duration) -> std::io::Result<Option<Vec<u8>>> {
    std::thread::sleep(pause);
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
    if length == 0 {
        return Ok(None);
    }
    let mut payload = vec![0u8; length - 4];
    stream.read_exact(&mut payload)?;
    Ok(Some(payload))
}

/// A clone over the git protocol as libgit2 makes it (version 0, side-band-64k, no haves), reading each packet
/// after `pause` the way a client does that has work to do between reads. Returns the bytes of the pack.
fn clone_conversation(port: u16, pause: Duration) -> std::io::Result<usize> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    stream.write_all(&packet(
        format!("git-upload-pack /repo\0host=127.0.0.1:{port}\0").as_bytes(),
    ))?;
    let mut first = None;
    while let Some(line) = read_packet(&mut stream, Duration::ZERO)? {
        first.get_or_insert(line);
    }
    let first = first.expect("the repository advertises a ref");
    let want = format!(
        "want {} multi_ack_detailed side-band-64k ofs-delta\n",
        std::str::from_utf8(&first[..40]).unwrap()
    );
    stream.write_all(&packet(want.as_bytes()))?;
    stream.write_all(b"0000")?;
    stream.write_all(&packet(b"done\n"))?;
    read_packet(&mut stream, pause)?; // NAK
    let mut pack = 0;
    while let Some(line) = read_packet(&mut stream, pause)? {
        if line.first() == Some(&1) {
            pack += line.len() - 1;
        }
    }
    Ok(pack)
}

/// Git for Windows' `git-daemon` ends every session with a reset, and Windows throws away what a reset finds
/// unread, so a client that is busy between its reads loses the end of the pack (`WSAECONNRESET`: "error
/// receiving data from socket"). The daemon of a test that clones with libgit2 is that client's server.
#[test]
fn a_client_that_is_slow_between_reads_still_gets_the_whole_pack() {
    let temp = TempDir::new("git-daemon-late-reader");
    repository_under(temp.path());
    let daemon = GitDaemon::start(temp.path());

    let pack = clone_conversation(daemon.port(), Duration::from_millis(100))
        .expect("a slow reader is served to the end");

    assert!(pack > 0);
}

const RESET: &str = "error receiving data from socket: An existing connection was forcibly closed by the remote host.";
const CUT: &str = "could not read from remote repository";

#[test]
fn a_clone_that_lost_the_end_of_the_session_is_tried_again() {
    for message in [RESET, CUT] {
        let mut failures = vec![message, message].into_iter();
        let mut attempts = 0;
        let result = retry_a_lost_tail(|| {
            attempts += 1;
            match failures.next() {
                Some(message) => Err(format!("ModelError {{ message: {message:?} }}")),
                None => Ok(attempts),
            }
        });
        assert_eq!(result, Ok(3), "{message}");
    }
}

#[test]
fn any_other_failure_is_final() {
    let mut attempts = 0;
    let result: Result<(), String> = retry_a_lost_tail(|| {
        attempts += 1;
        Err("the repository does not exist".to_owned())
    });
    assert!(result.is_err());
    assert_eq!(attempts, 1);
}

#[test]
fn a_clone_that_keeps_losing_the_end_gives_up_after_three_attempts_with_the_last_error() {
    let mut attempts = 0;
    let result: Result<(), String> = retry_a_lost_tail(|| {
        attempts += 1;
        Err(format!("{CUT} (attempt {attempts})"))
    });
    assert_eq!(attempts, 3);
    assert_eq!(result, Err(format!("{CUT} (attempt 3)")));
}
