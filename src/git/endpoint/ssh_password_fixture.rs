//! A loopback SSH server that accepts a fixed test password, for TR2.18's
//! URL-password tests: `tests/transport_backend/password_sshd.py`, run by
//! `python3`. Stock `sshd` checks only system passwords, which a test must
//! never send, so this server, on Python's standard library alone, checks its
//! own. It offers the methods a test names, `password` and `publickey` (RSA
//! keys), runs each exec request's Git command, and logs each authentication
//! request without its password. It stops and reaps its server on drop.
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

const SERVER: &str = include_str!("../../../tests/transport_backend/password_sshd.py");

/// Names the Python interpreter that runs the server, when set. A test that
/// runs in a child whose `HOME` is a temporary directory passes it the path
/// [`interpreter`] finds, since a version manager's `python3` may need the
/// user's own `HOME`.
pub(crate) const PYTHON: &str = "GWZ_TEST_PYTHON";

/// The interpreter `python3` runs here, by its own path.
pub(crate) fn interpreter() -> String {
    let output = Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .expect("python3 runs");
    assert!(output.status.success(), "python3 runs");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub(crate) struct PasswordSshd {
    child: Child,
    pub(crate) port: u16,
    /// The host key's `known_hosts` field: its type and base64 blob.
    host_key: String,
    log: PathBuf,
}

impl PasswordSshd {
    /// A server in `dir` that accepts `password`, offers `methods` in that
    /// order, and takes the RSA public keys of `authorized`, each a `.pub`
    /// file's text.
    pub(crate) fn start(
        dir: &Path,
        password: &str,
        methods: &[&str],
        authorized: &[String],
    ) -> Self {
        fs::create_dir_all(dir).unwrap();
        let script = dir.join("password_sshd.py");
        fs::write(&script, SERVER).unwrap();
        let log = dir.join("attempts.jsonl");
        let blobs: Vec<&str> = authorized
            .iter()
            .map(|line| {
                line.split_whitespace()
                    .nth(1)
                    .expect("an OpenSSH public key")
            })
            .collect();
        let config = dir.join("config.json");
        let text = serde_json::json!({
            "password": password,
            "methods": methods,
            "authorized": blobs,
            "log": log,
        });
        fs::write(&config, text.to_string()).unwrap();
        let python = std::env::var_os(PYTHON).unwrap_or_else(|| "python3".into());
        let mut child = Command::new(python)
            .arg("-B")
            .arg(&script)
            .arg(&config)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(fs::File::create(dir.join("stderr.txt")).unwrap())
            .spawn()
            .expect("python3 runs the password fixture");
        let mut ready = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        let ready: serde_json::Value = serde_json::from_str(&ready)
            .unwrap_or_else(|_| panic!("the fixture did not start: {ready:?}"));
        Self {
            port: ready["port"].as_u64().unwrap() as u16,
            host_key: ready["host_key"].as_str().unwrap().to_owned(),
            child,
            log,
        }
    }

    /// The server's host key as a `known_hosts` line for `[name]:port`.
    pub(crate) fn known_host(&self, name: &str) -> String {
        format!("[{name}]:{} {}", self.port, self.host_key)
    }

    /// The authentication requests the server has seen, in order: `none`,
    /// `password` with whether it was accepted, and `publickey` as a query
    /// or with whether its signature was accepted.
    pub(crate) fn attempts(&self) -> Vec<String> {
        let text = fs::read_to_string(&self.log).unwrap_or_default();
        text.lines()
            .map(|line| {
                let entry: serde_json::Value = serde_json::from_str(line).unwrap();
                let method = entry["method"].as_str().unwrap();
                match (entry["signed"].as_bool(), entry["accepted"].as_bool()) {
                    (Some(false), _) => format!("{method}:query"),
                    (_, Some(true)) => format!("{method}:accepted"),
                    (_, Some(false)) => format!("{method}:refused"),
                    _ => method.to_owned(),
                }
            })
            .collect()
    }

    /// Forgets the requests seen so far.
    pub(crate) fn clear(&self) {
        let _ = fs::remove_file(&self.log);
    }
}

impl Drop for PasswordSshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
