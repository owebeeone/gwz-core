//! Step 1.6's fixture rows: a connection another operation binding last used is leased only after this binding proves
//! the connection's authority now (contract §7.1; dev-docs/GwzConnectionReuseDesign.md §4), and the proof's own rows
//! (its exact signed bytes, OpenSSH's `ssh-keygen -Y sign` as the oracle of the SSHSIG format and of the verifier, and
//! the agent's possession of each key type). The server is the loopback `sshd` fixture. The agent is the platform's
//! private one, stopped when the row ends: on Unix a private `ssh-agent` on its own socket in the fixture's temporary
//! directory (stopped by its process), behind a counting proxy that can forge one signature; on Windows the pipe agent
//! (`agent_fixture_pipe`), whose keys are fixed when it starts, so a key it "no longer holds" is its next sign request
//! refused. Nothing here reads or contacts the user's agent (`SSH_AUTH_SOCK` is never read), keys or `known_hosts`.
use super::{attachment, local_endpoint};
use crate::git::endpoint::{
    agent_source::AgentSource,
    fixture_host,
    ssh_channel::GitService,
    ssh_fixture::{self as common, SshdFixture},
    ssh_worker::{BridgeContext, Endpoint},
    stream_io::BlockingStream,
};
use gwz_transport::{
    pool::{Config, Key},
    protocol::{ErrorCode, Opened},
};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::net::{UnixListener, UnixStream};
        use std::{
            process::{Child, Stdio},
            sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering},
        };

        /// A private `ssh-agent` holding the row's keys, and a proxy socket in front of it that counts sign requests
        /// and, when asked, damages the signature of the next one.
        struct Agent {
            child: Child,
            real: PathBuf,
            path: PathBuf,
            signs: Arc<AtomicUsize>,
            forge: Arc<AtomicU8>,
            delay: Arc<AtomicU64>,
            stop: Arc<AtomicBool>,
            accept: Option<thread::JoinHandle<()>>,
        }
        impl Agent {
            /// The agent in `dir`, holding `keys`, added in order.
            fn start(dir: &Path, keys: &[&Path]) -> Self {
                let real = dir.join("reuse-real-agent.sock");
                let path = dir.join("reuse-proxy-agent.sock");
                let child = Command::new("ssh-agent")
                    .args(["-D", "-a"])
                    .arg(&real)
                    .env_remove("SSH_AUTH_SOCK")
                    .env_remove("SSH_AGENT_PID")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let until = Instant::now() + Duration::from_secs(5);
                while UnixStream::connect(&real).is_err() {
                    assert!(Instant::now() < until, "the private agent never bound its socket");
                    thread::sleep(Duration::from_millis(2));
                }
                let listener = UnixListener::bind(&path).unwrap();
                listener.set_nonblocking(true).unwrap();
                let signs = Arc::new(AtomicUsize::new(0));
                let forge = Arc::new(AtomicU8::new(0));
                let delay = Arc::new(AtomicU64::new(0));
                let stop = Arc::new(AtomicBool::new(false));
                let accept = {
                    let (real, signs, forge, delay, stop) =
                        (real.clone(), signs.clone(), forge.clone(), delay.clone(), stop.clone());
                    thread::spawn(move || {
                        while !stop.load(Ordering::SeqCst) {
                            match listener.accept() {
                                Ok((peer, _)) => {
                                    let (real, signs, forge, delay) =
                                        (real.clone(), signs.clone(), forge.clone(), delay.clone());
                                    thread::spawn(move || relay(peer, &real, &signs, &forge, &delay));
                                }
                                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                    thread::sleep(Duration::from_millis(2));
                                }
                                Err(error) => panic!("agent proxy accept: {error}"),
                            }
                        }
                    })
                };
                let agent = Self { child, real, path, signs, forge, delay, stop, accept: Some(accept) };
                for key in keys {
                    agent.ssh_add(&[key.as_os_str()]);
                }
                agent
            }
            fn source(&self) -> AgentSource {
                AgentSource::from_path(Some(self.path.clone()))
            }
            fn ssh_add(&self, args: &[&std::ffi::OsStr]) {
                common::run(
                    Command::new("ssh-add")
                        .env("SSH_AUTH_SOCK", &self.real)
                        .env_remove("SSH_AGENT_PID")
                        .args(args),
                );
            }
            /// The agent no longer holds `key`.
            fn withdraw(&self, key: &Path) {
                let public = key.with_extension("pub");
                self.ssh_add(&["-d".as_ref(), public.as_os_str()]);
            }
            /// The next signature the agent returns no longer verifies.
            fn forge_next(&self) {
                self.forge.store(1, Ordering::SeqCst);
            }
            /// The next signature the agent returns is named `ssh-rsa`, as an agent that answers a `rsa-sha2-512`
            /// request with a SHA-1 signature does.
            fn downgrade_next(&self) {
                self.forge.store(2, Ordering::SeqCst);
            }
            /// The agent answers the next sign request only after `ms`.
            fn delay_next(&self, ms: u64) {
                self.delay.store(ms, Ordering::SeqCst);
            }
            fn signs(&self) -> usize {
                self.signs.load(Ordering::SeqCst)
            }
        }
        impl Drop for Agent {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::SeqCst);
                if let Some(accept) = self.accept.take() {
                    let _ = accept.join();
                }
                // Stopped by its own process (its PID), never by name.
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
        fn frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
            let mut header = [0; 4];
            stream.read_exact(&mut header).ok()?;
            let len = u32::from_be_bytes(header) as usize;
            assert!((1..=1 << 20).contains(&len));
            let mut body = vec![0; len];
            stream.read_exact(&mut body).ok()?;
            Some(body)
        }
        fn send(stream: &mut UnixStream, body: &[u8]) -> io::Result<()> {
            stream.write_all(&(body.len() as u32).to_be_bytes())?;
            stream.write_all(body)
        }
        fn relay(mut peer: UnixStream, real: &Path, signs: &AtomicUsize, forge: &AtomicU8, delay: &AtomicU64) {
            // An accepted socket inherits the listener's non-blocking mode on macOS.
            if peer.set_nonblocking(false).is_err() {
                return;
            }
            let Ok(mut upstream) = UnixStream::connect(real) else {
                return;
            };
            while let Some(body) = frame(&mut peer) {
                let signing = body[0] == 13;
                if signing {
                    signs.fetch_add(1, Ordering::SeqCst);
                }
                if send(&mut upstream, &body).is_err() {
                    return;
                }
                let Some(mut response) = frame(&mut upstream) else {
                    return;
                };
                if signing {
                    let ms = delay.swap(0, Ordering::SeqCst);
                    if ms > 0 {
                        thread::sleep(Duration::from_millis(ms));
                    }
                }
                if signing && response[0] == 14 {
                    match forge.swap(0, Ordering::SeqCst) {
                        // The signature's last byte: a well-formed reply that no longer verifies.
                        1 => {
                            let last = response.len() - 1;
                            response[last] ^= 0x01;
                        }
                        // The signature renamed `ssh-rsa`, its bytes kept.
                        2 => {
                            let blob = &response[5..];
                            let name = u32::from_be_bytes(blob[..4].try_into().unwrap()) as usize;
                            let raw = blob[4 + name..].to_vec();
                            let mut signature = Vec::new();
                            signature.extend_from_slice(&7u32.to_be_bytes());
                            signature.extend_from_slice(b"ssh-rsa");
                            signature.extend_from_slice(&raw);
                            response = vec![14];
                            response.extend_from_slice(&(signature.len() as u32).to_be_bytes());
                            response.extend_from_slice(&signature);
                        }
                        _ => {}
                    }
                }
                if send(&mut peer, &response).is_err() {
                    return;
                }
            }
        }
        /// The type of the rows' agent keys: Ed25519.
        const ROW_KEY: &[&str] = &["-t", "ed25519"];
    } else {
        use super::agent_fixture_pipe::{Behavior, Damage, Fault, Identity, PipeAgent, Request, pipe_name};

        /// The pipe agent holding the row's keys, which counts sign requests and can refuse or damage the next one.
        struct Agent {
            agent: PipeAgent,
        }
        impl Agent {
            /// The agent in `dir`, listing `keys` in order.
            fn start(dir: &Path, keys: &[&Path]) -> Self {
                let entries = keys
                    .iter()
                    .map(|key| (Identity::from_key_file(key), Behavior::Signs))
                    .collect();
                Self { agent: PipeAgent::start(&pipe_name(dir), entries).unwrap() }
            }
            fn source(&self) -> AgentSource {
                use crate::git::endpoint::agent_source::PipeName;
                AgentSource::NamedPipe(
                    PipeName::parse(std::ffi::OsStr::new(self.agent.name()))
                        .expect("the fixture's pipe name is a local pipe"),
                )
            }
            /// The agent's keys are fixed when it starts: it refuses its next sign request, as an agent that no longer
            /// holds the key does.
            fn withdraw(&self, _key: &Path) {
                self.agent.script(Fault::Refuse);
            }
            /// The next signature the agent returns no longer verifies.
            fn forge_next(&self) {
                self.agent.script(Fault::Damage(Damage::WrongAlgorithm));
            }
            /// The next signature the agent returns is named `ssh-rsa` (the pipe agent's `OneByteRsa`), as an agent
            /// that answers a `rsa-sha2-512` request with a SHA-1 signature names it.
            fn downgrade_next(&self) {
                self.agent.script(Fault::Damage(Damage::OneByteRsa));
            }
            /// The agent answers the next sign request only after `ms`.
            fn delay_next(&self, ms: u64) {
                self.agent.script(Fault::Delay(ms));
            }
            fn signs(&self) -> usize {
                self.agent
                    .requests()
                    .iter()
                    .filter(|request| matches!(request, Request::Sign { .. }))
                    .count()
            }
        }
        /// The type of the rows' agent keys: RSA, the agent key type libssh2 on Windows (CNG) signs with, in the PEM
        /// form the pipe agent signs from.
        const ROW_KEY: &[&str] = &["-t", "rsa", "-b", "2048", "-m", "PEM"];
    }
}

