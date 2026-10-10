//! TR2.8's key fixture: `tests/transport_backend/key_agent.py`, run by
//! `python3` in front of a private `ssh-agent`, with keys, security keys and
//! certificates that `ssh-keygen` makes in the test's temporary directory.
//! The agent's software authenticator signs for each security key with the
//! plain key that backs it, as a FIDO authenticator holding that private key
//! would, with user presence asserted; OpenSSH's own client authenticates
//! with its signatures. Nothing here reads or contacts the user's agent or
//! keys, and both agents stop on drop.
use crate::git::endpoint::{
    agent_auth, agent_job::Job, agent_socket, ssh_fixture as common, ssh_network,
    ssh_password_fixture,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gwz_transport::pool::Key;
use serde_json::json;
use std::{
    fs,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

const AGENT: &str = include_str!("../../../../tests/transport_backend/key_agent.py");

/// How the agent answers a sign request for one listed key.
pub(crate) enum Sign<'a> {
    /// The private agent signs, for this base64 public key blob.
    Upstream(&'a str),
    /// The software authenticator signs for the security key (or its
    /// certificate) `key`, with the plain key `backing` in the private agent.
    Sk { key: &'a str, backing: &'a str },
    /// The agent signs with a DSA key it makes itself.
    Dsa,
    /// Every request for this key is refused, as for an absent security key.
    Absent(&'a str),
    /// The first reply is a malformed SHA-1 signature; later replies would sign.
    MalformedRsa { key: &'a str, length: usize },
}

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) struct KeyAgent {
    // Drop the proxy first, then its upstream, including during construction.
    _agent: OwnedChild,
    _upstream: OwnedChild,
    pub(crate) path: PathBuf,
    log: PathBuf,
    /// Each listed key's base64 public blob, in list order.
    pub(crate) blobs: Vec<String>,
}

impl KeyAgent {
    /// An agent in `dir` that lists `keys`, in order, with the `private` key
    /// files (and each one's `-cert.pub`, as `ssh-add` loads it) in its
    /// private agent. With `rsa_sha1` it answers a `rsa-sha2-*` request with
    /// an `ssh-rsa` signature.
    pub(crate) fn start(dir: &Path, private: &[&Path], keys: &[Sign<'_>], rsa_sha1: bool) -> Self {
        Self::start_observing(dir, private, keys, rsa_sha1, |_| {})
    }

    fn start_observing(
        dir: &Path,
        private: &[&Path],
        keys: &[Sign<'_>],
        rsa_sha1: bool,
        mut spawned: impl FnMut(&Child),
    ) -> Self {
        let real = dir.join("private-agent.sock");
        let upstream = OwnedChild(
            Command::new("ssh-agent")
                .args(["-D", "-a"])
                .arg(&real)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        spawned(&upstream.0);
        // ssh-agent binds its socket before it listens: wait until it accepts.
        let deadline = Instant::now() + Duration::from_secs(5);
        while std::os::unix::net::UnixStream::connect(&real).is_err() {
            assert!(Instant::now() < deadline, "ssh-agent did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
        for key in private {
            common::run(
                Command::new("ssh-add")
                    .env("SSH_AUTH_SOCK", &real)
                    .env_remove("SSH_AGENT_PID")
                    .arg("-q")
                    .arg(key),
            );
        }
        let script = dir.join("key_agent.py");
        fs::write(&script, AGENT).unwrap();
        let (path, log) = (dir.join("key-agent.sock"), dir.join("agent.jsonl"));
        let entries: Vec<_> = keys
            .iter()
            .map(|key| match key {
                Sign::Upstream(blob) => json!({"sign": "upstream", "blob": blob}),
                Sign::Sk { key, backing } => json!({"sign": "sk", "blob": key, "backing": backing}),
                Sign::Dsa => json!({"sign": "dsa"}),
                Sign::Absent(blob) => json!({"sign": "absent", "blob": blob}),
                Sign::MalformedRsa { key, length } => {
                    json!({"sign": "malformed_rsa", "blob": key, "length": length})
                }
            })
            .collect();
        let config = dir.join("agent.json");
        let text = json!({"socket": path, "log": log, "upstream": real, "rsa_sha1": rsa_sha1, "keys": entries});
        fs::write(&config, text.to_string()).unwrap();
        let python =
            std::env::var_os(ssh_password_fixture::PYTHON).unwrap_or_else(|| "python3".into());
        let mut agent = OwnedChild(
            Command::new(python)
                .arg("-B")
                .arg(&script)
                .arg(&config)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(fs::File::create(dir.join("agent-stderr.txt")).unwrap())
                .spawn()
                .expect("python3 runs the key agent"),
        );
        spawned(&agent.0);
        let mut ready = String::new();
        BufReader::new(agent.0.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        let ready: serde_json::Value = serde_json::from_str(&ready)
            .unwrap_or_else(|_| panic!("the key agent did not start: {ready:?}"));
        let blobs = ready["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|blob| blob.as_str().unwrap().to_owned())
            .collect();
        Self {
            _upstream: upstream,
            _agent: agent,
            path,
            log,
            blobs,
        }
    }

    /// The requests the agent has seen, in order: `list`, or
    /// `sign:<listed key's index>:<flags>`.
    pub(crate) fn requests(&self) -> Vec<String> {
        let text = fs::read_to_string(&self.log).unwrap_or_default();
        text.lines()
            .map(|line| {
                let entry: serde_json::Value = serde_json::from_str(line).unwrap();
                match entry["op"].as_str().unwrap() {
                    "sign" => format!("sign:{}:{}", entry["key"], entry["flags"]),
                    other => other.to_owned(),
                }
            })
            .collect()
    }
}

#[test]
fn startup_failures_terminate_and_reap_every_spawned_child() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    for fail_after in [0, 1, 2] {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing-key");
        let private: Vec<&Path> = if fail_after == 0 {
            vec![&missing]
        } else {
            vec![]
        };
        let mut pids = Vec::new();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                KeyAgent::start_observing(dir.path(), &private, &[], false, |child| {
                    pids.push(child.id() as libc::pid_t);
                    assert!(pids.len() != fail_after, "injected startup failure");
                })
            }))
            .is_err()
        );
        assert_eq!(pids.len(), fail_after.max(1));
        for pid in pids {
            // Neither a live child nor a zombie is left after unwinding.
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
            assert_eq!(
                unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
                -1
            );
            assert_eq!(
                io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
    }
}

pub(crate) use super::key_material::{keygen, public};

/// The security key whose point is the plain Ed25519 or ECDSA P-256 key
/// `blob`'s, with the application `ssh:`, as its type and base64 blob.
pub(crate) fn security_key(blob: &str) -> (String, String) {
    let raw = STANDARD.decode(blob).unwrap();
    let mut fields = Vec::new();
    let mut input = &raw[..];
    while !input.is_empty() {
        let length = u32::from_be_bytes(input[..4].try_into().unwrap()) as usize;
        fields.push(&input[4..4 + length]);
        input = &input[4 + length..];
    }
    let kind = match fields[0] {
        b"ssh-ed25519" => "sk-ssh-ed25519@openssh.com",
        b"ecdsa-sha2-nistp256" => "sk-ecdsa-sha2-nistp256@openssh.com",
        other => panic!("no security key of {}", String::from_utf8_lossy(other)),
    };
    let mut out = Vec::new();
    let tail: [&[u8]; 1] = [b"ssh:"];
    for field in [kind.as_bytes()]
        .into_iter()
        .chain(fields[1..].iter().copied())
        .chain(tail)
    {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field);
    }
    (kind.to_owned(), STANDARD.encode(out))
}

/// Has the CA key `ca` certify `key` for `user` as `dir/name.pub`, which
/// `ssh-keygen` signs as `dir/name-cert.pub`; returns the certificate's type
/// and base64 blob.
pub(crate) fn certify(
    dir: &Path,
    ca: &Path,
    name: &str,
    key: &(String, String),
    user: &str,
) -> (String, String) {
    let path = dir.join(format!("{name}.pub"));
    fs::write(&path, format!("{} {}\n", key.0, key.1)).unwrap();
    common::run(
        Command::new("ssh-keygen")
            .args(["-q", "-s"])
            .arg(ca)
            .args(["-I", "tr2-8", "-n", user])
            .arg(&path),
    );
    public(dir, &format!("{name}-cert"))
}

/// Authenticates as `user` with the agent at `agent` to the server at
/// `port`, trusted through `known_hosts`, on the production path: the
/// endpoint's own trust setup, then `agent_auth::authenticate_reporting`.
pub(crate) fn authenticate(
    port: u16,
    known_hosts: &Path,
    user: &str,
    agent: &Path,
) -> io::Result<()> {
    let key = Key::ssh(user, "127.0.0.1", port);
    let (known, agent, user) = (known_hosts.to_owned(), agent.to_owned(), user.to_owned());
    let mut job = Job::start_isolated(
        Some(Instant::now() + Duration::from_secs(20)),
        Duration::from_secs(1),
        move |control| {
            let (connection, trusted) = ssh_network::establish(&key, &known, &control)?;
            agent_auth::authenticate_reporting(
                connection,
                &user,
                &trusted,
                control.clone(),
                || agent_socket::connect(&agent, control),
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
