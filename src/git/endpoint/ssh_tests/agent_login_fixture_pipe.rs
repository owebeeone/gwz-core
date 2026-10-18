//! The Windows twin of [`super::agent_fixture`]'s `Fixture` (plan step 3.4): the loopback `sshd.exe`, and an agent on
//! a named pipe ([`super::agent_fixture_pipe::PipeAgent`]) that lists two keys, one the server rejects and one it
//! trusts, records each list and sign request, and can stall or damage a signature. It serves the login rows of the
//! endpoint (`agent_auth`, `network_agent`), which sign through `agent_auth::authenticate_reporting`.
//!
//! It never contacts the user's agent. The Unix fixture is a proxy in front of a private `ssh-agent`; this one signs
//! itself, with the keys the test makes (`PipeAgent`).
use super::agent_fixture_pipe::{
    Behavior, Damage, Fault, Identity, PipeAgent, PipeChannel, Request, pipe_name,
};
use crate::git::endpoint::{
    agent_client::Agent,
    agent_job::Control,
    agent_source::{AgentSource, PipeName},
    fixture_host,
    ssh_fixture::{self as common, SshConnection, SshdFixture},
};
use std::{
    fs,
    io::{self, Read},
    net::TcpStream,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// The key type and sign algorithm the endpoint rows use by default. libssh2 on Windows reads only RSA PEM key files
/// (step 3.8), and the rows that pass the fixture's key as a selected key need one, so the agent holds RSA keys and
/// signs at SHA-512, the one flag libssh2 on Windows offers.
pub const METHOD: &str = "rsa-sha2-512";

pub struct Fixture {
    pub ssh: SshdFixture,
    /// The agent's pipe name.
    pub path: String,
    agent: PipeAgent,
    monitor: Mutex<Option<TcpStream>>,
}

impl Fixture {
    /// A server and an agent for `method`, a key type's name as the sign algorithm names it: `ssh-ed25519`,
    /// `rsa-sha2-256` or `rsa-sha2-512`. `stall` makes the agent answer no sign request.
    pub fn new(method: &str, stall: bool) -> Self {
        Self::start(method, stall, true)
    }

    /// A server and a reachable agent that holds no key at all.
    pub fn without_keys(method: &str) -> Self {
        Self::start(method, false, false)
    }

    fn start(method: &str, stall: bool, loaded: bool) -> Self {
        let ssh = SshdFixture::new();
        let key = ssh.temp.path().join("auth_key");
        let bad = ssh.temp.path().join("rejected_key");
        for path in [&key, &bad] {
            let rsa = method.starts_with("rsa");
            let mut generate = Command::new(fixture_host::programs().keygen);
            generate.args(["-q", "-t", if rsa { "rsa" } else { "ed25519" }]);
            if rsa {
                // The only form libssh2 on Windows reads; the agent signs it from the file.
                generate.args(["-b", "2048", "-m", "PEM"]);
            }
            generate.args(["-N", ""]).arg("-f").arg(path);
            common::run(&mut generate);
        }
        fs::copy(
            key.with_extension("pub"),
            ssh.temp.path().join("authorized_keys"),
        )
        .unwrap();
        // The rejected key is listed first, as the Unix fixture lists it.
        let entries = [&bad, &key]
            .into_iter()
            .filter(|_| loaded)
            .map(|path| (Identity::from_key_file(path), Behavior::Signs))
            .collect();
        let path = pipe_name(ssh.temp.path());
        let agent = PipeAgent::start(&path, entries).unwrap();
        if stall {
            agent.script(Fault::Stall);
        }
        Self {
            ssh,
            path,
            agent,
            monitor: Mutex::new(None),
        }
    }

    /// The agent source a runtime would choose for this agent: the pipe it serves on.
    pub fn source(&self) -> AgentSource {
        AgentSource::NamedPipe(
            PipeName::parse(std::ffi::OsStr::new(&self.path))
                .expect("the fixture's pipe name is a local pipe"),
        )
    }

    /// The agent's answers as a function of the request, for a Pageant that holds these keys (plan step 3.6).
    pub fn responder(&self) -> impl Fn(&[u8]) -> Vec<u8> + Send + Sync + 'static {
        self.agent.responder()
    }

    /// Whether the agent has been asked to sign.
    pub fn signed(&self) -> bool {
        self.agent
            .requests()
            .iter()
            .any(|request| matches!(request, Request::Sign { .. }))
    }

    /// A handshaken connection to the server and the host key it presented, trusted through the fixture's
    /// `known_hosts`, set to sign with `method`.
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

    /// The agent was listed once and asked `signatures` times, with the flags `method` asks for, all for the key the
    /// server trusts (the rejected key is probed without a signature and never signs).
    pub fn assert_requests(&self, method: &str, signatures: usize) {
        let flags = match method {
            "rsa-sha2-256" => 2,
            "rsa-sha2-512" => 4,
            _ => 0,
        };
        let mut expected = vec![Request::List];
        expected.extend(std::iter::repeat_n(
            Request::Sign { key: 1, flags },
            signatures,
        ));
        assert_eq!(self.agent.requests(), expected);
    }

    pub fn no_requests(&self) {
        assert!(self.agent.requests().is_empty());
    }

    /// Waits for the agent to have been asked to sign.
    pub fn wait_signing(&self, within: Duration) {
        self.wait_for(within, |requests| {
            requests
                .iter()
                .any(|request| matches!(request, Request::Sign { .. }))
        });
    }

    /// Waits for the agent to have been asked for its keys.
    pub fn wait_listed(&self, within: Duration) {
        self.wait_for(within, |requests| requests.contains(&Request::List));
    }

    fn wait_for(&self, within: Duration, done: impl Fn(&[Request]) -> bool) {
        let until = Instant::now() + within;
        while !done(&self.agent.requests()) {
            assert!(Instant::now() < until, "the agent was not asked in time");
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Damages the next signature: 1 is a short signature, 2 a wrong algorithm name, 3 a one-byte `ssh-rsa` and 4 a
    /// 257-byte one (the Unix fixture's codes).
    pub fn damage(&self, kind: u8) {
        let damage = match kind {
            1 => Damage::ShortSignature,
            2 => Damage::WrongAlgorithm,
            3 => Damage::OneByteRsa,
            4 => Damage::OversizeRsa,
            other => panic!("no damage {other}"),
        };
        self.agent.script(Fault::Damage(damage));
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

    /// Waits for the last client of the agent to have left.
    pub fn wait_closed(&self) {
        assert!(
            self.agent.wait_client_closed(Duration::from_secs(2)),
            "the agent's client did not leave"
        );
    }

    /// A raw client of the agent, for a test that wraps its own channel.
    pub fn raw_channel(&self) -> PipeChannel {
        self.agent.connect()
    }
}

/// An agent client over the pipe named `path`, for `agent_auth`'s `open` argument.
pub fn connect(path: &str, control: Arc<Control>) -> io::Result<Agent<PipeChannel>> {
    PipeChannel::open(path).map(|channel| Agent::new(channel, control))
}

/// Whether a read shows the socket's connection ended: EOF, or a reset or abort, as Windows reads a connection whose
/// peer closed with bytes it never read. A handle left open reads neither: the read times out, so each caller sets a
/// timeout.
pub fn reads_closed(read: &io::Result<usize>) -> bool {
    match read {
        Ok(count) => *count == 0,
        Err(error) => matches!(
            error.kind(),
            io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
        ),
    }
}
