//! A disposable OpenSSH server for the SSH endpoint's tests: a loopback
//! `sshd` (`/usr/sbin/sshd`, or Windows' own `sshd.exe`; see `fixture_host`)
//! on a high port with temporary host and client keys, a temporary
//! `known_hosts` that trusts only its host key, and a bare repository whose
//! name carries a shell-injection marker. It never reads or changes the user's
//! SSH configuration, keys, agent or `known_hosts`, and it stops and reaps its
//! server, and on Windows every process the server started, on drop. A missing
//! `sshd` fails the test rather than skipping it.
pub(crate) use super::{fixture_host::server_path, ssh_channel, ssh_connection};
use super::{
    fixture_host::{self, Programs},
    fixture_job::ProcessJob,
};
use crate::test_support::fixture_port;

use ssh2::{CheckResult, KnownHostFileKind};
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub(crate) use dial::{DialRetries, dialer, dialer_counting, handshaken, handshaken_counting};
pub(crate) use ssh_channel::{GitService, SshChannel};
pub(crate) use ssh_connection::SshConnection;

/// The transport sets up as many connections to one host at once as an
/// operation allows, 32 by default. OpenSSH's default `MaxStartups` drops
/// unauthenticated connections beyond 10, so the fixture's server takes more.
const OPEN_STARTUPS: &str = "MaxStartups 64\n";

/// What a session says when the server answering is not the fixture's own.
const HOST_KEY_MISMATCH: &str = "temporary host key did not match";

