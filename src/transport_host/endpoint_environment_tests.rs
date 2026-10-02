//! 1.1.0 S6.1's environment snapshot: the runtime takes its endpoint
//! configuration, TLS and proxy included, from the snapshot its caller passes,
//! and reads no process environment (gwz-core
//! `dev-docs/GwzTransportReleasePlanAmendment-2.md` §3.17; gwz-py
//! `dev-docs/GwzPyPerOperationTransportDesign.md` §2.2).

use super::driver_tests::{commit, common, endpoint_home, fixture_url};
use super::endpoint_environment::endpoint_config;
use super::*;
use crate::git::GitBackend;
use crate::session_host::EnvironmentSnapshot;
use crate::{TransportOptions, TransportPlacement};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Instant,
};

/// Set in the child that runs the snapshot test, to the directory that holds
/// the child's own `HOME` and agent socket path.
const CHILD: &str = "GWZ_S6_1_SNAPSHOT_CHILD";

fn snapshot(pairs: &[(&str, &OsStr)]) -> EnvironmentSnapshot {
    EnvironmentSnapshot::from_os_pairs(
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), value.to_os_string())),
    )
    .unwrap()
}

fn caller_token() -> (CallControls<()>, CancellationToken) {
    let (controls, gate) = CallControls::new(&Arc::new(()));
    (controls, gate.token().clone())
}

#[test]
fn the_endpoint_configuration_comes_from_the_snapshot() {
    let root = tempfile::TempDir::new().unwrap();
    let home = root.path().join("home");
    let agent = root.path().join("agent.sock");
    let ca = root.path().join("ca.pem");
    let pem = super::ca_bundle_tests::unrelated_ca(root.path(), "snapshot-ca");
    std::fs::write(&ca, &pem).unwrap();
    let environment = snapshot(&[
        ("HOME", home.as_os_str()),
        ("SSH_AUTH_SOCK", agent.as_os_str()),
        ("GIT_SSL_CAINFO", ca.as_os_str()),
        ("HTTPS_PROXY", OsStr::new("http://proxy.example:3128")),
        ("NO_PROXY", OsStr::new("internal.example, .corp.example")),
    ]);
    let (ssh, https) = endpoint_config(&environment).unwrap();
    assert_eq!(ssh.home, home);
    assert_eq!(ssh.agent, Some(agent));
    let der = |roots: &[native_tls::Certificate]| -> Vec<Vec<u8>> {
        roots.iter().map(|root| root.to_der().unwrap()).collect()
    };
    let expected = crate::git::endpoint::ca_bundle::certificates(&pem).unwrap();
    assert_eq!(der(&https.tls.ca_roots), der(&expected));
    let proxy = https.tls.proxy.as_ref().expect("the snapshot's proxy");
    assert_eq!(
        (proxy.host.as_str(), proxy.port, proxy.tls),
        ("proxy.example", 3128, false)
    );
    assert_eq!(https.tls.no_proxy, ["internal.example", ".corp.example"]);
    // TR1.6 §3.2/configuration-view amendment: supervised Git receives the
    // captured snapshot; helper selection is performed in its controlled view.
    let auth = https.auth.expect("configured credential authentication");
    assert_eq!(auth.executable, PathBuf::from("git"));
    let mut entries = auth.environment;
    entries.sort();
    assert_eq!(
        entries,
        [
            ("GIT_SSL_CAINFO".into(), ca.into_os_string()),
            ("HOME".into(), home.into_os_string()),
            ("HTTPS_PROXY".into(), "http://proxy.example:3128".into()),
            ("NO_PROXY".into(), "internal.example, .corp.example".into()),
            (
                "SSH_AUTH_SOCK".into(),
                root.path().join("agent.sock").into()
            ),
        ]
    );
}