/// A new key at `path` (and `path.pub`) of the type `kind` names, by the fixture host's `ssh-keygen`.
fn keygen_kind(path: &Path, kind: &[&str]) {
    common::run(
        Command::new(fixture_host::programs().keygen)
            .args(["-q", "-N", ""])
            .args(kind)
            .arg("-f")
            .arg(path),
    );
}

/// A server that accepts two fresh agent keys, and the agent holding both, the first added first, so the server
/// accepts the first.
struct Rig {
    ssh: SshdFixture,
    agent: Agent,
    first: PathBuf,
    second: PathBuf,
    endpoint: Endpoint,
}
fn config() -> Config {
    Config {
        total: 4,
        per_host: 4,
        per_user_host: 4,
        cleanup_timeout_ms: 500,
        ..Config::default()
    }
}
impl Rig {
    fn new() -> Self {
        Self::with_keys(ROW_KEY)
    }
    /// The rig with agent keys of the type `kind` names.
    fn with_keys(kind: &[&str]) -> Self {
        Self::with_config(kind, config())
    }
    /// The rig with agent keys of the type `kind` names and the pool `config`.
    fn with_config(kind: &[&str], pool: Config) -> Self {
        let ssh = SshdFixture::with_config("");
        let first = ssh.temp.path().join("reuse_first");
        let second = ssh.temp.path().join("reuse_second");
        keygen_kind(&first, kind);
        keygen_kind(&second, kind);
        let mut authorized = fs::read_to_string(first.with_extension("pub")).unwrap();
        authorized.push_str(&fs::read_to_string(second.with_extension("pub")).unwrap());
        fs::write(ssh.temp.path().join("authorized_keys"), authorized).unwrap();
        let agent = Agent::start(ssh.temp.path(), &[&first, &second]);
        let endpoint = local_endpoint::connect(pool, ssh.known_hosts.clone(), agent.source(), 3000);
        Self {
            ssh,
            agent,
            first,
            second,
            endpoint,
        }
    }
    fn key(&self) -> Key {
        Key::ssh(&self.ssh.user, "127.0.0.1", self.ssh.port)
    }
    /// One upload-pack exchange to completion for the operation binding `binding` (its bridge's session), with the
    /// selected key file `selected` or the agent.
    fn exchange_selected(&self, binding: &str, selected: Option<PathBuf>) -> io::Result<Opened> {
        let context = BridgeContext {
            session_id: binding.into(),
            stream_id: 1,
            version: 2,
            limits: gwz_transport::binding::default_limits(),
            deadlines: attachment::deadlines(&config(), 3000),
            waker: None,
        };
        let pending = self.endpoint.start_endpoint_open(
            self.key(),
            selected,
            GitService::UploadPack,
            self.ssh.repository.to_str().unwrap(),
            context.clone(),
            Arc::new(AtomicBool::new(false)),
        )?;
        let (attachment, opened) = attachment::finish(&pending)?;
        drain(attachment::drive(attachment, &context))?;
        Ok(opened)
    }
    fn exchange(&self, binding: &str) -> io::Result<Opened> {
        self.exchange_selected(binding, None)
    }
}
fn drain(mut stream: BlockingStream) -> io::Result<()> {
    stream.write_all(b"0000")?;
    stream.end_write()?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    assert!(!bytes.is_empty());
    stream.close().map(drop)
}

