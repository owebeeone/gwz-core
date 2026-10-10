//! A disposable OpenSSH server for the SSH endpoint's tests: a loopback
//! `sshd` (`/usr/sbin/sshd`, or Windows' own `sshd.exe`; see `fixture_host`)
//! on a high port with temporary host and client keys, a temporary
//! `known_hosts` that trusts only its host key, and a bare repository whose
//! name carries a shell-injection marker. It never reads or changes the user's
//! SSH configuration, keys, agent or `known_hosts`, and it stops and reaps its
//! server, and on Windows every process the server started, on drop. A missing
//! `sshd` fails the test rather than skipping it.
use super::{
    fixture_host::{self, Programs},
    fixture_job::ProcessJob,
};
pub(crate) use super::{ssh_channel, ssh_connection};

use ssh2::{CheckResult, KnownHostFileKind};
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub(crate) use ssh_channel::{GitService, SshChannel};
pub(crate) use ssh_connection::SshConnection;

/// The transport sets up as many connections to one host at once as an
/// operation allows, 32 by default. OpenSSH's default `MaxStartups` drops
/// unauthenticated connections beyond 10, so the fixture's server takes more.
const OPEN_STARTUPS: &str = "MaxStartups 64\n";

/// How long a server has to start accepting: a Windows `sshd.exe` is slower to start than a Unix one.
const READY_WITHIN: Duration = Duration::from_secs(if cfg!(windows) { 20 } else { 5 });

pub(crate) struct SshdFixture {
    pub(crate) temp: TempDir,
    pub(crate) child: Child,
    job: ProcessJob,
    pub(crate) port: u16,
    pub(crate) user: String,
    pub(crate) known_hosts: PathBuf,
    pub(crate) repository: PathBuf,
    pub(crate) marker: PathBuf,
    pub(crate) authenticated_sessions: usize,
}

impl SshdFixture {
    pub(crate) fn new() -> Self {
        Self::new_mode(false, OPEN_STARTUPS)
    }

    pub(crate) fn new_debug() -> Self {
        Self::new_mode(true, OPEN_STARTUPS)
    }

    /// A server with `startups` in place of [`OPEN_STARTUPS`]: its
    /// `MaxStartups` line, and any other directive the test needs with it.
    pub(crate) fn with_startups(startups: &str) -> Self {
        Self::new_mode(false, startups)
    }

    /// A server whose configuration starts with `extra`, which therefore
    /// overrides the lines after it, since sshd keeps a keyword's first value.
    /// It logs each authentication, at `VERBOSE`, to [`Self::log`]'s file.
    pub(crate) fn with_config(extra: &str) -> Self {
        Self::build(false, OPEN_STARTUPS, &format!("LogLevel VERBOSE\n{extra}"))
    }

    /// What a server made by [`Self::with_config`] has logged.
    pub(crate) fn log(&self) -> String {
        fs::read_to_string(self.temp.path().join("sshd.log")).unwrap_or_default()
    }

    fn new_mode(debug: bool, startups: &str) -> Self {
        Self::build(debug, startups, "")
    }

