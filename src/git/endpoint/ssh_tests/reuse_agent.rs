//! The private agent of the reuse rows (plan step 1.6): the platform's own, stopped when the row ends. On Unix a private
//! `ssh-agent` on its own socket in the caller's temporary directory (stopped by its process, never by name), behind a
//! counting proxy that can delay or forge one signature; on Windows the pipe agent (`agent_fixture_pipe`), whose keys are
//! fixed when it starts, so a key it "no longer holds" is its next sign request refused. Nothing here reads or contacts
//! the user's agent (`SSH_AUTH_SOCK` is never read, and is removed from the children's environment), keys or
//! `known_hosts`. The adapter rows of the connection host share it with the old worker's rows (`reuse_auth`).
use crate::git::endpoint::{agent_source::AgentSource, fixture_host, ssh_fixture as common};
use std::{
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
            sync::{Mutex, atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering}},
        };

        /// A private `ssh-agent` holding the row's keys, and a proxy socket in front of it that counts sign requests
        /// and, when asked, damages the signature of the next one. Or the same proxy in front of another agent
        /// (`in_front_of`), which holds the keys and is stopped by whoever started it.
        pub(crate) struct Agent {
            child: Option<Child>,
            real: PathBuf,
            path: PathBuf,
            signs: Arc<AtomicUsize>,
            forge: Arc<AtomicU8>,
            delay: Arc<AtomicU64>,
            withdrawn: Arc<Mutex<Vec<Vec<u8>>>>,
            stop: Arc<AtomicBool>,
            accept: Option<thread::JoinHandle<()>>,
        }
        impl Agent {
            /// The agent in `dir`, holding `keys`, added in order.
            pub(crate) fn start(dir: &Path, keys: &[&Path]) -> Self {
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
                let agent = Self::proxy(Some(child), real, path);
                for key in keys {
                    agent.ssh_add(&[key.as_os_str()]);
                }
                agent
            }
            /// The counting proxy in `dir` in front of the agent that listens on `upstream`, which holds the keys.
            pub(crate) fn in_front_of(dir: &Path, upstream: &Path) -> Self {
                Self::proxy(None, upstream.to_owned(), dir.join("reuse-proxy-agent.sock"))
            }
            fn proxy(child: Option<Child>, real: PathBuf, path: PathBuf) -> Self {
                let listener = UnixListener::bind(&path).unwrap();
                listener.set_nonblocking(true).unwrap();
                let signs = Arc::new(AtomicUsize::new(0));
                let forge = Arc::new(AtomicU8::new(0));
                let delay = Arc::new(AtomicU64::new(0));
                let withdrawn = Arc::new(Mutex::new(Vec::new()));
                let stop = Arc::new(AtomicBool::new(false));
                let accept = {
                    let (real, signs, forge, delay, withdrawn, stop) =
                        (real.clone(), signs.clone(), forge.clone(), delay.clone(), withdrawn.clone(), stop.clone());
                    thread::spawn(move || {
                        while !stop.load(Ordering::SeqCst) {
                            match listener.accept() {
                                Ok((peer, _)) => {
                                    let (real, signs, forge, delay, withdrawn) =
                                        (real.clone(), signs.clone(), forge.clone(), delay.clone(), withdrawn.clone());
                                    thread::spawn(move || relay(peer, &real, &signs, &forge, &delay, &withdrawn));
                                }
                                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                    thread::sleep(Duration::from_millis(2));
                                }
                                Err(error) => panic!("agent proxy accept: {error}"),
                            }
                        }
                    })
                };
                Self { child, real, path, signs, forge, delay, withdrawn, stop, accept: Some(accept) }
            }
            pub(crate) fn source(&self) -> AgentSource {
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
            pub(crate) fn withdraw(&self, key: &Path) {
                let public = key.with_extension("pub");
                self.ssh_add(&["-d".as_ref(), public.as_os_str()]);
            }
            /// The agent no longer holds the key whose public blob is `blob`: a sign request for it fails, whether or
            /// not the agent behind the proxy is one `ssh-add` can remove it from.
            pub(crate) fn withdraw_blob(&self, blob: &[u8]) {
                self.withdrawn.lock().unwrap().push(blob.to_vec());
            }
            /// The next signature the agent returns no longer verifies.
            pub(crate) fn forge_next(&self) {
                self.forge.store(1, Ordering::SeqCst);
            }
            /// The next signature the agent returns is named `ssh-rsa`, as an agent that answers a `rsa-sha2-512`
            /// request with a SHA-1 signature does.
            pub(crate) fn downgrade_next(&self) {
                self.forge.store(2, Ordering::SeqCst);
            }
            /// The agent answers the next sign request only after `ms`.
            pub(crate) fn delay_next(&self, ms: u64) {
                self.delay.store(ms, Ordering::SeqCst);
            }
            pub(crate) fn signs(&self) -> usize {
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
                if let Some(child) = self.child.as_mut() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
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
        /// Whether `body` asks the agent to sign with a key that was withdrawn.
        fn withdrawn_signing(body: &[u8], withdrawn: &Mutex<Vec<Vec<u8>>>) -> bool {
            let Some(length) = body.get(1..5).map(|bytes| u32::from_be_bytes(bytes.try_into().unwrap()) as usize) else {
                return false;
            };
            let Some(blob) = body.get(5..5 + length) else {
                return false;
            };
            withdrawn.lock().unwrap().iter().any(|held| held == blob)
        }
        fn relay(
            mut peer: UnixStream,
            real: &Path,
            signs: &AtomicUsize,
            forge: &AtomicU8,
            delay: &AtomicU64,
            withdrawn: &Mutex<Vec<Vec<u8>>>,
        ) {
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
                    if withdrawn_signing(&body, withdrawn) {
                        // SSH_AGENT_FAILURE, as an agent that lacks the key answers.
                        if send(&mut peer, &[5]).is_err() {
                            return;
                        }
                        continue;
                    }
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
        pub(crate) use super::key_fixture::{KeyAgent, Sign};

        /// The key agent (`key_fixture`: certificates and security keys beside plain keys) behind the counting proxy,
        /// which can stop signing for one key (the proof rows of `reuse_proof_keys`).
        pub(crate) struct ProofAgent {
            // The proxy first, then the agent behind it.
            front: Agent,
            keys: KeyAgent,
        }
        impl ProofAgent {
            /// The key agent in `dir` listing `listed`, signing with the `private` key files (and their certificates).
            pub(crate) fn start(dir: &Path, private: &[&Path], listed: &[Sign<'_>]) -> Self {
                let keys = KeyAgent::start(dir, private, listed, false);
                let front = Agent::in_front_of(dir, &keys.path);
                Self { front, keys }
            }
            pub(crate) fn source(&self) -> AgentSource {
                self.front.source()
            }
            /// The agent no longer holds the key `blob`: it answers a sign request for it with a failure.
            pub(crate) fn withdraw(&self, blob: &[u8]) {
                self.front.withdraw_blob(blob);
            }
            /// The requests the key agent has seen, in order: `list`, or `sign:<listed key's index>:<flags>`.
            pub(crate) fn requests(&self) -> Vec<String> {
                self.keys.requests()
            }
        }
        /// The type of the rows' agent keys: Ed25519.
        pub(crate) const ROW_KEY: &[&str] = &["-t", "ed25519"];
    } else {
        use super::agent_fixture_pipe::{Behavior, Damage, Fault, Identity, PipeAgent, Request, pipe_name};

        /// The pipe agent holding the row's keys, which counts sign requests and can refuse or damage the next one.
        pub(crate) struct Agent {
            agent: PipeAgent,
        }
        impl Agent {
            /// The agent in `dir`, listing `keys` in order.
            pub(crate) fn start(dir: &Path, keys: &[&Path]) -> Self {
                let entries = keys
                    .iter()
                    .map(|key| (Identity::from_key_file(key), Behavior::Signs))
                    .collect();
                Self { agent: PipeAgent::start(&pipe_name(dir), entries).unwrap() }
            }
            pub(crate) fn source(&self) -> AgentSource {
                use crate::git::endpoint::agent_source::PipeName;
                AgentSource::NamedPipe(
                    PipeName::parse(std::ffi::OsStr::new(self.agent.name()))
                        .expect("the fixture's pipe name is a local pipe"),
                )
            }
            /// The agent's keys are fixed when it starts: it refuses its next sign request, as an agent that no longer
            /// holds the key does.
            pub(crate) fn withdraw(&self, _key: &Path) {
                self.agent.script(Fault::Refuse);
            }
            /// The next signature the agent returns no longer verifies.
            pub(crate) fn forge_next(&self) {
                self.agent.script(Fault::Damage(Damage::WrongAlgorithm));
            }
            /// The next signature the agent returns is named `ssh-rsa` (the pipe agent's `OneByteRsa`), as an agent
            /// that answers a `rsa-sha2-512` request with a SHA-1 signature names it.
            pub(crate) fn downgrade_next(&self) {
                self.agent.script(Fault::Damage(Damage::OneByteRsa));
            }
            /// The agent answers the next sign request only after `ms`.
            pub(crate) fn delay_next(&self, ms: u64) {
                self.agent.script(Fault::Delay(ms));
            }
            pub(crate) fn signs(&self) -> usize {
                self.agent
                    .requests()
                    .iter()
                    .filter(|request| matches!(request, Request::Sign { .. }))
                    .count()
            }
        }
        pub(crate) use super::key_fixture_pipe::{KeyAgent, Sign};

        /// The key agent (`key_fixture_pipe`: certificates and security keys beside plain keys) on its pipe (the proof
        /// rows of `reuse_proof_keys`).
        pub(crate) struct ProofAgent {
            keys: KeyAgent,
        }
        impl ProofAgent {
            /// The key agent in `dir` listing `listed`, signing with the `private` key files (and their certificates).
            pub(crate) fn start(dir: &Path, private: &[&Path], listed: &[Sign<'_>]) -> Self {
                Self { keys: KeyAgent::start(dir, private, listed) }
            }
            pub(crate) fn source(&self) -> AgentSource {
                self.keys.source()
            }
            /// The agent's keys are fixed when it starts: it refuses its next sign request, as an agent that no longer
            /// holds the key does.
            pub(crate) fn withdraw(&self, _blob: &[u8]) {
                self.keys.refuse_next_sign();
            }
            /// The requests the key agent has seen, in order: `list`, or `sign:<listed key's index>:<flags>`.
            pub(crate) fn requests(&self) -> Vec<String> {
                self.keys.requests()
            }
        }
        /// The type of the rows' agent keys: RSA, the agent key type libssh2 on Windows (CNG) signs with, in the PEM
        /// form the pipe agent signs from.
        pub(crate) const ROW_KEY: &[&str] = &["-t", "rsa", "-b", "2048", "-m", "PEM"];
    }
}

/// A new key at `path` (and `path.pub`) of the type `kind` names, by the fixture host's `ssh-keygen`.
pub(crate) fn keygen_kind(path: &Path, kind: &[&str]) {
    common::run(
        Command::new(fixture_host::programs().keygen)
            .args(["-q", "-N", ""])
            .args(kind)
            .arg("-f")
            .arg(path),
    );
}