#[test]
fn p1_6_the_same_binding_reuses_its_connection_without_asking_the_agent_again() {
    let rig = Rig::new();
    let first = rig
        .exchange("binding-a")
        .unwrap_or_else(|e| panic!("{e:?}\n{}", rig.ssh.log()));
    let signs = rig.agent.signs();
    let second = rig.exchange("binding-a").unwrap();
    assert!(!first.reused);
    assert!(second.reused);
    assert_eq!(first.connection_id, second.connection_id);
    assert_eq!(
        rig.agent.signs(),
        signs,
        "within one binding reuse asks the agent nothing"
    );
}

#[test]
fn p1_6_another_binding_proves_possession_once_then_reuses() {
    let rig = Rig::new();
    let first = rig.exchange("binding-a").unwrap();
    let signs = rig.agent.signs();
    let other = rig.exchange("binding-b").unwrap();
    assert!(other.reused, "a proven connection serves the other binding");
    assert_eq!(first.connection_id, other.connection_id);
    assert_eq!(
        rig.agent.signs(),
        signs + 1,
        "exactly one fresh possession signature"
    );
    let again = rig.exchange("binding-b").unwrap();
    assert!(again.reused);
    assert_eq!(
        rig.agent.signs(),
        signs + 1,
        "the binding's proof is kept for its life"
    );
}