    fn build(debug: bool, startups: &str, extra: &str) -> Self {
        let programs = fixture_host::programs();
        let temp = TempDir::new().unwrap();
        let host_key = temp.path().join("host_ed25519");
        let client_key = temp.path().join("client_ed25519");
        run_fixture_keygen(&programs, &host_key);
        run_fixture_keygen(&programs, &client_key);
        let authorized = temp.path().join("authorized_keys");
        fs::copy(client_key.with_extension("pub"), &authorized).unwrap();
        let marker = temp.path().join("injection-marker");
        let known_hosts = temp.path().join("known_hosts");
        let repository = temp.path().join(fixture_host::repository_name(&marker));
        run(Command::new("git")
            .args(["init", "--bare", "--initial-branch=main", "--"])
            .arg(&repository));
        let port = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let host_public = fs::read_to_string(host_key.with_extension("pub")).unwrap();
        fs::write(
            &known_hosts,
            fixture_host::known_hosts_line(&format!("[127.0.0.1]:{port}"), &host_public),
        )
        .unwrap();
        let config = temp.path().join("sshd_config");
        let user = fixture_host::login_name();
        let session = fixture_host::session_directives(temp.path());
        let config_text =
            fixture_host::server_config(extra, port, &host_key, &authorized, &session, startups);
        fs::write(&config, config_text).unwrap();
        run(Command::new(&programs.sshd).args(["-t", "-f"]).arg(&config));
        let log = temp.path().join("sshd.log");
        let mut server = Command::new(&programs.sshd);
        server.args(["-D", "-e"]);
        if debug {
            server.arg("-ddd");
        }
        if !extra.is_empty() || cfg!(windows) {
            // A Windows server's reasons for failing to start go to this log, since nothing reads its stderr.
            server.arg("-E").arg(&log);
        }
        let job = ProcessJob::new().unwrap();
        let child = server
            .args(["-f"])
            .arg(&config)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        job.adopt(&child).unwrap();
        let mut fixture = Self {
            temp,
            child,
            job,
            port,
            user,
            known_hosts,
            repository,
            marker,
            authenticated_sessions: 0,
        };
        if !debug {
            fixture.wait_ready();
        }
        fixture
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + READY_WITHIN;
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!(
                    "temporary sshd exited with {status} before it was ready: {}",
                    self.log()
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("temporary sshd did not become ready: {}", self.log());
    }

    /// This server's host key as a `known_hosts` line for `name` at `port`:
    /// plain, or hashed as OpenSSH hashes a name, an HMAC-SHA1 keyed by a
    /// salt of `[name]:port` exactly as given.
    pub(crate) fn known_host(&self, name: &str, port: u16, hashed: bool) -> String {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use sha1::{Digest, Sha1};
        let line = fs::read_to_string(&self.known_hosts).unwrap();
        let (_, key) = line.split_once(' ').unwrap();
        let host = format!("[{name}]:{port}");
        if !hashed {
            return format!("{host} {key}");
        }
        let salt: [u8; 20] = std::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11));
        let mut block = [0u8; 64];
        block[..salt.len()].copy_from_slice(&salt);
        let inner = Sha1::new()
            .chain_update(block.map(|b| b ^ 0x36))
            .chain_update(host.as_bytes())
            .finalize();
        let hash = Sha1::new()
            .chain_update(block.map(|b| b ^ 0x5c))
            .chain_update(inner)
            .finalize();
        format!(
            "|1|{}|{} {key}",
            STANDARD.encode(salt),
            STANDARD.encode(hash)
        )
    }

    pub(crate) fn session(&mut self) -> SshConnection {
        self.session_with_retry(false)
    }

    pub(crate) fn debug_session(&mut self) -> SshConnection {
        self.session_with_retry(true)
    }

    pub(crate) fn session_with_retry(&mut self, retry: bool) -> SshConnection {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.try_session() {
                Ok(connection) => return connection,
                Err(_error) if retry && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("temporary SSH session setup failed: {error}"),
            }
        }
    }

    pub(crate) fn try_session(&mut self) -> io::Result<SshConnection> {
        let connection = self.dialer(self.port)()?;
        self.authenticated_sessions += 1;
        Ok(connection)
    }

    /// Opens an authenticated, nonblocking session through `port`, which may
    /// be a proxy in front of this server: the host key checked is this
    /// server's own.
    pub(crate) fn dialer(
        &self,
        port: u16,
    ) -> impl Fn() -> io::Result<SshConnection> + Send + Sync + 'static {
        let (server_port, user, known_hosts) =
            (self.port, self.user.clone(), self.known_hosts.clone());
        let key = self.temp.path().join("client_ed25519");
        move || {
            let stream = TcpStream::connect(("127.0.0.1", port))?;
            let mut connection = SshConnection::new(stream)?;
            {
                let session = connection.session();
                session.set_timeout(5_000);
                session.handshake()?;
                let (host_key, _) = session
                    .host_key()
                    .ok_or_else(|| io::Error::other("sshd did not provide a host key"))?;
                let host_key = host_key.to_owned();
                let mut known = session.known_hosts()?;
                known.read_file(&known_hosts, KnownHostFileKind::OpenSSH)?;
                if !matches!(
                    known.check_port("127.0.0.1", server_port, &host_key),
                    CheckResult::Match
                ) {
                    return Err(io::Error::other("temporary host key did not match"));
                }
                session.userauth_pubkey_file(&user, None, &key, None)?;
                if !session.authenticated() {
                    return Err(io::Error::other("temporary SSH authentication failed"));
                }
            }
            connection.set_nonblocking()?;
            Ok(connection)
        }
    }
}

