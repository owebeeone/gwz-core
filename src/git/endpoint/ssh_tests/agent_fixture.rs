//! A fake SSH agent for the endpoint's agent tests: a proxy socket in front of
//! a private `ssh-agent` holding two fixture keys, one authorized and one not.
//! It records each list and sign request, can stall or damage a signature,
//! and never contacts the user's agent.
use crate::git::endpoint::{
    agent_source::AgentSource,
    ssh_fixture::{self as common, SshConnection, SshdFixture},
};
use std::os::unix::net::{UnixListener, UnixStream};
use std::{
    fs,
    io::{self, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
/// The key type and sign algorithm the endpoint rows use by default: Ed25519.
pub const METHOD: &str = "ssh-ed25519";
/// An agent client over the socket at `path`, for `agent_auth`'s `open` argument.
pub fn connect(
    path: &std::path::Path,
    control: Arc<crate::git::endpoint::agent_job::Control>,
) -> io::Result<
    crate::git::endpoint::agent_client::Agent<impl crate::git::endpoint::agent_client::Channel>,
> {
    crate::git::endpoint::agent_socket::connect(path, control)
}
pub struct Fixture {
    pub ssh: SshdFixture,
    pub path: PathBuf,
    agent: Child,
    monitor: Mutex<Option<TcpStream>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    events: Arc<Mutex<Vec<u32>>>,
    pub signing: std::sync::mpsc::Receiver<()>,
    pub listed: std::sync::mpsc::Receiver<()>,
    pub closed: Arc<AtomicBool>,
    pub fault: Arc<AtomicU8>,
}
impl Fixture {
    pub fn new(method: &str, stall: bool) -> Self {
        let ssh = SshdFixture::new();
        let key = ssh.temp.path().join("auth_key");
        let bad = ssh.temp.path().join("rejected_key");
        for path in [&key, &bad] {
            let mut cmd = Command::new("ssh-keygen");
            cmd.args([
                "-q",
                "-t",
                if method.starts_with("rsa") {
                    "rsa"
                } else {
                    "ed25519"
                },
                "-N",
                "",
                "-f",
            ])
            .arg(path);
            if method.starts_with("rsa") {
                cmd.args(["-b", "2048"]);
            }
            common::run(&mut cmd);
        }
        fs::copy(
            key.with_extension("pub"),
            ssh.temp.path().join("authorized_keys"),
        )
        .unwrap();
        let real = ssh.temp.path().join("real-agent.sock");
        let agent = Command::new("ssh-agent")
            .args(["-D", "-a"])
            .arg(&real)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let path = ssh.temp.path().join("proxy-agent.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let events = Arc::new(Mutex::new(Vec::new()));
        let record = events.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let eof = closed.clone();
        let fault = Arc::new(AtomicU8::new(0));
        let damage = fault.clone();
        let (sign, signing) = std::sync::mpsc::channel();
        let (list, listed) = std::sync::mpsc::channel();
        let mut fixture = Self {
            ssh,
            path,
            agent,
            stop,
            worker: None,
            events,
            signing,
            listed,
            closed,
            fault,
            monitor: Mutex::new(None),
        };
        // ssh-agent binds its socket before it listens: wait until it accepts.
        let deadline = Instant::now() + Duration::from_secs(3);
        while UnixStream::connect(&real).is_err() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        for path in [&bad, &key] {
            common::run(
                Command::new("ssh-add")
                    .env("SSH_AUTH_SOCK", &real)
                    .env_remove("SSH_AGENT_PID")
                    .arg(path),
            );
        }
        fixture.worker = Some(thread::spawn(move || {
            let mut peer = loop {
                match listener.accept() {
                    Ok((peer, _)) => break peer,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if flag.load(Ordering::SeqCst) {
                            return;
                        }
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => panic!("fixture accept {e}"),
                }
            };
            peer.set_nonblocking(false).unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
            peer.set_write_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            let mut upstream = UnixStream::connect(real).unwrap();
            upstream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            upstream
                .set_write_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            loop {
                let Some(body) = read_frame(&mut peer) else {
                    eof.store(true, Ordering::SeqCst);
                    return;
                };
                if body[0] == 13 {
                    let flags = u32::from_be_bytes(body[body.len() - 4..].try_into().unwrap());
                    record.lock().unwrap().push(flags);
                    let _ = sign.send(());
                    if stall {
                        let mut byte = [0];
                        assert_eq!(peer.read(&mut byte).unwrap(), 0);
                        eof.store(true, Ordering::SeqCst);
                        return;
                    }
                } else {
                    assert_eq!(body, [11]);
                    record.lock().unwrap().push(11);
                }
                write_frame(&mut upstream, &body);
                let mut response = read_frame(&mut upstream).unwrap();
                if body[0] == 13 {
                    match damage.swap(0, Ordering::SeqCst) {
                        1 => {
                            // Valid envelope carrying a structurally short signature.
                            let n = u32::from_be_bytes(response[5..9].try_into().unwrap()) as usize;
                            response.truncate(9 + n);
                            response.extend_from_slice(&1_u32.to_be_bytes());
                            response.push(1);
                            let n = (response.len() - 5) as u32;
                            response[1..5].copy_from_slice(&n.to_be_bytes());
                        }
                        2 => {
                            response[9] = b'X';
                        }
                        fault @ (3 | 4) => {
                            let size = if fault == 3 { 1 } else { 257 };
                            let mut signature = Vec::new();
                            for field in [b"ssh-rsa".as_slice(), &vec![0xff; size]] {
                                signature.extend_from_slice(&(field.len() as u32).to_be_bytes());
                                signature.extend_from_slice(field);
                            }
                            response = vec![14];
                            response.extend_from_slice(&(signature.len() as u32).to_be_bytes());
                            response.extend_from_slice(&signature);
                        }
                        _ => {}
                    }
                }
                if body[0] == 11 {
                    assert_eq!(u32::from_be_bytes(response[1..5].try_into().unwrap()), 2);
                }
                write_frame(&mut peer, &response);
                if body[0] == 11 {
                    let _ = list.send(());
                }
            }
        }));
        fixture
    }
    pub fn prepared(&self, method: &str) -> (SshConnection, Vec<u8>) {
        let (mut connection, stream) = common::handshaken(self.ssh.port, 3_000).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        *self.monitor.lock().unwrap() = Some(stream);
        let session = connection.session();
        let key = session.host_key().unwrap().0.to_vec();
        let mut known = session.known_hosts().unwrap();
        known
            .read_file(&self.ssh.known_hosts, ssh2::KnownHostFileKind::OpenSSH)
            .unwrap();
        assert!(matches!(
            known.check_port("127.0.0.1", self.ssh.port, &key),
            ssh2::CheckResult::Match
        ));
        session
            .method_pref(ssh2::MethodType::SignAlgo, method)
            .unwrap();
        (connection, key)
    }
    /// The agent source a runtime would choose for this agent: the socket it serves on.
    pub fn source(&self) -> AgentSource {
        AgentSource::from_path(Some(self.path.clone()))
    }
    /// Whether the agent has been asked to sign (and the signal is consumed).
    pub fn signed(&self) -> bool {
        self.signing.try_recv().is_ok()
    }
    /// Waits for the agent to have been asked to sign.
    pub fn wait_signing(&self, within: Duration) {
        self.signing.recv_timeout(within).unwrap();
    }
    /// Waits for the agent to have been asked for its keys.
    pub fn wait_listed(&self, within: Duration) {
        self.listed.recv_timeout(within).unwrap();
    }
    /// Damages the next signature: 1 is a short signature, 2 a wrong algorithm name, 3 a one-byte `ssh-rsa` and 4 a
    /// 257-byte one.
    pub fn damage(&self, kind: u8) {
        self.fault.store(kind, Ordering::SeqCst);
    }
    /// A raw, nonblocking client of the agent, for a test that wraps its own channel.
    pub fn raw_channel(&self) -> UnixStream {
        let socket = UnixStream::connect(&self.path).unwrap();
        socket.set_nonblocking(true).unwrap();
        socket
    }
    pub fn assert_requests(&self, method: &str, signatures: usize) {
        let mut expected = vec![11];
        expected.extend(std::iter::repeat_n(
            match method {
                "rsa-sha2-256" => 2,
                "rsa-sha2-512" => 4,
                _ => 0,
            },
            signatures,
        ));
        assert_eq!(*self.events.lock().unwrap(), expected);
    }
    pub fn network_handle(&self) -> TcpStream {
        self.monitor
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .try_clone()
            .unwrap()
    }
    pub fn assert_tcp_closed(&self) {
        let mut guard = self.monitor.lock().unwrap();
        let socket = guard.as_mut().unwrap();
        let read = socket.read(&mut [0; 1]);
        assert!(
            reads_closed(&read),
            "connection owner must shut down TCP before disposal ack: {read:?}"
        );
    }
    pub fn wait_closed(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.closed.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
    }
    pub fn no_requests(&self) {
        assert!(self.events.lock().unwrap().is_empty());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.agent.kill();
        let _ = self.agent.wait();
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}
/// Whether a read shows the socket's connection ended: EOF, as macOS reads
/// it, or a reset, as Linux reads it. Linux resets a TCP connection whose
/// socket, shut down for reading, receives more data, and a stream socket
/// whose peer closes with bytes it never read, AF_UNIX included. A handle
/// left open reads neither: the read times out, so each caller sets a timeout.
pub fn reads_closed(read: &io::Result<usize>) -> bool {
    match read {
        Ok(count) => *count == 0,
        Err(error) => error.kind() == io::ErrorKind::ConnectionReset,
    }
}
fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut header = [0; 4];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return None,
        Err(e) => panic!("fixture read {e}"),
    }
    let len = u32::from_be_bytes(header) as usize;
    assert!((1..=1 << 20).contains(&len));
    let mut body = vec![0; len];
    stream.read_exact(&mut body).unwrap();
    Some(body)
}
fn write_frame(stream: &mut UnixStream, body: &[u8]) {
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(body).unwrap();
}
