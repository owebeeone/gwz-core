//! The Windows twin of [`super::key_fixture`] (GwzTransportWindowsParityPlan.md, step 1.7): an agent that lists the
//! keys a test names, in order, and signs as each key's entry says, served on a named pipe.
//!
//! The Unix `KeyAgent` is `key_agent.py` in front of a private `ssh-agent`, and starts two processes, which its
//! startup-failure row terminates and reaps. This one is [`PipeAgent`] with a thread and a pipe handle in the test
//! process, so its startup-failure row is that a start that fails part way leaves no pipe behind. What it cannot
//! represent stays Unix: the SHA-1 answer to an `rsa-sha2` request (libssh2 on Windows offers only `rsa-sha2-512`, and the
//! agent signs RSA with `ring`, which has no SHA-1 RSA). It signs Ed25519 and ECDSA keys (P-256, P-384, P-521) too, and
//! lists a key's OpenSSH certificate (`<key>-cert.pub`, which `ssh-keygen -s` writes) as a separate identity that the key
//! signs for, as OpenSSH's agent does. A security key (`Sign::Sk`, or a certificate of one) is signed by a software
//! authenticator over the plain key that backs it, and a DSA key (`Sign::Dsa`) is one the agent makes and signs with
//! itself, as `key_agent.py` does for the Unix rows; a key nothing backs can be listed and refused (`Sign::Absent`).
//!
//! [`KeyAgent::authenticate`] is the Windows twin of the Unix `authenticate`: the endpoint's own trust setup, then
//! `agent_auth::authenticate_reporting` over a pipe channel to this agent (plan step 3.4). It authenticates on the
//! production path, and signs with the agent as the transport does.
use super::agent_fixture_pipe::{
    Behavior, Identity, PipeAgent, PipeChannel, Request, Stage, pipe_name,
};
use crate::git::endpoint::{
    agent_auth, agent_client::Agent, agent_job::Job, ssh_fixture as common, ssh_network,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gwz_transport::pool::Key;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

/// How the agent answers a sign request for one listed key.
pub(crate) enum Sign<'a> {
    /// The agent signs, for this base64 public key blob.
    Upstream(&'a str),
    /// The software authenticator signs for the security key (or its certificate) `key`, with the plain key `backing`.
    Sk { key: &'a str, backing: &'a str },
    /// The agent signs with a DSA key it makes itself.
    Dsa,
    /// Every request for this key is refused, as for an absent security key. The key need not be one the agent holds.
    Absent(&'a str),
    /// The first reply is a malformed SHA-1 signature; later replies sign.
    MalformedRsa { key: &'a str, length: usize },
}

pub(crate) struct KeyAgent {
    agent: PipeAgent,
    /// Each listed key's base64 public blob, in list order.
    pub(crate) blobs: Vec<String>,
}

impl KeyAgent {
    /// An agent in `dir` that lists `keys`, in order, signing with the `private` key files (PEM RSA keys).
    pub(crate) fn start(dir: &Path, private: &[&Path], keys: &[Sign<'_>]) -> Self {
        Self::start_observing(dir, private, keys, |_| {})
    }

    fn start_observing(
        dir: &Path,
        private: &[&Path],
        keys: &[Sign<'_>],
        observe: impl FnMut(Stage),
    ) -> Self {
        // As `ssh-add` loads a key with its `-cert.pub`, the agent holds a key's certificate beside the key; the
        // test's list names which of them the agent lists.
        let identities: Vec<Identity> = private
            .iter()
            .flat_map(|path| {
                let key = Identity::from_key_file(path);
                let certificate = certificate_blob(path).map(|blob| key.certified_by(blob));
                [Some(key), certificate].into_iter().flatten()
            })
            .collect();
        let held = |blob: &str| {
            identities
                .iter()
                .find(|identity| STANDARD.encode(identity.blob()) == blob)
                .cloned()
        };
        let find = |blob: &str| {
            held(blob).unwrap_or_else(|| panic!("no private key for the listed key {blob}"))
        };
        let entries: Vec<_> = keys
            .iter()
            .map(|key| match key {
                Sign::Upstream(blob) => (find(blob), Behavior::Signs),
                Sign::Sk { key, backing } => (
                    find(backing).security_key(STANDARD.decode(key).unwrap()),
                    Behavior::Signs,
                ),
                Sign::Dsa => (Identity::dsa(), Behavior::Signs),
                Sign::Absent(blob) => (
                    held(blob)
                        .unwrap_or_else(|| Identity::unbacked(STANDARD.decode(blob).unwrap())),
                    Behavior::Refuses,
                ),
                Sign::MalformedRsa { key, length } => {
                    (find(key), Behavior::MalformedFirst(*length))
                }
            })
            .collect();
        let blobs = entries
            .iter()
            .map(|(identity, _)| STANDARD.encode(identity.blob()))
            .collect();
        let agent = PipeAgent::start_observing(&pipe_name(dir), entries, observe).unwrap();
        Self { agent, blobs }
    }

    pub(crate) fn connect(&self) -> PipeChannel {
        self.agent.connect()
    }

    /// The agent source a runtime would choose for this agent: the pipe it serves on.
    pub(crate) fn source(&self) -> crate::git::endpoint::agent_source::AgentSource {
        use crate::git::endpoint::agent_source::{AgentSource, PipeName};
        AgentSource::NamedPipe(
            PipeName::parse(std::ffi::OsStr::new(self.agent.name()))
                .expect("the fixture's pipe name is a local pipe"),
        )
    }

    /// The requests the agent has seen, in order: `list`, or `sign:<listed key's index>:<flags>`.
    pub(crate) fn requests(&self) -> Vec<String> {
        self.agent
            .requests()
            .into_iter()
            .map(|request| match request {
                Request::List => "list".to_owned(),
                Request::Sign { key, flags } => format!("sign:{key}:{flags}"),
            })
            .collect()
    }
}

pub(crate) use super::key_material::{certify, keygen, public, security_key};

/// The blob of the certificate `ssh-keygen -s` wrote for the key at `key`, as `<key>-cert.pub`, if there is one.
fn certificate_blob(key: &Path) -> Option<Vec<u8>> {
    let mut name = key.file_name()?.to_owned();
    name.push("-cert.pub");
    let text = fs::read_to_string(key.with_file_name(name)).ok()?;
    STANDARD.decode(text.split_whitespace().nth(1)?).ok()
}

/// A new RSA key of `bits` at `dir/name` in the traditional PEM form, which the agent signs from (a new-format RSA
/// key is `keygen`'s default); its public key is `dir/name.pub`.
pub(crate) fn keygen_pem_rsa(dir: &Path, name: &str, bits: u32) -> PathBuf {
    let path = dir.join(name);
    common::run(
        Command::new(crate::git::endpoint::fixture_host::programs().keygen)
            .args([
                "-q",
                "-t",
                "rsa",
                "-m",
                "PEM",
                "-N",
                "",
                "-b",
                &bits.to_string(),
                "-f",
            ])
            .arg(&path),
    );
    path
}

/// The transport's setup (connect, handshake, trust), connecting again when libssh2 on Windows (CNG) ends the key
/// exchange on its own side, about one connection in a hundred, whatever the server (`ssh_fixture::handshaken`). The
/// setup reports that as `Other`, the only kind it gives a handshake failure; any other failure is returned at once.
fn establish(
    key: &Key,
    known_hosts: &Path,
    control: &crate::git::endpoint::agent_job::Control,
) -> io::Result<(crate::git::endpoint::ssh_connection::SshConnection, Vec<u8>)> {
    let mut attempt = 1;
    loop {
        match ssh_network::establish(key, known_hosts, control) {
            Err(error) if attempt < 6 && error.kind() == io::ErrorKind::Other => attempt += 1,
            result => return result,
        }
    }
}

impl KeyAgent {
    /// Authenticates as `user` to the server at `port`, trusted through `known_hosts`, on the production path: the
    /// endpoint's own trust setup, then `agent_auth::authenticate_reporting` over this agent's pipe. `PermissionDenied`
    /// when no key authenticates.
    pub(crate) fn authenticate(&self, port: u16, known_hosts: &Path, user: &str) -> io::Result<()> {
        let key = Key::ssh(user, "127.0.0.1", port);
        let (known, user) = (known_hosts.to_owned(), user.to_owned());
        let channel = self.agent.connect();
        let mut job = Job::start_isolated(
            Some(Instant::now() + Duration::from_secs(20)),
            Duration::from_secs(1),
            move |control| {
                let (connection, trusted) = establish(&key, &known, &control)?;
                agent_auth::authenticate_reporting(
                    connection,
                    &user,
                    &trusted,
                    control.clone(),
                    || Ok(Agent::new(channel, control)),
                    || {},
                    || {},
                )
                .map(drop)
            },
        )?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
                return result;
            }
            assert!(Instant::now() < deadline, "authentication did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::endpoint::{agent_client::Agent, agent_job::Job, ssh_fixture};
    use std::{
        io,
        panic::{AssertUnwindSafe, catch_unwind},
        sync::Arc,
        task::{Context, Poll, Waker},
        time::{Duration, Instant},
    };

    fn key(dir: &std::path::Path, name: &str) -> (std::path::PathBuf, String) {
        let path = dir.join(name);
        ssh_fixture::run_fixture_key(&path);
        let (_, blob) = public(dir, name);
        (path, blob)
    }

    fn run<T: Send + 'static>(
        body: impl FnOnce(Arc<crate::git::endpoint::agent_job::Control>) -> io::Result<T>
        + Send
        + 'static,
    ) -> io::Result<T> {
        let mut job = Job::start_isolated(
            Some(Instant::now() + Duration::from_secs(10)),
            Duration::from_secs(1),
            body,
        )
        .unwrap();
        let until = Instant::now() + Duration::from_secs(12);
        loop {
            if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
                return result;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_agent_lists_the_keys_it_is_told_to_in_that_order() {
        let dir = tempfile::tempdir().unwrap();
        let (first, first_blob) = key(dir.path(), "first");
        let (second, second_blob) = key(dir.path(), "second");
        let agent = KeyAgent::start(
            dir.path(),
            &[&first, &second],
            &[Sign::Upstream(&second_blob), Sign::Upstream(&first_blob)],
        );
        assert_eq!(agent.blobs, vec![second_blob.clone(), first_blob.clone()]);
        let channel = agent.connect();
        let listed = run(move |control| Agent::new(channel, control).identities()).unwrap();
        assert_eq!(
            listed,
            vec![
                STANDARD.decode(&second_blob).unwrap(),
                STANDARD.decode(&first_blob).unwrap()
            ]
        );
        assert_eq!(agent.requests(), vec!["list".to_owned()]);
    }

    #[test]
    fn a_certificate_is_listed_for_its_key_and_signed_by_that_key() {
        // As `ssh-add` loads a key with its `-cert.pub`: the agent lists the certificate, and a sign request for the
        // certificate's blob is answered with the key's own signature (RSA at the requested SHA-2, flag 4).
        for (kind, flags) in [("ed25519", 0), ("rsa", 4)] {
            let dir = tempfile::tempdir().unwrap();
            let ca = keygen(dir.path(), "ca", "ed25519", None);
            let path = if kind == "rsa" {
                keygen_pem_rsa(dir.path(), "key", 2048)
            } else {
                keygen(dir.path(), "key", kind, None)
            };
            let certificate = certify(
                dir.path(),
                &ca,
                "key",
                &public(dir.path(), "key"),
                "someone",
            );
            let agent = KeyAgent::start(dir.path(), &[&path], &[Sign::Upstream(&certificate.1)]);
            assert_eq!(agent.blobs, vec![certificate.1.clone()], "{kind}");
            let (channel, blob) = (agent.connect(), STANDARD.decode(&certificate.1).unwrap());
            let signature =
                run(move |control| Agent::new(channel, control).sign(&blob, b"data", flags))
                    .unwrap()
                    .unwrap_or_else(|| panic!("{kind}: the agent refused"));
            assert!(
                Identity::from_key_file(&path).verify(flags, b"data", &signature),
                "{kind}"
            );
            assert_eq!(
                agent.requests(),
                vec!["sign:0:".to_owned() + &flags.to_string()],
                "{kind}"
            );
        }
    }

    /// Asks the agent for a signature of `data` with the listed key `blob`, with flags 0.
    fn signed(agent: &KeyAgent, blob: &str, data: &'static [u8]) -> Option<Vec<u8>> {
        let (channel, blob) = (agent.connect(), STANDARD.decode(blob).unwrap());
        run(move |control| Agent::new(channel, control).sign(&blob, data, 0)).unwrap()
    }

    /// The fields of a signature blob: its algorithm name, and the rest after the first two strings.
    fn fields(signature: &[u8]) -> (&[u8], &[u8], &[u8]) {
        let length =
            |at: usize| u32::from_be_bytes(signature[at..at + 4].try_into().unwrap()) as usize;
        let name = 4..4 + length(0);
        let raw = name.end + 4..name.end + 4 + length(name.end);
        (
            &signature[name],
            &signature[raw.clone()],
            &signature[raw.end..],
        )
    }

    #[test]
    fn a_security_key_is_signed_by_the_software_authenticator_with_a_counter() {
        // The signature OpenSSH's FIDO key types carry: the algorithm, the raw signature, the flags byte with user
        // presence set and the counter. The raw signature is the plain key's over what an authenticator signs.
        for (kind, bits, name, raw_length) in [
            ("ed25519", None, "sk-ssh-ed25519@openssh.com", Some(64)),
            (
                "ecdsa",
                Some(256),
                "sk-ecdsa-sha2-nistp256@openssh.com",
                None,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = keygen(dir.path(), "backing", kind, bits);
            let backing = public(dir.path(), "backing");
            let security = security_key(&backing.1);
            let sign = Sign::Sk {
                key: &security.1,
                backing: &backing.1,
            };
            let agent = KeyAgent::start(dir.path(), &[&path], &[sign]);
            assert_eq!(agent.blobs, vec![security.1.clone()], "{kind}");
            for counter in [1_u32, 2] {
                let signature = signed(&agent, &security.1, b"data")
                    .unwrap_or_else(|| panic!("{kind}: the agent refused"));
                let (found, raw, tail) = fields(&signature);
                assert_eq!(found, name.as_bytes(), "{kind}");
                assert!(
                    raw_length.is_none_or(|length| raw.len() == length),
                    "{kind}"
                );
                assert_eq!(
                    tail,
                    [&[1][..], &counter.to_be_bytes()[..]].concat(),
                    "{kind}: flags and counter"
                );
            }
            assert_eq!(agent.requests(), ["sign:0:0", "sign:0:0"], "{kind}");
        }
    }

    #[test]
    fn a_dsa_key_is_listed_with_its_blob_and_signs_ssh_dss_afresh_each_time() {
        let dir = tempfile::tempdir().unwrap();
        let agent = KeyAgent::start(dir.path(), &[], &[Sign::Dsa]);
        let blob = STANDARD.decode(&agent.blobs[0]).unwrap();
        assert_eq!(&blob[..11], b"\0\0\0\x07ssh-dss");
        let (first, second) = (
            signed(&agent, &agent.blobs[0], b"data").unwrap(),
            signed(&agent, &agent.blobs[0], b"data").unwrap(),
        );
        for signature in [&first, &second] {
            let (name, raw, tail) = fields(signature);
            assert_eq!((name, raw.len(), tail), (&b"ssh-dss"[..], 40, &[][..]));
        }
        assert_ne!(first, second, "a nonce for each signature");
    }

    #[test]
    fn a_key_nothing_backs_is_listed_and_refused() {
        // The agent lists the blob it is told to, and every request for it fails, as for a device that is absent.
        let dir = tempfile::tempdir().unwrap();
        keygen(dir.path(), "absent", "ed25519", None);
        let absent = security_key(&public(dir.path(), "absent").1);
        let agent = KeyAgent::start(dir.path(), &[], &[Sign::Absent(&absent.1)]);
        assert_eq!(agent.blobs, vec![absent.1.clone()]);
        assert_eq!(signed(&agent, &absent.1, b"data"), None);
        assert_eq!(agent.requests(), ["sign:0:0"]);
    }

    #[test]
    fn an_absent_key_is_refused_every_time_and_a_malformed_first_reply_is_followed_by_signing() {
        let dir = tempfile::tempdir().unwrap();
        let (absent, absent_blob) = key(dir.path(), "absent");
        let (rsa, rsa_blob) = key(dir.path(), "rsa");
        let agent = KeyAgent::start(
            dir.path(),
            &[&absent, &rsa],
            &[
                Sign::Absent(&absent_blob),
                Sign::MalformedRsa {
                    key: &rsa_blob,
                    length: 3,
                },
            ],
        );
        let (absent_key, rsa_key) = (
            STANDARD.decode(&absent_blob).unwrap(),
            STANDARD.decode(&rsa_blob).unwrap(),
        );
        let channel = agent.connect();
        let (first, second, refused) = run(move |control| {
            let mut agent = Agent::new(channel, control);
            let refused = (
                agent.sign(&absent_key, b"d", 4)?,
                agent.sign(&absent_key, b"d", 4)?,
            );
            Ok((
                agent.sign(&rsa_key, b"d", 4)?,
                agent.sign(&rsa_key, b"d", 4)?,
                refused,
            ))
        })
        .unwrap();
        assert_eq!(refused, (None, None));
        assert!(first.is_some() && second.is_some());
        assert_ne!(first, second, "the first reply was not malformed");
        assert_eq!(
            agent.requests(),
            vec!["sign:0:4", "sign:0:4", "sign:1:4", "sign:1:4"]
        );
    }

    #[test]
    fn a_start_that_fails_part_way_leaves_no_pipe_behind() {
        for stage in [Stage::PipeCreated, Stage::Serving] {
            let dir = tempfile::tempdir().unwrap();
            let (private, blob) = key(dir.path(), "k");
            let name = pipe_name(dir.path());
            let failed = catch_unwind(AssertUnwindSafe(|| {
                KeyAgent::start_observing(
                    dir.path(),
                    &[&private],
                    &[Sign::Upstream(&blob)],
                    |reached| {
                        assert!(reached != stage, "injected startup failure");
                    },
                )
            }));
            assert!(failed.is_err());
            assert_eq!(
                PipeChannel::open(&name)
                    .err()
                    .expect("the pipe is gone")
                    .kind(),
                io::ErrorKind::NotFound
            );
        }
    }
}
