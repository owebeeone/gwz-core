//! One client's session on the controlled server: public-key authentication against the one key the fixture trusts,
//! and an exec channel that runs the Git service the client asked for. What the server does when the service is over
//! is the test's choice ([`Ending`]).
use super::{Ending, command};
use russh::{
    Channel, ChannelId,
    keys::PublicKey,
    server::{Auth, Handler, Msg, Session},
};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, Command},
};

/// The extended-data type of a channel's standard error (RFC 4254, section 5.2).
const STDERR: u32 = 1;

pub(super) struct Client {
    pub(super) user: String,
    pub(super) authorized: PublicKey,
    pub(super) ending: Ending,
    /// Stops the connection's reads (see [`Ending::Stuck`]).
    hold: Arc<AtomicBool>,
    /// The standard input of each channel's service, until the client's EOF.
    stdin: HashMap<ChannelId, ChildStdin>,
}

impl Client {
    pub(super) fn new(
        user: String,
        authorized: PublicKey,
        ending: Ending,
        hold: Arc<AtomicBool>,
    ) -> Self {
        Self {
            user,
            authorized,
            ending,
            hold,
            stdin: HashMap::new(),
        }
    }

    fn accepts(&self, user: &str, key: &PublicKey) -> Auth {
        if user == self.user && key.key_data() == self.authorized.key_data() {
            Auth::Accept
        } else {
            Auth::reject()
        }
    }
}

impl Handler for Client {
    type Error = russh::Error;

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(self.accepts(user, key))
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        Ok(self.accepts(user, key))
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(request) = command::parse(data) else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        // The service is started directly, with the path as one argument: no shell reads the client's text.
        let spawned = Command::new("git")
            .arg(request.service)
            .arg("--")
            .arg(native_path(&request.path))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(_) => {
                session.channel_failure(channel)?;
                return Ok(());
            }
        };
        session.channel_success(channel)?;
        if let Some(stdin) = child.stdin.take() {
            self.stdin.insert(channel, stdin);
        }
        let handle = session.handle();
        let ending = self.ending;
        tokio::spawn(async move {
            let mut stdout = child.stdout.take().expect("piped");
            let mut stderr = child.stderr.take().expect("piped");
            let output = forward(&mut stdout, |bytes| handle.data(channel, bytes));
            let diagnostics = forward(&mut stderr, |bytes| {
                handle.extended_data(channel, STDERR, bytes)
            });
            tokio::join!(output, diagnostics);
            let status = child
                .wait()
                .await
                .ok()
                .and_then(|status| status.code())
                .unwrap_or(255);
            // The service is over and its output has ended. Whether the channel ends with it is the test's call.
            let _ = handle.eof(channel).await;
            match ending {
                Ending::Prompt => {
                    let _ = handle.exit_status_request(channel, status as u32).await;
                    let _ = handle.close(channel).await;
                }
                Ending::Late(delay) => {
                    tokio::time::sleep(delay).await;
                    let _ = handle.exit_status_request(channel, status as u32).await;
                    let _ = handle.close(channel).await;
                }
                Ending::Stuck => {}
            }
        });
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(stdin) = self.stdin.get_mut(&channel) {
            // A service that has exited takes no more input.
            if stdin.write_all(data).await.is_err() {
                self.stdin.remove(&channel);
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stdin.remove(&channel);
        if self.ending == Ending::Stuck {
            // The client has sent its whole request. From here the connection is not read: the CHANNEL_CLOSE that
            // follows the client's EOF stays unread, so russh never answers it, and the service's output (which the
            // session task still sends) is all the client gets.
            self.hold.store(true, Ordering::Release);
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stdin.remove(&channel);
        Ok(())
    }
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// The path a native Git takes for the path a client named: a URL carries a drive path in its POSIX form
        /// (`/c/Users/x`, the fixtures' `server_path`), which a native Windows Git does not read.
        fn native_path(path: &str) -> String {
            match path.as_bytes() {
                [b'/', letter, b'/', ..] if letter.is_ascii_alphabetic() => {
                    format!("{}:{}", *letter as char, &path[2..])
                }
                _ => path.to_owned(),
            }
        }
    } else {
        /// The path a native Git takes for the path a client named: the same.
        fn native_path(path: &str) -> String {
            path.to_owned()
        }
    }
}

/// Reads `from` to its end, handing each piece to `send`; stops early when the client is gone.
async fn forward<R, F, Fut>(from: &mut R, send: F)
where
    R: AsyncRead + Unpin,
    F: Fn(Vec<u8>) -> Fut,
    Fut: Future<Output = Result<(), bytes::Bytes>>,
{
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        match from.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(count) => {
                if send(buffer[..count].to_vec()).await.is_err() {
                    return;
                }
            }
        }
    }
}
