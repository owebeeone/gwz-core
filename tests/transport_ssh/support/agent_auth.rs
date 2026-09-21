use crate::{
    common::{self, SshConnection, SshdFixture},
    *,
};
use std::os::unix::net::{UnixListener, UnixStream};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread,
};
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
        let deadline = Instant::now() + Duration::from_secs(3);
        while !real.exists() {
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
                    match damage.load(Ordering::SeqCst) {
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
                        _ => {}
                    }
                }
                if body[0] == 11 {
                    assert_eq!(u32::from_be_bytes(response[1..5].try_into().unwrap()), 2);
                }
                write_frame(&mut peer, &response);
                if body[0] == 11 { let _ = list.send(()); }
            }
        }));
        fixture
    }
    pub fn prepared(&self, method: &str) -> (SshConnection, Vec<u8>) {
        let stream = TcpStream::connect(("127.0.0.1", self.ssh.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        *self.monitor.lock().unwrap() = Some(stream.try_clone().unwrap());
        let mut connection = SshConnection::new(stream).unwrap();
        let session = connection.session();
        session.set_timeout(3_000);
        session.handshake().unwrap();
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
    pub fn assert_tcp_closed(&self) {
        let mut guard = self.monitor.lock().unwrap();
        let socket = guard.as_mut().unwrap();
        assert_eq!(
            socket.read(&mut [0; 1]).unwrap(),
            0,
            "connection owner must shut down TCP before disposal ack"
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
