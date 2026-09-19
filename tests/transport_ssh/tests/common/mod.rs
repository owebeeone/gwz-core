#![allow(dead_code)]

#[path = "../../../../src/git/endpoint/ssh_channel.rs"]
pub(crate) mod ssh_channel;
#[path = "../../../../src/git/endpoint/ssh_connection.rs"]
pub(crate) mod ssh_connection;

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
use std::collections::HashMap;

pub(crate) struct SshdFixture {
    pub(crate) temp: TempDir,
    pub(crate) child: Child,
    pub(crate) port: u16,
    pub(crate) user: String,
    pub(crate) known_hosts: PathBuf,
    pub(crate) repository: PathBuf,
    pub(crate) marker: PathBuf,
    pub(crate) authenticated_sessions: usize,
}

impl SshdFixture {
    pub(crate) fn new() -> Self {
        Self::new_mode(false)
    }

    pub(crate) fn new_debug() -> Self {
        Self::new_mode(true)
    }

    fn new_mode(debug: bool) -> Self {
        assert!(
            Path::new("/usr/sbin/sshd").exists(),
            "native gate requires /usr/sbin/sshd; this is not a skipped qualification"
        );
        let temp = TempDir::new().unwrap();
        let host_key = temp.path().join("host_ed25519");
        let client_key = temp.path().join("client_ed25519");
        run_keygen(&host_key);
        run_keygen(&client_key);
        let authorized = temp.path().join("authorized_keys");
        fs::copy(client_key.with_extension("pub"), &authorized).unwrap();
        let marker = temp.path().join("injection-marker");
        let known_hosts = temp.path().join("known_hosts");
        let repository = temp
            .path()
            .join("repo'$(touch ".to_owned() + marker.to_str().unwrap() + ")'");
        run(Command::new("git")
            .args(["init", "--bare", "--initial-branch=main", "--"])
            .arg(&repository));
        let port = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let host_public = fs::read_to_string(host_key.with_extension("pub")).unwrap();
        fs::write(&known_hosts, format!("[127.0.0.1]:{port} {host_public}")).unwrap();
        let config = temp.path().join("sshd_config");
        let user = run_output(Command::new("id").args(["-un"]));
        let config_text = format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\nPidFile none\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nChallengeResponseAuthentication no\nUsePAM no\nPermitRootLogin yes\nPubkeyAuthentication yes\nStrictModes no\nLogLevel ERROR\n",
            host_key.display(),
            authorized.display(),
        );
        fs::write(&config, config_text).unwrap();
        run(Command::new("/usr/sbin/sshd")
            .args(["-t", "-f"])
            .arg(&config));
        let mut server = Command::new("/usr/sbin/sshd");
        server.args(["-D", "-e"]);
        if debug {
            server.arg("-ddd");
        }
        let child = server
            .args(["-f"])
            .arg(&config)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let fixture = Self {
            temp,
            child,
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

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("temporary sshd did not become ready");
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
        let stream = TcpStream::connect(("127.0.0.1", self.port))?;
        let mut connection = SshConnection::new(stream)?;
        {
            let session = connection.session();
            session.set_timeout(5_000);
            session.handshake()?;
            let (key, _) = session
                .host_key()
                .ok_or_else(|| io::Error::other("sshd did not provide a host key"))?;
            let key = key.to_owned();
            let mut known = session.known_hosts()?;
            known.read_file(&self.known_hosts, KnownHostFileKind::OpenSSH)?;
            if !matches!(
                known.check_port("127.0.0.1", self.port, &key),
                CheckResult::Match
            ) {
                return Err(io::Error::other("temporary host key did not match"));
            }
            session.userauth_pubkey_file(
                &self.user,
                None,
                &self.temp.path().join("client_ed25519"),
                None,
            )?;
            if !session.authenticated() {
                return Err(io::Error::other("temporary SSH authentication failed"));
            }
        }
        connection.set_nonblocking()?;
        self.authenticated_sessions += 1;
        Ok(connection)
    }
}

impl Drop for SshdFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn run_keygen(path: &Path) {
    run(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", ""])
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

pub(crate) fn open(channel: &mut SshChannel) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let _ = channel.block_directions();
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
