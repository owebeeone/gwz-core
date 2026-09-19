//! Single-owner, nonblocking SSH Git channel. Connection, trust/authentication,
//! readiness, deadlines and pool disposition are responsibilities of the host.
use super::ssh_connection::SshConnection;
use ssh2::{BlockDirections, Channel};
use std::io::{self, Read, Write};

#[derive(Clone, Copy, Debug)]
pub enum GitService {
    UploadPack,
    ReceivePack,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Open,
    Exec,
    Active,
    Close,
    WaitClose,
    Finished,
    Failed,
    Disposed,
}

/// Takes sole ownership: no other Session or Channel clone may drive this
/// connection. The caller must have checked host trust BEFORE authenticating.
/// No constructor here asserts host trust or establishes endpoint identity.
pub struct SshChannel {
    // Drop channel before the last session owner.
    channel: Option<Channel>,
    session: Option<SshConnection>,
    command: String,
    phase: Phase,
    sent_eof: bool,
    stdout_eof: bool,
    stderr_eof: bool,
    exit_status: Option<i32>,
}
impl SshChannel {
    pub fn new(session: SshConnection, service: GitService, path: &str) -> io::Result<Self> {
        if !session.native().authenticated() || session.native().is_blocking() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "requires authenticated nonblocking session",
            ));
        }
        if path.is_empty() || path.len() > 16_384 || path.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid Git repository operand",
            ));
        }
        let executable = match service {
            GitService::UploadPack => "git-upload-pack",
            GitService::ReceivePack => "git-receive-pack",
        };
        let command = format!("{executable} -- '{}'", path.replace('\'', "'\\''"));
        Ok(Self {
            channel: None,
            session: Some(session),
            command,
            phase: Phase::Open,
            sent_eof: false,
            stdout_eof: false,
            stderr_eof: false,
            exit_status: None,
        })
    }

    /// Native readiness interest after WouldBlock; the host owns socket waits.
    pub fn block_directions(&self) -> BlockDirections {
        self.session
            .as_ref()
            .map_or(BlockDirections::None, |owner| {
                owner.native().block_directions()
            })
    }

    pub fn poll_open(&mut self) -> io::Result<()> {
        if self.phase == Phase::Open {
            let result = self
                .session
                .as_ref()
                .expect("owned session")
                .native()
                .channel_session();
            self.channel = Some(self.native(result)?);
            self.phase = Phase::Exec;
        }
        if self.phase == Phase::Exec {
            let result = self
                .channel
                .as_mut()
                .expect("opened channel")
                .exec(&self.command);
            self.native(result)?;
            self.phase = Phase::Active;
        }
        self.active()
    }

    /// Drain separately into an explicitly bounded diagnostic sink.
    pub fn read_stderr(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.active()?;
        if output.is_empty() {
            return Ok(0);
        }
        let result = self
            .channel
            .as_ref()
            .expect("active channel")
            .stderr()
            .read(output);
        let count = self.io(result)?;
        self.stderr_eof |= count == 0;
        Ok(count)
    }

    pub fn send_eof(&mut self) -> io::Result<()> {
        self.active()?;
        if !self.sent_eof {
            let result = self.channel.as_mut().expect("active channel").send_eof();
            self.native(result)?;
            self.sent_eof = true;
        }
        Ok(())
    }

    /// Complete SSH cleanup. The returned status is not a Git success verdict.
    pub fn finish(&mut self) -> io::Result<i32> {
        if self.phase == Phase::Active {
            if !self.sent_eof || !self.stdout_eof || !self.stderr_eof {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            self.phase = Phase::Close;
        }
        if self.phase == Phase::Close {
            let result = self.channel.as_mut().expect("closing channel").close();
            self.native(result)?;
            self.phase = Phase::WaitClose;
        }
        if self.phase == Phase::WaitClose {
            let result = self.channel.as_mut().expect("closing channel").wait_close();
            self.native(result)?;
            let result = self.channel.as_ref().expect("closed channel").exit_status();
            self.exit_status = Some(self.native(result)?);
            self.phase = Phase::Finished;
        }
        if self.phase == Phase::Finished {
            Ok(self.exit_status.expect("status recorded before finished"))
        } else {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "channel is not active",
            ))
        }
    }

    /// Recover the connection only after complete cleanup; Err retains ownership.
    pub fn into_session(mut self) -> Result<SshConnection, Self> {
        if self.phase != Phase::Finished {
            return Err(self);
        }
        self.channel.take();
        Ok(self.session.take().expect("owned session"))
    }

    /// Refuse further work. Drive poll_dispose or force_dispose before releasing
    /// pool capacity. Drop uses forced termination as a last-resort fallback.
    pub fn abort(&mut self) {
        if !self.is_disposed() {
            self.phase = Phase::Failed;
        }
    }

    /// Retain ownership across WouldBlock. Host supplies readiness/deadline;
    /// on expiry use force_dispose. Success means native owners have been dropped.
    pub fn poll_dispose(&mut self) -> io::Result<()> {
        if self.is_disposed() {
            return Ok(());
        }
        self.abort();
        if let Some(channel) = self.channel.as_mut() {
            let result = channel.close();
            if let Err(error) = self.native(result) {
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Err(error);
                }
                // A failed close also requires socket termination before free.
            }
        }
        self.force_dispose()
    }

    /// Bounded local termination; no peer acknowledgement or message replay.
    pub fn force_dispose(&mut self) -> io::Result<()> {
        self.abort();
        if let Some(connection) = &self.session {
            connection.terminate()?;
        }
        self.channel.take();
        self.session.take();
        self.phase = Phase::Disposed;
        Ok(())
    }

    pub fn is_disposed(&self) -> bool {
        self.phase == Phase::Disposed
    }

    fn active(&self) -> io::Result<()> {
        if self.phase == Phase::Active {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "channel is not active",
            ))
        }
    }
    fn native<T>(&mut self, result: Result<T, ssh2::Error>) -> io::Result<T> {
        self.io(result.map_err(|error| {
            let kind = io::Error::from(ssh2::Error::from_errno(error.code())).kind();
            io::Error::new(kind, error)
        }))
    }
    fn io<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        if result
            .as_ref()
            .is_err_and(|error| error.kind() != io::ErrorKind::WouldBlock)
        {
            self.phase = Phase::Failed;
        }
        result
    }
}
impl Read for SshChannel {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.active()?;
        if output.is_empty() {
            return Ok(0);
        }
        let result = self.channel.as_mut().expect("active channel").read(output);
        let count = self.io(result)?;
        self.stdout_eof |= count == 0;
        Ok(count)
    }
}
impl Write for SshChannel {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.active()?;
        if self.sent_eof {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let result = self.channel.as_mut().expect("active channel").write(input);
        self.io(result)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.active()?;
        // There is no outgoing application buffer. ssh2's similarly named
        // flush discards inbound data; it is NOT std::io::Write::flush.
        Ok(())
    }
}

impl Drop for SshChannel {
    fn drop(&mut self) {
        let _ = self.force_dispose();
    }
}