#[test]
fn p1_6_a_key_removed_from_the_agent_gives_the_other_binding_a_fresh_connection() {
    let rig = Rig::new();
    let first = rig.exchange("binding-a").unwrap();
    rig.agent.withdraw(&rig.first);
    let other = rig.exchange("binding-b").unwrap();
    assert!(!other.reused, "withdrawn authority is not reused");
    assert_ne!(first.connection_id, other.connection_id);
    // The new connection authenticated with the key the agent still holds.
    let _ = &rig.second;
}

impl Rig {
    /// Two exchanges of `binding` at once, on two connections, both run to completion.
    fn two_at_once(&self, binding: &str) -> (Opened, Opened) {
        let context = BridgeContext {
            session_id: binding.into(),
            stream_id: 1,
            version: 2,
            limits: gwz_transport::binding::default_limits(),
            deadlines: attachment::deadlines(&config(), 3000),
            waker: None,
        };
        let start = || {
            self.endpoint
                .start_endpoint_open(
                    self.key(),
                    None,
                    GitService::UploadPack,
                    self.ssh.repository.to_str().unwrap(),
                    context.clone(),
                    Arc::new(AtomicBool::new(false)),
                )
                .unwrap()
        };
        let (first, second) = (start(), start());
        let (one, opened_one) = attachment::finish(&first).unwrap();
        let (two, opened_two) = attachment::finish(&second).unwrap();
        let (one, two) = (
            attachment::drive(one, &context),
            attachment::drive(two, &context),
        );
        drain(one).unwrap();
        drain(two).unwrap();
        (opened_one, opened_two)
    }
    /// Waits until the endpoint's pool counts satisfy `condition`, and says what they were when it gave up.
    fn wait_counts(&self, what: &str, condition: impl Fn(gwz_transport::pool::Counts) -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let counts = self.endpoint.pool().counts();
            if condition(counts) {
                return;
            }
            assert!(
                Instant::now() < until,
                "{what}: the pool stayed at {counts:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn p1_6_a_failed_possession_proof_retires_the_idle_connections_of_that_key() {
    let rig = Rig::new();
    let (one, two) = rig.two_at_once("binding-a");
    assert_ne!(one.connection_id, two.connection_id);
    rig.wait_counts("binding A's two connections are idle", |counts| {
        counts.idle == 2 && counts.total() == 2
    });
    rig.agent.withdraw(&rig.first);
    let other = rig.exchange("binding-b").unwrap();
    assert!(!other.reused);
    // The connection the proof failed for and the other idle one with that key are both disposed of: what is left is
    // binding B's own new connection.
    rig.wait_counts("only binding B's connection is left", |counts| {
        counts.idle == 1 && counts.total() == 1
    });
}

#[test]
fn p1_6_a_forged_possession_signature_is_no_proof() {
    let rig = Rig::new();
    let first = rig.exchange("binding-a").unwrap();
    rig.agent.forge_next();
    let other = rig.exchange("binding-b").unwrap();
    assert!(
        !other.reused,
        "a signature that does not verify proves nothing"
    );
    assert_ne!(first.connection_id, other.connection_id);
}

#[test]
fn p1_6_a_changed_known_hosts_entry_refuses_the_other_binding() {
    let rig = Rig::new();
    rig.exchange("binding-a").unwrap();
    let trusted = fs::read_to_string(&rig.ssh.known_hosts).unwrap();
    // The same host listed with another key.
    let impostor = rig.ssh.temp.path().join("impostor_host");
    let trusted_kind = trusted.split_whitespace().nth(1).unwrap();
    keygen_kind(
        &impostor,
        match trusted_kind {
            "ssh-rsa" => &["-t", "rsa", "-b", "2048"],
            "ecdsa-sha2-nistp256" => &["-t", "ecdsa", "-b", "256"],
            _ => &["-t", "ed25519"],
        },
    );
    let other_key = fs::read_to_string(impostor.with_extension("pub")).unwrap();
    let mut fields = other_key.split_whitespace();
    let (kind, blob) = (fields.next().unwrap(), fields.next().unwrap());
    let (host, _) = trusted.split_once(' ').unwrap();
    fs::write(&rig.ssh.known_hosts, format!("{host} {kind} {blob}\n")).unwrap();
    let refused = rig.exchange("binding-b").unwrap_err();
    assert_eq!(local_endpoint::failure(&refused).code, ErrorCode::Trust);
    // Restored, a later binding is served again (on a new connection: the refused one was retired).
    fs::write(&rig.ssh.known_hosts, &trusted).unwrap();
    let restored = rig.exchange("binding-c").unwrap();
    assert!(!restored.reused);
}

#[test]
fn p1_6_a_changed_selected_key_file_is_never_reused_by_another_binding() {
    let rig = Rig::new();
    let selected = rig.ssh.temp.path().join("selected_copy");
    fs::copy(&rig.first, &selected).unwrap();
    let first = rig
        .exchange_selected("binding-a", Some(selected.clone()))
        .unwrap();
    // The same bytes, read again, for another binding: reused.
    let same = rig
        .exchange_selected("binding-b", Some(selected.clone()))
        .unwrap();
    assert!(same.reused);
    assert_eq!(first.connection_id, same.connection_id);
    // Other key bytes at that path: never that connection.
    fs::copy(&rig.second, &selected).unwrap();
    let changed = rig.exchange_selected("binding-c", Some(selected)).unwrap();
    assert!(!changed.reused);
    assert_ne!(first.connection_id, changed.connection_id);
}

// The proof's own rows: its exact signed bytes, OpenSSH as the oracle of the SSHSIG format and of the verifier, and
// the agent's possession of each key type the transport authenticates with.

use crate::git::endpoint::{
    agent_auth::{self, Possession},
    agent_job::Job,
    agent_keys::{KeyType, Proof},
};

fn finish_job<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
    use std::task::{Context, Poll, Waker};
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
            return result;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
}
fn ssh_string(out: &mut Vec<u8>, part: &[u8]) {
    out.extend_from_slice(&(part.len() as u32).to_be_bytes());
    out.extend_from_slice(part);
}
fn take<'a>(input: &mut &'a [u8]) -> &'a [u8] {
    let len = u32::from_be_bytes(input[..4].try_into().unwrap()) as usize;
    let (value, rest) = input[4..].split_at(len);
    *input = rest;
    value
}