/// How many ports a fixture's server is tried on before the fixture gives up.
const START_ATTEMPTS: usize = 5;

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
    /// The path a URL names the repository by: [`Self::repository`] itself, except on Windows (`fixture_host`).
    pub(crate) url_repository: PathBuf,
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

    /// A server whose sessions run the forced script `ssh_close_fixture` leaves in its directory.
    pub(crate) fn new_forced() -> Self {
        Self::build_with(false, OPEN_STARTUPS, "", true)
    }

    fn new_mode(debug: bool, startups: &str) -> Self {
        Self::build(debug, startups, "")
    }

    fn build(debug: bool, startups: &str, extra: &str) -> Self {
        Self::build_with(debug, startups, extra, false)
    }

    fn build_with(debug: bool, startups: &str, extra: &str, forced: bool) -> Self {
        Self::build_on(debug, startups, extra, forced, fixture_port)
    }

    /// A server on the port `next_port` names; a seam for a test that makes two fixtures meet on one port.
    pub(crate) fn new_on_ports(next_port: impl FnMut() -> u16) -> Self {
        Self::build_on(false, OPEN_STARTUPS, "", false, next_port)
    }

    /// [`Self::new_on_ports`] for a server that logs its debugging output and serves one connection.
    pub(crate) fn new_debug_on_ports(next_port: impl FnMut() -> u16) -> Self {
        Self::build_on(true, OPEN_STARTUPS, "", false, next_port)
    }

    /// A running server of the fixture's own on a port `next_port` names. A port is picked by binding and releasing
    /// one, so another test's server can take it before this one binds it: the server then exits, or never was the
    /// one that answers. That attempt ends (its server is stopped with its fixture) and the next starts on the next
    /// port, up to [`START_ATTEMPTS`] times.
    fn build_on(
        debug: bool,
        startups: &str,
        extra: &str,
        forced: bool,
        mut next_port: impl FnMut() -> u16,
    ) -> Self {
        let mut refused = Vec::new();
        for _ in 0..START_ATTEMPTS {
            let port = next_port();
            match Self::start_on(port, debug, startups, extra, forced) {
                Ok(fixture) => return fixture,
                Err(reason) => {
                    eprintln!(
                        "ssh fixture: no server of its own on port {port} ({reason}); trying another port"
                    );
                    refused.push(format!("port {port}: {reason}"));
                }
            }
        }
        panic!(
            "temporary sshd did not start in {START_ATTEMPTS} attempts: {}",
            refused.join("; ")
        );
    }

    fn start_on(
        port: u16,
        debug: bool,
        startups: &str,
        extra: &str,
        forced: bool,
    ) -> Result<Self, String> {
        let programs = fixture_host::programs();
        let Layout {
            temp,
            host_key,
            authorized,
            marker,
            known_hosts,
            repository,
            url_repository,
            user,
        } = Layout::new(port);
        let config = temp.path().join("sshd_config");
        let session = if forced {
            fixture_host::forced_session_directives(temp.path())
        } else {
            fixture_host::session_directives(temp.path())
        };
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
        fixture_host::server_environment(temp.path(), &mut server);
        let job = ProcessJob::named(&fixture_host::job_name(temp.path())).unwrap();
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
            url_repository,
            repository,
            marker,
            authenticated_sessions: 0,
        };
        if !debug {
            // Not ready: the fixture is dropped here, which stops exactly this attempt's server. (A debugging
            // server serves one connection, which a probe must not use; see `debug_session`.)
            fixture.wait_ready()?;
        }
        Ok(fixture)
    }

    /// Ready once this fixture's own server answers, and not before: a server that exited (it could not bind its
    /// port) or a foreign one that answers on that port is not it.
    fn wait_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + READY_WITHIN;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Err(format!(
                    "it exited with {status} before it was ready: {}",
                    self.log()
                ));
            }
            match answering(self.port, &self.known_hosts) {
                Answer::Own => return Ok(()),
                Answer::Foreign => {
                    return Err("another server answers on the port".to_owned());
                }
                Answer::Nothing => {}
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(format!(
            "it was not ready in {READY_WITHIN:?}: {}",
            self.log()
        ))
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
        match self.try_session() {
            Ok(connection) => connection,
            Err(error) => panic!("temporary SSH session setup failed: {error}"),
        }
    }

    /// A session on a debugging server, which serves one connection and so cannot be probed for being the fixture's
    /// own: the session's own handshake is the proof, as the host key it checks is this fixture's. The connection
    /// is retried while the server has yet to listen. A server that is not the fixture's own (another test's server
    /// answers on the port, or this one could not bind it and exited) is replaced by a new one on a new port, up to
    /// [`START_ATTEMPTS`] times, and the fixture's port and files are then the new server's.
    pub(crate) fn debug_session(&mut self) -> SshConnection {
        let mut refused = Vec::new();
        for _ in 0..START_ATTEMPTS {
            let deadline = Instant::now() + READY_WITHIN;
            let reason = loop {
                match self.try_session() {
                    Ok(connection) => return connection,
                    Err(error) if error.to_string() == HOST_KEY_MISMATCH => {
                        break error.to_string();
                    }
                    Err(error) => {
                        if self.child.try_wait().unwrap().is_some() {
                            break format!("the server exited ({error})");
                        }
                        if Instant::now() >= deadline {
                            break format!("no session in {READY_WITHIN:?} ({error})");
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                }
            };
            eprintln!(
                "ssh fixture: no debugging server of its own on port {} ({reason}); trying another port",
                self.port
            );
            refused.push(format!("port {}: {reason}", self.port));
            *self = Self::build_on(true, OPEN_STARTUPS, "", false, fixture_port);
        }
        panic!(
            "temporary debugging sshd did not start in {START_ATTEMPTS} attempts: {}",
            refused.join("; ")
        );
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
        self.dialer_counting(port, DialRetries::default())
    }

    /// [`Self::dialer`], whose retried connections are counted (and, if `retries` says so,
    /// injected) as [`DialRetries`] describes.
    pub(crate) fn dialer_counting(
        &self,
        port: u16,
        retries: DialRetries,
    ) -> impl Fn() -> io::Result<SshConnection> + Send + Sync + 'static {
        dialer_counting(
            port,
            self.port,
            self.user.clone(),
            self.known_hosts.clone(),
            self.temp.path().join("client_ed25519"),
            retries,
        )
    }
}

/// How long a probe's handshake may take before the probe tries again.
const PROBE_MS: u32 = 1_000;

/// Whose server answers on a port.
enum Answer {
    /// The server whose host key the fixture's `known_hosts` trusts.
    Own,
    /// A server that answers with another host key.
    Foreign,
    /// Nothing that completes an SSH handshake yet: the port refuses, or what holds it says nothing, or the server
    /// dropped the probe (`MaxStartups`).
    Nothing,
}