/// The runtime uses the snapshot's `HOME` and `SSH_AUTH_SOCK`, not the
/// process's, which differ from them. The body runs in a child of this test
/// binary whose own environment holds a home without `known_hosts`, an agent
/// socket nobody serves, and a proxy and CA file the runtime would refuse.
#[test]
fn the_runtime_takes_home_and_the_agent_from_the_snapshot_not_the_process() {
    let Some(decoys) = std::env::var_os(CHILD) else {
        return run_in_child(
            "the_runtime_takes_home_and_the_agent_from_the_snapshot_not_the_process",
        );
    };
    let decoys = PathBuf::from(decoys);
    let (process_home, process_agent) = (decoys.join("home"), decoys.join("agent.sock"));
    assert_eq!(std::env::var_os("HOME"), Some(process_home.clone().into()));
    assert_eq!(
        std::env::var_os("SSH_AUTH_SOCK"),
        Some(process_agent.clone().into())
    );

    let fixture = common::SshdFixture::new();
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    commit(&server, "snapshot");
    let home = endpoint_home(&fixture);
    let agent = Agent::start(fixture.temp.path(), &home.join("client_ed25519"));
    assert_ne!(home, process_home);
    assert_ne!(agent.socket, process_agent);
    let clone = |home: &Path, socket: &Path, name: &str| {
        let environment = snapshot(&[
            ("HOME", home.as_os_str()),
            ("SSH_AUTH_SOCK", socket.as_os_str()),
        ]);
        let options = TransportOptions {
            placement: Some(TransportPlacement::Local),
            ..Default::default()
        };
        let meta = RequestMeta {
            request_id: name.into(),
            schema_version: "gwz.protocol/v0".into(),
            transport: Some(options.clone()),
            ..Default::default()
        };
        let (_controls, token) = caller_token();
        let (result, cleanup) = with_cancellable_local_transport(
            meta,
            "clone".into(),
            &environment,
            &token,
            |backend| {
                let backend = backend
                    .with_transport(fixture.temp.path(), Some(&options))
                    .unwrap()
                    .unwrap();
                backend
                    .clone_repo(&fixture_url(&fixture), &fixture.temp.path().join(name))
                    .map(|_| backend.transport_observations().unwrap().snapshot())
            },
        );
        (result.expect("the entry ran the clone"), cleanup)
    };

    // Each of the process's values, in a snapshot, fails the clone: its home
    // does not know the fixture's host key, and nothing serves its agent.
    let (result, _) = clone(&process_home, &agent.socket, "process-home");
    assert!(result.is_err(), "the process's HOME has no known_hosts");
    let (result, _) = clone(&home, &process_agent, "process-agent");
    assert!(
        result.is_err(),
        "the process's SSH_AUTH_SOCK serves no agent"
    );

    // The snapshot's values clone through the transport, authenticated by the
    // snapshot's agent, while the process's own values stay as above.
    let (result, cleanup) = clone(&home, &agent.socket, "snapshot");
    let rows = result.expect("the snapshot's HOME and agent reach the fixture");
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|row| row.endpoint_id.is_some() && row.authenticated == Some(true)),
        "{rows:?}"
    );
    assert_eq!(cleanup.pending_local_work, 0);
    assert!(!fixture.marker.exists());
}

/// Runs `test` alone in a child of this test binary, with the process values
/// the snapshot test must not use, and asserts that it passed.
fn run_in_child(test: &str) {
    let decoys = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(decoys.path().join("home")).unwrap();
    let name = format!("{}::{test}", module_path!().split_once("::").unwrap().1);
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture", "--test-threads", "1"])
        .env(CHILD, decoys.path())
        .env("HOME", decoys.path().join("home"))
        .env("SSH_AUTH_SOCK", decoys.path().join("agent.sock"))
        .env("HTTPS_PROXY", "http://user:secret@proxy.invalid:3128")
        .env("GIT_SSL_CAINFO", decoys.path().join("missing-ca.pem"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the snapshot test's child failed:\n{stdout}\n{stderr}"
    );
    assert_eq!(
        stdout.matches(&format!("test {name} ... ok")).count(),
        1,
        "the child ran no single test:\n{stdout}"
    );
}

/// An ssh-agent of the test's own, holding one key, on a socket in `dir`.
struct Agent {
    socket: PathBuf,
    child: Child,
}

impl Agent {
    fn start(dir: &Path, key: &Path) -> Self {
        let socket = dir.join("snapshot-agent.sock");
        let child = Command::new("ssh-agent")
            .args(["-D", "-a"])
            .arg(&socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let agent = Self { socket, child };
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while !agent.socket.exists() {
            assert!(Instant::now() < deadline, "ssh-agent did not start");
            thread::sleep(std::time::Duration::from_millis(10));
        }
        common::run(
            Command::new("ssh-add")
                .arg(key)
                .env("SSH_AUTH_SOCK", &agent.socket)
                .stdin(Stdio::null()),
        );
        agent
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