impl Drop for SshdFixture {
    fn drop(&mut self) {
        // The job ends every process the server started, by membership; the child is then reaped.
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn run_keygen(path: &Path) {
    run(Command::new(fixture_host::programs().keygen)
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(path));
}

/// Writes the key a fixture server or client uses: ed25519 where the fixture's libssh2 reads it, RSA in PEM form
/// on Windows, where it does not.
fn run_fixture_keygen(programs: &Programs, path: &Path) {
    run(Command::new(&programs.keygen)
        .args(["-q"])
        .args(fixture_host::fixture_key_arguments())
        .args(["-N", ""])
        .arg("-f")
        .arg(path));
}

pub(crate) fn run(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn run_output(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::collections::HashMap;

        pub(crate) struct PausedProcessTree {
            pids: Vec<i32>,
            resumed: bool,
        }

        impl PausedProcessTree {
            pub(crate) fn resume(&mut self) {
                if self.resumed {
                    return;
                }
                self.resumed = true;
                for pid in &self.pids {
                    // Teardown must not panic while unwinding; an exited child needs no
                    // resume. Keep the guard armed before the first STOP below.
                    let _ = Command::new("kill")
                        .args(["-CONT", &pid.to_string()])
                        .output();
                }
            }
        }

        impl Drop for PausedProcessTree {
            fn drop(&mut self) {
                self.resume();
            }
        }

        pub(crate) fn pause_process_tree(root: u32) -> PausedProcessTree {
            thread::sleep(Duration::from_millis(100));
            let output = run_output(Command::new("ps").args(["-axo", "pid=,ppid=,state="]));
            let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
            for line in output.lines() {
                let mut fields = line.split_whitespace();
                let Some(pid) = fields.next().and_then(|value| value.parse::<i32>().ok()) else {
                    continue;
                };
                let Some(ppid) = fields.next().and_then(|value| value.parse::<i32>().ok()) else {
                    continue;
                };
                if fields.next().is_some_and(|state| state.starts_with('Z')) {
                    continue;
                }
                children.entry(ppid).or_default().push(pid);
            }
            let mut pids = Vec::new();
            collect_children(root as i32, &children, &mut pids);
            pids.push(root as i32);
            let mut paused = PausedProcessTree {
                pids: Vec::new(),
                resumed: false,
            };
            for pid in pids {
                paused.pids.push(pid);
                run(Command::new("kill").args(["-STOP", &pid.to_string()]));
            }
            for pid in &paused.pids {
                let state = run_output(Command::new("ps").args(["-o", "state=", "-p", &pid.to_string()]));
                assert!(
                    state.starts_with('T'),
                    "fixture process {pid} was not stopped: {state}"
                );
            }
            paused
        }

        fn collect_children(root: i32, children: &HashMap<i32, Vec<i32>>, output: &mut Vec<i32>) {
            let Some(owned) = children.get(&root) else {
                return;
            };
            for child in owned {
                collect_children(*child, children, output);
                output.push(*child);
            }
        }
    } else if #[cfg(windows)] {
        /// A server's process tree, suspended thread by thread until [`Self::resume`] or drop.
        pub(crate) struct PausedProcessTree(super::fixture_job::Suspended);

        impl PausedProcessTree {
            pub(crate) fn resume(&mut self) {
                self.0.resume();
            }
        }

        /// Suspends `root` and every process descended from it, as `kill -STOP` does on Unix.
        pub(crate) fn pause_process_tree(root: u32) -> PausedProcessTree {
            // Give the server's session processes a moment to exist, as the Unix twin does.
            thread::sleep(Duration::from_millis(100));
            PausedProcessTree(super::fixture_job::suspend_tree(root).expect("the server's processes were suspended"))
        }
    }
}

pub(crate) fn open(channel: &mut SshChannel) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match channel.poll_open() {
            Ok(()) => return,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("SSH channel open failed: {error}"),
        }
        assert!(Instant::now() < deadline, "SSH channel open timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

pub(crate) fn exchange(channel: &mut SshChannel) -> (Vec<u8>, Vec<u8>, i32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut advertised = false;
    let request = b"0000";
    let mut request_written = 0;
    let mut request_flushed = false;
    let mut sent_eof = false;
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    while Instant::now() < deadline {
        if !stdout_eof {
            let mut output = [0; 4096];
            match channel.read(&mut output) {
                Ok(0) => stdout_eof = true,
                Ok(count) => {
                    stdout.extend_from_slice(&output[..count]);
                    assert!(
                        stdout.len() <= 64 * 1024,
                        "stdout diagnostic exceeded bound"
                    );
                    advertised |= stdout.windows(4).any(|window| window == b"0000");
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("SSH stdout read failed: {error}"),
            }
        }
        if !stderr_eof {
            let mut diagnostics = [0; 4096];
            match channel.read_stderr(&mut diagnostics) {
                Ok(0) => stderr_eof = true,
                Ok(count) => {
                    stderr.extend_from_slice(&diagnostics[..count]);
                    assert!(
                        stderr.len() <= 64 * 1024,
                        "stderr diagnostic exceeded bound"
                    );
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("SSH stderr read failed: {error}"),
            }
        }
        if advertised && !sent_eof {
            if request_written < request.len() {
                match channel.write(&request[request_written..]) {
                    Ok(count) => request_written += count,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("SSH request write failed: {error}"),
                }
            }
            if request_written == request.len() && !request_flushed {
                match channel.flush() {
                    Ok(()) => request_flushed = true,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("SSH request flush failed: {error}"),
                }
            }
            if request_flushed {
                match channel.send_eof() {
                    Ok(()) => sent_eof = true,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("SSH EOF failed: {error}"),
                }
            }
        }
        if sent_eof && stdout_eof && stderr_eof {
            match channel.finish() {
                Ok(status) => return (stdout, stderr, status),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("SSH channel finish failed: {error}"),
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("SSH channel exchange timed out; stderr={stderr:?}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn the_server_answers_a_handshake_and_serves_upload_pack_for_the_bare_repository() {
        let mut fixture = SshdFixture::new();
        let session = fixture.session();
        let repository = fixture.repository.to_str().unwrap().to_owned();
        let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
        open(&mut channel);
        let (stdout, stderr, status) = exchange(&mut channel);
        assert!(
            stdout.windows(4).any(|window| window == b"0000"),
            "{stdout:?}"
        );
        assert!(
            stderr.is_empty(),
            "unexpected upload-pack diagnostics: {stderr:?}"
        );
        assert_eq!(status, 0);
        assert_eq!(fixture.authenticated_sessions, 1);
        assert!(
            !fixture.marker.exists(),
            "repository path was shell-injected"
        );
    }

    #[test]
    fn the_known_hosts_file_trusts_the_server_key_and_nothing_else() {
        let fixture = SshdFixture::new();
        let text = fs::read_to_string(&fixture.known_hosts).unwrap();
        let fields: Vec<&str> = text.split_whitespace().collect();
        assert_eq!(text.lines().count(), 1, "{text:?}");
        assert!(text.ends_with('\n') && !text.contains('\r'), "{text:?}");
        assert!(fields.len() >= 3, "{text:?}");
        assert_eq!(fields[0], format!("[127.0.0.1]:{}", fixture.port));
        assert!(fields[1].starts_with("ssh-"), "{text:?}");
    }

    #[test]
    fn a_paused_server_gives_no_banner_until_it_is_resumed() {
        let fixture = SshdFixture::new();
        let mut paused = pause_process_tree(fixture.child.id());
        let mut stream = TcpStream::connect(("127.0.0.1", fixture.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut banner = [0u8; 8];
        let error = stream
            .read(&mut banner)
            .expect_err("a paused server sent a banner");
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ),
            "{error}"
        );
        paused.resume();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.read_exact(&mut banner).unwrap();
        assert_eq!(&banner[..4], b"SSH-");
    }

    #[test]
    fn dropping_the_fixture_stops_the_server() {
        let fixture = SshdFixture::new();
        let port = fixture.port;
        drop(fixture);
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(("127.0.0.1", port)).is_ok() {
            assert!(
                Instant::now() < deadline,
                "the server still accepted after its fixture dropped"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}
