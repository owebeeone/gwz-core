#[path = "../../../src/git/endpoint/ssh_channel.rs"]
mod ssh_channel;

use ssh2::{CheckResult, KnownHostFileKind, Session};
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

use ssh_channel::{GitService, SshChannel};

struct SshdFixture {
    temp: TempDir,
    child: Child,
    port: u16,
    user: String,
    known_hosts: PathBuf,
    repository: PathBuf,
    marker: PathBuf,
    authenticated_sessions: usize,
}

impl SshdFixture {
    fn new() -> Self {
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
        let child = Command::new("/usr/sbin/sshd")
            .args(["-D", "-e", "-f"])
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
        fixture.wait_ready();
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

    fn session(&mut self) -> Session {
        let stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        let mut session = Session::new().unwrap();
        let nonblocking_control = stream.try_clone().unwrap();
        session.set_tcp_stream(stream);
        session.set_timeout(5_000);
        session.handshake().unwrap();
        let (key, _) = session.host_key().expect("sshd did not provide a host key");
        let key = key.to_owned();
        let mut known = session.known_hosts().unwrap();
        known
            .read_file(&self.known_hosts, KnownHostFileKind::OpenSSH)
            .unwrap();
        match known.check_port("127.0.0.1", self.port, &key) {
            CheckResult::Match => {}
            result => panic!("temporary host key did not match: {result:?}"),
        }
        session
            .userauth_pubkey_file(
                &self.user,
                None,
                &self.temp.path().join("client_ed25519"),
                None,
            )
            .unwrap();
        assert!(session.authenticated());
        nonblocking_control.set_nonblocking(true).unwrap();
        session.set_blocking(false);
        self.authenticated_sessions += 1;
        session
    }
}

impl Drop for SshdFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run_keygen(path: &Path) {
    run(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(path));
}

fn run(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_output(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn open(channel: &mut SshChannel) {
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

fn exchange(channel: &mut SshChannel) -> (Vec<u8>, Vec<u8>, i32) {
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

#[test]
fn receive_pack_reuses_one_authenticated_connection_and_quotes_repository() {
    let mut fixture = SshdFixture::new();
    let mut session = fixture.session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    for _ in 0..2 {
        let mut channel = SshChannel::new(session, GitService::ReceivePack, &repository).unwrap();
        open(&mut channel);
        let (stdout, stderr, status) = exchange(&mut channel);
        assert!(stdout.windows(4).any(|window| window == b"0000"));
        assert!(
            stderr.is_empty(),
            "unexpected receive-pack diagnostics: {stderr:?}"
        );
        assert_eq!(status, 0);
        session = match channel.into_session() {
            Ok(session) => session,
            Err(_) => panic!("receive-pack channel was not fully cleaned up"),
        };
    }
    assert_eq!(fixture.authenticated_sessions, 1);
    assert!(
        !fixture.marker.exists(),
        "repository path was shell-injected"
    );
}

#[test]
fn upload_pack_advertisement_and_abort_keep_cleanup_explicit() {
    let mut fixture = SshdFixture::new();
    let session = fixture.session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
    open(&mut channel);
    channel = match channel.into_session() {
        Ok(_) => panic!("open channel yielded a reusable session"),
        Err(channel) => channel,
    };
    let (stdout, stderr, status) = exchange(&mut channel);
    assert!(stdout.windows(4).any(|window| window == b"0000"));
    assert!(
        stderr.is_empty(),
        "unexpected upload-pack diagnostics: {stderr:?}"
    );
    assert_eq!(status, 0);
    let session = match channel.into_session() {
        Ok(session) => session,
        Err(_) => panic!("finished upload-pack did not release its session"),
    };
    let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
    open(&mut channel);
    channel.abort();
    let mut output = [0; 1];
    let error = channel.read(&mut output).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert!(
        channel.into_session().is_err(),
        "aborted channel must never yield a session"
    );
}