#[test]
fn p1_6_the_proof_signs_exactly_the_sshsig_data_of_the_nonce_digest() {
    let digest: [u8; 64] = std::array::from_fn(|i| i as u8);
    let mut expected = b"SSHSIG".to_vec();
    ssh_string(
        &mut expected,
        b"gwz-connection-reuse-v1@owebeeone.github.io",
    );
    ssh_string(&mut expected, b"");
    ssh_string(&mut expected, b"sha512");
    ssh_string(&mut expected, &digest);
    assert_eq!(agent_auth::proof_data(&digest), expected);
    assert_eq!(expected.len(), 6 + 4 + 43 + 4 + 4 + 6 + 4 + 64);
    // Never parseable as SSH user-authentication data, which starts with the session identifier's length.
    assert_eq!(&expected[..6], b"SSHSIG");
}

/// `ssh-keygen -Y sign` signs `message` under the proof's namespace; the row rebuilds the signed data with
/// `proof_data` and checks the signature with the transport's verifier, so both match OpenSSH's PROTOCOL.sshsig.
fn openssh_signed(dir: &Path, kind: &[&str], name: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use sha2::{Digest, Sha512};
    let key = dir.join(name);
    keygen_kind(&key, kind);
    let message = dir.join(format!("{name}.message"));
    fs::write(&message, format!("reuse proof oracle for {name}")).unwrap();
    common::run(
        Command::new(fixture_host::programs().keygen)
            .args(["-Y", "sign", "-n", agent_auth::PROOF_NAMESPACE, "-f"])
            .arg(&key)
            .arg(&message),
    );
    let armored = fs::read_to_string(message.with_extension("message.sig")).unwrap();
    let body: String = armored
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let blob = STANDARD.decode(body).unwrap();
    let mut input = &blob[..];
    assert_eq!(&input[..6], b"SSHSIG");
    input = &input[10..]; // preamble and version
    let public = take(&mut input).to_vec();
    assert_eq!(take(&mut input), agent_auth::PROOF_NAMESPACE.as_bytes());
    assert_eq!(take(&mut input), b"");
    assert_eq!(take(&mut input), b"sha512");
    let signature = take(&mut input).to_vec();
    let digest: [u8; 64] = Sha512::digest(fs::read(&message).unwrap()).into();
    (public, agent_auth::proof_data(&digest), signature)
}

