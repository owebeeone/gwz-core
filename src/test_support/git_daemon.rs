//! A `git daemon` that a test owns, and that dies with the test.
//!
//! `git daemon` is a wrapper: the `git` the test starts runs `git-daemon` as a
//! child and waits for it. Killing the wrapper leaves that child listening
//! with its parent gone, and every test that did so left one behind (672 of
//! them, after 41 days on one machine). The guard starts the `git-daemon`
//! program itself, so the process it kills is the one that listens.
//!
//! The daemon cannot take a listener that is already bound, so the port is
//! chosen by binding and releasing one, and in the gap between that release
//! and the daemon's own bind another test can take the port. The daemon then
//! exits, and a bare "something accepts connections" check would have gone on
//! to talk to whatever took the port. [`GitDaemon::start`] therefore counts the
//! daemon as ready only when it is still running and the listener answers as
//! `git-daemon` does, and otherwise tries again on a new port.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ATTEMPTS: usize = 5;
const READY_WITHIN: Duration = Duration::from_secs(5);

/// A running `git-daemon` on a loopback port, killed and reaped on drop.
pub(crate) struct GitDaemon {
    child: Child,
    port: u16,
}

impl GitDaemon {
    /// Serve every repository under `base_path`, with `receive-pack` enabled
    /// so a test can push to it, and return once it is ready.
    pub(crate) fn start(base_path: &Path) -> Self {
        let program = git_daemon_program();
        for _ in 0..ATTEMPTS {
            let port = free_port();
            let child = Command::new(&program)
                .arg("--reuseaddr")
                .arg("--export-all")
                .arg("--enable=receive-pack")
                .arg("--listen=127.0.0.1")
                .arg(format!("--port={port}"))
                .arg(format!("--base-path={}", base_path.display()))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("git-daemon starts");
            // From here the guard owns the child, whatever happens next.
            let mut daemon = Self { child, port };
            if daemon.wait_until_ready() {
                return daemon;
            }
        }
        panic!("git-daemon did not become ready in {ATTEMPTS} attempts");
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// Kill the daemon and wait for it; the port is closed on return.
    pub(crate) fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Ready: our child is still running, and the listener on the port answers
    /// a request for a repository that does not exist with `git-daemon`'s
    /// refusal. False if the child exited (it could not bind) or the listener
    /// is not a git daemon; nothing listens for us then.
    fn wait_until_ready(&mut self) -> bool {
        let deadline = Instant::now() + READY_WITHIN;
        while Instant::now() < deadline {
            if self.child.try_wait().unwrap().is_some() {
                return false;
            }
            if answers_as_git_daemon(self.port) && self.child.try_wait().unwrap().is_none() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }
}

impl Drop for GitDaemon {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Ask for a repository that is not there. `git-daemon` answers with a
/// pkt-line `ERR ...`; a connection that is refused, closed, silent or
/// answered otherwise is not it.
pub(super) fn answers_as_git_daemon(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let timeout = Some(Duration::from_secs(2));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return false;
    }
    let request = b"git-upload-pack /gwz-readiness-probe\0host=localhost\0";
    let line = format!("{:04x}", request.len() + 4);
    if stream.write_all(line.as_bytes()).is_err() || stream.write_all(request).is_err() {
        return false;
    }
    let mut reply = [0u8; 8];
    stream.read_exact(&mut reply).is_ok() && &reply[4..7] == b"ERR"
}

fn free_port() -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    probe.local_addr().unwrap().port()
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