/// Connects to `port` and completes the handshake, which is as far as a host key is known, and judges the key
/// against the fixture's own `known_hosts`.
fn answering(port: u16, known_hosts: &Path) -> Answer {
    let Ok((mut connection, _watch)) = handshaken(port, PROBE_MS) else {
        return Answer::Nothing;
    };
    let session = connection.session();
    let Some((host_key, _)) = session.host_key() else {
        return Answer::Nothing;
    };
    let host_key = host_key.to_owned();
    let Ok(mut known) = session.known_hosts() else {
        return Answer::Nothing;
    };
    if known
        .read_file(known_hosts, KnownHostFileKind::OpenSSH)
        .is_err()
    {
        return Answer::Nothing;
    }
    if matches!(
        known.check_port("127.0.0.1", port, &host_key),
        CheckResult::Match
    ) {
        Answer::Own
    } else {
        Answer::Foreign
    }
}

/// The directory every fixture server stands in, whichever server it is (the system's `sshd`, or the test-controlled
/// one in `ssh_controlled_server`): its host key and the client key it trusts, a bare repository whose name carries
/// a shell-injection marker, and a `known_hosts` that trusts only the host key at `port`.
pub(crate) struct Layout {
    pub(crate) temp: TempDir,
    pub(crate) host_key: PathBuf,
    pub(crate) authorized: PathBuf,
    pub(crate) marker: PathBuf,
    pub(crate) known_hosts: PathBuf,
    pub(crate) repository: PathBuf,
    /// The path a URL names the repository by (see [`SshdFixture::url_repository`]).
    pub(crate) url_repository: PathBuf,
    pub(crate) user: String,
}

impl Layout {
    pub(crate) fn new(port: u16) -> Self {
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
        let url_repository = fixture_host::url_repository(temp.path(), &repository);
        let host_public = fs::read_to_string(host_key.with_extension("pub")).unwrap();
        fs::write(
            &known_hosts,
            fixture_host::known_hosts_line(&format!("[127.0.0.1]:{port}"), &host_public),
        )
        .unwrap();
        Self {
            temp,
            host_key,
            authorized,
            marker,
            known_hosts,
            repository,
            url_repository,
            user: fixture_host::login_name(),
        }
    }
}

/// What a test needs to reach a fixture server as the transport's client does: where it listens, who logs in, what
/// the client trusts and the key it presents, and the repository it serves.
pub(crate) trait Reach {
    fn port(&self) -> u16;
    fn user(&self) -> &str;
    fn known_hosts(&self) -> &Path;
    fn client_key(&self) -> PathBuf;
    fn repository(&self) -> &Path;
}

impl Reach for SshdFixture {
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

impl Drop for SshdFixture {
    fn drop(&mut self) {
        // The job ends every process the server started, by membership; the child is then reaped.
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
        // A row that failed says what its server saw (the log is only read once the server has ended: a Windows
        // server holds it without sharing).
        if thread::panicking() {
            let log = self.log();
            if !log.is_empty() {
                eprintln!("the fixture's sshd logged:\n{log}");
            }
        }
    }
}

/// Writes a key the fixture server trusts: [`run_fixture_keygen`] for the host's programs.
pub(crate) fn run_fixture_key(path: &Path) {
    run_fixture_keygen(&fixture_host::programs(), path);
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

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// A path that names no regular file and that a plain open would block on: a FIFO in `dir`.
        pub(crate) fn special_file(dir: &Path) -> PathBuf {
            let path = dir.join("pipe");
            let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            path
        }
    } else {
        /// A device path with no server behind it: a reader of key and trust files refuses it before it opens it.
        pub(crate) fn special_file(_dir: &Path) -> PathBuf {
            PathBuf::from(r"\\.\pipe\gwz-fixture-no-server")
        }
    }
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
    let deadline = Instant::now() + Duration::from_secs(30);
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
    let deadline = Instant::now() + Duration::from_secs(30);
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

mod dial;
mod dial_retry_tests;
mod port_stress;
mod port_tests;