#[test]
fn p1_6_openssh_signatures_of_the_proof_data_verify_and_damaged_ones_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let cases: &[(&[&str], &str)] = &[
        (&["-t", "ed25519"], "ed25519"),
        (&["-t", "ecdsa", "-b", "256"], "ecdsa256"),
        (&["-t", "ecdsa", "-b", "384"], "ecdsa384"),
        (&["-t", "rsa", "-b", "3072"], "rsa3072"),
    ];
    for (kind, name) in cases {
        let (public, data, signature) = openssh_signed(dir.path(), kind, name);
        let key = KeyType::of(&public).unwrap();
        assert_eq!(
            key.verify_proof(&public, &data, &signature),
            Proof::Valid,
            "{name}"
        );
        let mut damaged = signature.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 0x01;
        assert_eq!(
            key.verify_proof(&public, &data, &damaged),
            Proof::Invalid,
            "{name}"
        );
        let mut other = data.clone();
        let last = other.len() - 1;
        other[last] ^= 0x01;
        assert_eq!(
            key.verify_proof(&public, &other, &signature),
            Proof::Invalid,
            "{name}"
        );
    }
    // A key the transport cannot check proves nothing.
    let (public, data, signature) =
        openssh_signed(dir.path(), &["-t", "ecdsa", "-b", "521"], "ecdsa521");
    assert_eq!(
        KeyType::of(&public)
            .unwrap()
            .verify_proof(&public, &data, &signature),
        Proof::Unsupported
    );
}

