//! A small SSH server a test controls, for the rows that need a channel whose output ends while the server's session
//! lives (the stuck-close rows of `ssh_tests::background_close` and `ssh_tests::channel`).
//!
//! A system `sshd` cannot serve them everywhere: Windows' `sshd.exe` 9.5p2 sends a channel's EOF only when the session
//! process exits, whatever has happened to the pipes (run `2026-10-11-winp1c`, `raw/stuck-close-experiments`), so no
//! process under it can end the output and keep the channel open. This server is test code only, [russh] in the
//! candidate manifest's dev-dependencies (`tests/transport_backend/prepare.py`), and the same on every platform. It
//! serves what those rows use and nothing more: the fixture's host key, public-key authentication for the fixture's
//! one client key and user, and exec requests for `git-upload-pack` and `git-receive-pack`, which it runs with Git.
//! When the service is over, [`Ending`] says whether the server then closes the channel as `sshd` does, or sends the
//! EOF and nothing else, as a server whose session process is still running. (russh answers a client's
//! CHANNEL_CLOSE at once and offers no way to hold the answer back, so a stuck server stops reading its connection
//! when the client's EOF arrives; see [`Ending::Stuck`].)
//!
//! The server lives in a thread of its own with a current-thread runtime, and stops (ending the services it started)
//! when the [`ControlledSshd`] drops. Its files are [`Layout`]'s, the same as the system `sshd` fixture's.
//!
//! [russh]: https://crates.io/crates/russh
use super::{
    ssh_connection::SshConnection,
    ssh_fixture::{self as common, Layout, Reach},
};
use russh::{
    MethodKind, Preferred,
    keys::{Algorithm, HashAlg, PublicKey},
    server::{Config, run_stream},
};
use std::{
    borrow::Cow,
    fs, io,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    thread::{self, JoinHandle},
    time::Duration,
};
use tempfile::TempDir;
use tokio::sync::oneshot;

mod command;
mod handler;
mod holdable;
mod host_key;
mod tests;

/// What the server does when the Git service a channel runs has exited and its output has ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Ending {
    /// The channel ends as `sshd`'s does: EOF, then the service's exit status, then the close.
    Prompt,
    /// The EOF is sent, and the exit status and the close never are: the server's session process has not exited.
    /// The server reads nothing more from the connection once the client's EOF arrives, so the CHANNEL_CLOSE a
    /// client sends after its EOF (libssh2 sends the EOF first, always) is never read, and never answered.
    Stuck,
}

/// A running controlled server and the files a client of it needs.
pub(crate) struct ControlledSshd {
    pub(crate) temp: TempDir,
    pub(crate) port: u16,
    pub(crate) user: String,
    pub(crate) known_hosts: PathBuf,
    pub(crate) repository: PathBuf,
    pub(crate) marker: PathBuf,
    pub(crate) authenticated_sessions: usize,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

/// The algorithms the server offers. Its default lists leave out `ssh-rsa`, and the fixture's libssh2 (CNG on
/// Windows) may negotiate nothing else for an RSA key, so every RSA form is offered; the other lists are russh's.
fn preferred() -> Preferred {
    Preferred {
        key: Cow::Owned(vec![
            Algorithm::Ed25519,
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha512),
            },
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha256),
            },
            Algorithm::Rsa { hash: None },
        ]),
        ..Preferred::default()
    }
}

impl ControlledSshd {
    /// A server in a new temporary directory, ready to accept connections, that ends its channels as `ending` says.
    pub(crate) fn start(ending: Ending) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let layout = Layout::new(port);
        let host_key = host_key::load(&layout.host_key).expect("the fixture's host key file loads");
        let authorized = fs::read_to_string(&layout.authorized).unwrap();
        let authorized = PublicKey::from_openssh(authorized.trim())
            .expect("the fixture's client key is a public key");
        let config = Config {
            methods: [MethodKind::PublicKey].as_slice().into(),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key],
            preferred: preferred(),
            inactivity_timeout: Some(Duration::from_secs(120)),
            ..Config::default()
        };
        let config = Arc::new(config);
        let user = layout.user.clone();
        let (stop, stopped) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("controlled-sshd".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                    let accepting = async {
                        while let Ok((socket, _)) = listener.accept().await {
                            let hold = Arc::new(AtomicBool::new(false));
                            let stream = holdable::Holdable::new(socket, hold.clone());
                            let client = handler::Client::new(
                                user.clone(),
                                authorized.clone(),
                                ending,
                                hold,
                            );
                            let config = config.clone();
                            tokio::spawn(async move {
                                if let Ok(session) = run_stream(config, stream, client).await {
                                    let _ = session.await;
                                }
                            });
                        }
                    };
                    tokio::select! {
                        _ = accepting => {}
                        _ = stopped => {}
                    }
                });
                // The connections and services still running are dropped with their tasks, and the services killed.
                runtime.shutdown_timeout(Duration::from_secs(2));
            })
            .unwrap();
        Self {
            temp: layout.temp,
            port,
            user: layout.user,
            known_hosts: layout.known_hosts,
            repository: layout.repository,
            marker: layout.marker,
            authenticated_sessions: 0,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    /// An authenticated, nonblocking session; panics if the setup fails.
    pub(crate) fn session(&mut self) -> SshConnection {
        match self.try_session() {
            Ok(connection) => connection,
            Err(error) => panic!("controlled SSH session setup failed: {error}"),
        }
    }

    pub(crate) fn try_session(&mut self) -> io::Result<SshConnection> {
        let connection = common::dialer(
            self.port,
            self.port,
            self.user.clone(),
            self.known_hosts.clone(),
            self.client_key(),
        )()?;
        self.authenticated_sessions += 1;
        Ok(connection)
    }
}

impl Drop for ControlledSshd {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Reach for ControlledSshd {
    fn port(&self) -> u16 {
        self.port
    }

    fn user(&self) -> &str {
        &self.user
    }

    fn known_hosts(&self) -> &Path {
        &self.known_hosts
    }

    fn client_key(&self) -> PathBuf {
        self.temp.path().join("client_ed25519")
    }

    fn repository(&self) -> &Path {
        &self.repository
    }
}
