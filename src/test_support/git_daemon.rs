//! A `git daemon` that a test owns, and that dies with the test.
//!
//! `git daemon` is a wrapper: the `git` the test starts runs `git-daemon` as a
//! child and waits for it. Killing the wrapper leaves that child listening
//! with its parent gone, and every test that did so left one behind (672 of
//! them, after 41 days on one machine). The guard starts the `git-daemon`
//! program itself, so the process it kills is the one that listens.
//!
//! The daemon cannot take a listener that is already bound, so the port is
//! chosen by binding and releasing one (from the range `fixture_port` draws
//! from, which the system gives to nothing else), and in the gap between that
//! release and the daemon's own bind another test's daemon can still take the
//! port. The daemon binds exclusively, which is why it is not given
//! `--reuseaddr`: on Windows that option lets a second daemon bind a port that
//! a first one listens on, and both go on running. The daemon then exits, and a check that something accepts
//! connections, or that it answers as a `git-daemon` does, would have gone on
//! to talk to whatever took the port while the daemon was still starting.
//! [`GitDaemon::start`] therefore counts the daemon as ready only when it is
//! still running and its own log says it listens (`Ready to rumble`, which
//! the daemon writes after it has bound its sockets, and so only its own
//! process can), and otherwise tries again on a new port.
//!
//! Clients do not connect to the daemon's port but to a relay's (`port`;
//! `daemon_port` is the daemon's own): Git for Windows' daemon ends every
//! session with a reset, which costs a client that is slow to read the end of
//! the pack (see `git_daemon_relay`). The relay is the guard's, bound before
//! `start` returns, and stops with the daemon.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::git_daemon_relay::Relay;
use super::{TempDir, fixture_port};

const ATTEMPTS: usize = 5;
const READY_WITHIN: Duration = Duration::from_secs(5);

/// A running `git-daemon` on a loopback port, killed and reaped on drop.
pub(crate) struct GitDaemon {
    child: Child,
    /// The port the daemon itself listens on.
    daemon_port: u16,
    /// The port a client connects to: the relay's, once the daemon is ready.
    port: u16,
    /// What stands between a client and the daemon (see `git_daemon_relay`), dropped when the daemon is stopped.
    relay: Option<Relay>,
    /// Where the daemon's log is, and the directory that holds it until the guard is dropped.
    log: PathBuf,
    _logs: TempDir,
}

impl GitDaemon {
    /// Serve every repository under `base_path`, with `receive-pack` enabled
    /// so a test can push to it, and return once it is ready.
    pub(crate) fn start(base_path: &Path) -> Self {
        Self::start_on_ports(base_path, fixture_port)
    }

    /// [`Self::start`] on the ports `next_port` names, one for each attempt; a seam for a test that makes two
    /// daemons meet on one port.
    pub(crate) fn start_on_ports(base_path: &Path, mut next_port: impl FnMut() -> u16) -> Self {
        let program = git_daemon_program();
        for _ in 0..ATTEMPTS {
            let port = next_port();
            let logs = TempDir::new("git-daemon-log");
            let log = logs.path().join("daemon.log");
            let child = Command::new(&program)
                // The daemon logs `Ready to rumble` only when it is told to log at all.
                .arg("--verbose")
                .arg("--export-all")
                .arg("--enable=receive-pack")
                .arg("--listen=127.0.0.1")
                .arg(format!("--port={port}"))
                .arg(format!("--base-path={}", base_path.display()))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(File::create(&log).expect("the daemon's log is created"))
                .spawn()
                .expect("git-daemon starts");
            // From here the guard owns the child, whatever happens next.
            let mut daemon = Self {
                child,
                daemon_port: port,
                port,
                relay: None,
                log,
                _logs: logs,
            };
            if daemon.wait_until_ready() {
                let relay = Relay::start(port);
                daemon.port = relay.port();
                daemon.relay = Some(relay);
                return daemon;
            }
        }
        panic!("git-daemon did not become ready in {ATTEMPTS} attempts");
    }

    /// The port to connect to.
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// The port the daemon itself listens on, behind the relay that `port` names.
    pub(crate) fn daemon_port(&self) -> u16 {
        self.daemon_port
    }

    /// What the daemon has written to its log so far.
    pub(crate) fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Whether the daemon's process is still running.
    pub(crate) fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    /// Kill the daemon and wait for it, and stop the relay; both ports are closed on return.
    pub(crate) fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.relay = None;
    }

    /// Ready: our child is still running and its log says it listens. False
    /// if the child exited (it could not bind), whatever else answers on the
    /// port; nothing listens for us then.
    fn wait_until_ready(&mut self) -> bool {
        let deadline = Instant::now() + READY_WITHIN;
        while Instant::now() < deadline {
            if self.child.try_wait().unwrap().is_some() {
                return false;
            }
            if fs::read_to_string(&self.log).is_ok_and(|log| log.contains("Ready to rumble")) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }
}

impl Drop for GitDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let relay = self.relay.take();
        // A test that fails while it holds a daemon has nothing else to say what the daemon saw.
        if std::thread::panicking() {
            eprintln!(
                "git-daemon on port {} wrote:\n{}",
                self.daemon_port,
                self.log()
            );
            if let Some(relay) = relay {
                eprintln!("the relay in front of it carried:\n{}", relay.finish());
            }
        }
    }
}

/// How many times [`retry_a_lost_tail`] tries.
const CLONE_ATTEMPTS: usize = 3;

/// What libgit2 says when the end of a session was lost: the read that met the reset, or, through the relay, the
/// session ending before its last packet.
const LOST_TAIL: [&str; 2] = [
    "forcibly closed by the remote host",
    "could not read from remote repository",
];

/// Run `attempt` (a clone from a `GitDaemon`), and run it again, up to [`CLONE_ATTEMPTS`] times in all, when it
/// failed the way a client fails that the daemon's reset caught before it had read the end of the session (see
/// `git_daemon_relay`: the relay makes that rare, about 1 clone in 2,500, and cannot make it impossible, because
/// the reset can land between two reads of the relay's own). Any other failure is final, and so is the last
/// attempt's. Each failed attempt is printed.
pub(crate) fn retry_a_lost_tail<T, E: std::fmt::Debug>(
    mut attempt: impl FnMut() -> Result<T, E>,
) -> Result<T, E> {
    let mut tried = 1;
    loop {
        let error = match attempt() {
            Ok(done) => {
                return Ok(done);
            }
            Err(error) => error,
        };
        let described = format!("{error:?}");
        if tried == CLONE_ATTEMPTS || !LOST_TAIL.iter().any(|text| described.contains(text)) {
            return Err(error);
        }
        eprintln!("attempt {tried} lost the end of the session, trying again: {described}");
        tried += 1;
    }
}

/// The `git-daemon` program in Git's exec path, which is where `git daemon`
/// would have found it.
fn git_daemon_program() -> PathBuf {
    let output = Command::new("git")
        .arg("--exec-path")
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git --exec-path failed");
    let exec_path = String::from_utf8(output.stdout).expect("git's exec path is UTF-8");
    Path::new(exec_path.trim()).join(format!("git-daemon{}", std::env::consts::EXE_SUFFIX))
}