#[test]
fn p1_6_the_agent_proves_possession_of_each_key_type_it_holds_and_not_of_one_it_lacks() {
    let dir = tempfile::tempdir().unwrap();
    let cases: &[(&[&str], &str, Possession)] = &[
        (&["-t", "ed25519"], "ed25519", Possession::Proven),
        (
            &["-t", "ecdsa", "-b", "256"],
            "ecdsa256",
            Possession::Proven,
        ),
        (
            &["-t", "ecdsa", "-b", "384"],
            "ecdsa384",
            Possession::Proven,
        ),
        (
            &["-t", "rsa", "-b", "2048", "-m", "PEM"],
            "rsa2048",
            Possession::Proven,
        ),
        (
            &["-t", "ecdsa", "-b", "521"],
            "ecdsa521",
            Possession::Unprovable,
        ),
        // C-P3-2: an RSA key outside 2048 to 8192 bits is never sent to the agent.
        (
            &["-t", "rsa", "-b", "1024", "-m", "PEM"],
            "rsa1024",
            Possession::Unprovable,
        ),
        (&["-t", "ed25519"], "forged", Possession::Refused),
    ];
    let keys: Vec<PathBuf> = cases
        .iter()
        .map(|(kind, name, _)| {
            let key = dir.path().join(name);
            keygen_kind(&key, kind);
            key
        })
        .collect();
    let agent = Agent::start(
        dir.path(),
        &keys.iter().map(PathBuf::as_path).collect::<Vec<_>>(),
    );
    let prove = |public: Vec<u8>| {
        let source = agent.source();
        let mut job = Job::start_isolated(
            Some(Instant::now() + Duration::from_secs(20)),
            Duration::from_secs(1),
            move |control| {
                let mut client = source.open(control.clone())?;
                agent_auth::prove_possession(&mut client, &public, &control)
            },
        )
        .unwrap();
        finish_job(&mut job).unwrap()
    };
    let public_of = |key: &Path| {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let line = fs::read_to_string(key.with_extension("pub")).unwrap();
        STANDARD
            .decode(line.split_whitespace().nth(1).unwrap())
            .unwrap()
    };
    for ((_, name, expected), key) in cases.iter().zip(&keys) {
        if *name == "forged" {
            agent.forge_next();
        }
        let signs = agent.signs();
        assert_eq!(prove(public_of(key)), *expected, "{name}");
        // A key no binding can prove is never sent to the agent.
        let asked = usize::from(*expected != Possession::Unprovable);
        assert_eq!(agent.signs(), signs + asked, "{name}");
        if *expected == Possession::Proven {
            agent.withdraw(key);
            assert_eq!(
                prove(public_of(key)),
                Possession::Refused,
                "{name} once withdrawn"
            );
        }
    }
}

// Step 1.6 remediation round 1 (dev-docs/GwzConnectionApiContract-Step1.6-RemPlan.md).

impl Rig {
    /// An open of `binding` submitted with a cancellation flag the row sets.
    fn start_cancellable(
        &self,
        binding: &str,
    ) -> (
        crate::git::endpoint::ssh_worker::PendingOpen,
        Arc<AtomicBool>,
    ) {
        let context = BridgeContext {
            session_id: binding.into(),
            stream_id: 1,
            version: 2,
            limits: gwz_transport::binding::default_limits(),
            deadlines: attachment::deadlines(&config(), 3000),
            waker: None,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let pending = self
            .endpoint
            .start_endpoint_open(
                self.key(),
                None,
                GitService::UploadPack,
                self.ssh.repository.to_str().unwrap(),
                context,
                cancelled.clone(),
            )
            .unwrap();
        (pending, cancelled)
    }
}

#[test]
fn p1_6_rem_a_bounced_connection_expires_from_its_last_real_release() {
    // S-P2-2: A's connection authenticated with a P-521 agent key, which no other binding can prove. Bouncing it off
    // B's open (B gets a connection of its own) never restarts its idle clock: it expires the idle time after A
    // released it, while B's, released later, is still idle.
    let idle = Duration::from_millis(3000);
    let rig = Rig::with_config(
        &["-t", "ecdsa", "-b", "521"],
        Config {
            idle_timeout_ms: idle.as_millis() as u64,
            ..config()
        },
    );
    let a = rig.exchange("binding-a").unwrap();
    let released = Instant::now();
    thread::sleep(Duration::from_millis(1000));
    let b = rig.exchange("binding-b").unwrap();
    assert!(!b.reused);
    assert_ne!(a.connection_id, b.connection_id);
    // Past A's idle time from its release, before it from the bounce.
    let check = released + idle + Duration::from_millis(400);
    thread::sleep(check.saturating_duration_since(Instant::now()));
    let counts = rig.endpoint.pool().counts();
    assert_eq!(
        (counts.idle, counts.total()),
        (1, 1),
        "A's connection expired from its last real release; B's is idle ({counts:?})"
    );
}

#[test]
fn p1_6_rem_an_agent_that_answers_ssh_rsa_is_unprovable_and_keeps_the_connections() {
    // C-P3-1: the agent answers B's `rsa-sha2-512` proof request with an `ssh-rsa` signature. That proves nothing, and
    // revokes nothing: A's connection survives and B gets its own.
    let rig = Rig::with_keys(&["-t", "rsa", "-b", "2048", "-m", "PEM"]);
    let a = rig.exchange("binding-a").unwrap();
    rig.agent.downgrade_next();
    let b = rig.exchange("binding-b").unwrap();
    assert!(!b.reused);
    assert_ne!(a.connection_id, b.connection_id);
    rig.wait_counts("A's connection survives beside B's", |counts| {
        counts.idle == 2 && counts.total() == 2
    });
}

#[test]
fn p1_6_rem_a_proof_outlives_its_cancelled_waiter_and_is_kept_for_the_binding() {
    // C-P3-5 = S-P3-2: B's only waiter is cancelled while the agent is still answering B's proof. The job runs on,
    // passes, and is recorded for B: B's next lease of A's connection asks the agent nothing.
    let rig = Rig::new();
    let a = rig.exchange("binding-a").unwrap();
    let signs = rig.agent.signs();
    rig.agent.delay_next(800);
    let (pending, cancelled) = rig.start_cancellable("binding-b");
    let until = Instant::now() + Duration::from_secs(10);
    while rig.agent.signs() == signs {
        assert!(Instant::now() < until, "B's proof never reached the agent");
        thread::sleep(Duration::from_millis(2));
    }
    cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        attachment::finish(&pending).is_err(),
        "B's cancelled open fails"
    );
    // The proof's answer arrives after the cancellation.
    thread::sleep(Duration::from_millis(1200));
    let b = rig.exchange("binding-b").unwrap();
    assert!(
        b.reused,
        "B reuses A's connection on the proof its cancelled open started"
    );
    assert_eq!(b.connection_id, a.connection_id);
    assert_eq!(
        rig.agent.signs(),
        signs + 1,
        "one sign for B: the proof that outlived its waiter"
    );
}
