//! 1.1.0 S6.1's environment snapshot: the runtime takes its endpoint
//! configuration, TLS and proxy included, from the snapshot its caller passes,
//! and reads no process environment (gwz-core
//! `dev-docs/GwzTransportReleasePlanAmendment-2.md` §3.17; gwz-py
//! `dev-docs/GwzPyPerOperationTransportDesign.md` §2.2).

use super::endpoint_environment::{endpoint_config, endpoint_config_in};
use super::*;
use crate::TransportOptions;
use crate::git::GitBackend;
use crate::git::endpoint::agent_source::AgentSource;
use crate::session_host::EnvironmentSnapshot;
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

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
    std::fs::create_dir(&home).unwrap();
    let agent = snapshot_agent(root.path());
    let ca = root.path().join("ca.pem");
    let pem = super::https_tests::unrelated_ca(root.path(), "snapshot-ca");
    std::fs::write(&ca, &pem).unwrap();
    let environment = snapshot(&[
        ("HOME", home.as_os_str()),
        ("SSH_AUTH_SOCK", agent.as_os_str()),
        ("GIT_SSL_CAINFO", ca.as_os_str()),
        ("HTTPS_PROXY", OsStr::new("http://proxy.example:3128")),
        ("NO_PROXY", OsStr::new("internal.example, .corp.example")),
    ]);
    let (ssh, https) = endpoint_config(&environment).unwrap();
    assert_eq!(ssh.ssh.as_ref().unwrap().home, home);
    assert_eq!(ssh.ssh.as_ref().unwrap().agent, expected_agent(&agent));
    let der = |roots: &[crate::git::endpoint::https_backend::Certificate]| -> Vec<Vec<u8>> {
        roots.iter().map(|root| root.to_der().unwrap()).collect()
    };
    let expected = crate::git::endpoint::ca_bundle::certificates(&pem).unwrap();
    assert_eq!(der(&https.tls.ca_roots), der(&expected));
    check_proxy(&https.tls);
    check_configured_authentication(https.auth, &ca, &home, &agent);
}

/// The agent source is chosen once for a runtime and travels in its configuration as the enum, whatever form the
/// caller handed the fixture (plan step 3.2a).
#[test]
fn the_agent_source_round_trips_through_the_runtime_configuration() {
    let home = PathBuf::from("endpoint-home");
    for (given, expected) in [
        (None, AgentSource::None),
        (
            Some(PathBuf::from("agent.sock")),
            AgentSource::Path(PathBuf::from("agent.sock")),
        ),
    ] {
        let settings = EndpointSettings::from(SshEndpointConfig::fixture(home.clone(), given));
        let ssh = settings.ssh.expect("an SSH runtime");
        assert_eq!(ssh.agent, expected);
        assert_eq!(ssh.home, home);
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// The proxy the snapshot's `HTTPS_PROXY` and `NO_PROXY` name.
        fn check_proxy(tls: &crate::git::endpoint::https_connection::Config) {
            let proxy = tls.proxy.as_ref().expect("the snapshot's proxy");
            assert_eq!(
                (proxy.host.as_str(), proxy.port, proxy.tls),
                ("proxy.example", 3128, false)
            );
            assert_eq!(tls.no_proxy, ["internal.example", ".corp.example"]);
        }

        /// The agent socket the snapshot's `SSH_AUTH_SOCK` names.
        fn expected_agent(socket: &PathBuf) -> AgentSource {
            AgentSource::Path(socket.clone())
        }

        /// What the snapshot's `SSH_AUTH_SOCK` holds: a socket path.
        fn snapshot_agent(root: &std::path::Path) -> PathBuf {
            root.join("agent.sock")
        }
    } else {
        /// What the snapshot's `SSH_AUTH_SOCK` holds: a pipe on this computer.
        fn snapshot_agent(_root: &std::path::Path) -> PathBuf {
            PathBuf::from(r"\\.\pipe\gwz-snapshot-agent")
        }

        /// A `SSH_AUTH_SOCK` that is not a local pipe refuses the agent, when a setup needs one, and nothing else:
        /// the runtime is built, so an operation that needs no agent is not refused for it (plan step 3.2b).
        #[test]
        fn a_variable_that_is_not_a_local_pipe_leaves_the_runtime_built_with_a_refused_agent() {
            let root = tempfile::TempDir::new().unwrap();
            let environment = snapshot(&[
                ("HOME", root.path().as_os_str()),
                ("SSH_AUTH_SOCK", OsStr::new("/tmp/ssh-abc/agent.1")),
            ]);
            let (ssh, _) = endpoint_config(&environment).unwrap();
            assert!(matches!(ssh.ssh.unwrap().agent, AgentSource::Refused(_)));
        }

        /// Windows takes its proxy from the machine, not from the environment (steps 4.6 and 4.7 capture it).
        fn check_proxy(tls: &crate::git::endpoint::https_connection::Config) {
            assert!(tls.proxy.is_none() && tls.no_proxy.is_empty());
        }

        /// Windows takes the pipe the snapshot's `SSH_AUTH_SOCK` names (step 3.2b), when no Pageant window is
        /// visible. The capture's probe is the real one (step 3.6), so these rows hold on a desktop with no Pageant
        /// (an SSH session's, or a machine where none runs); a Pageant on the desktop that runs them is selected
        /// instead, as it should be, and the rows say so by failing.
        fn expected_agent(pipe: &PathBuf) -> AgentSource {
            AgentSource::NamedPipe(
                crate::git::endpoint::agent_source::PipeName::parse(pipe.as_os_str()).unwrap(),
            )
        }
    }
}

// TR1.6 §3.2/configuration-view amendment: supervised Git receives the captured snapshot; helper selection is
// performed in its controlled view. On Windows too (plan step 4.3): the helper is `git`, which the lookup finds on
// the snapshot's `PATH`, and its environment is the snapshot.
fn check_configured_authentication(
    auth: Option<crate::git::endpoint::https_auth::Config>,
    ca: &PathBuf,
    home: &PathBuf,
    agent: &PathBuf,
) {
    let auth = auth.expect("configured credential authentication");
    assert_eq!(auth.executable, PathBuf::from("git"));
    let mut entries = auth.environment;
    entries.sort();
    assert_eq!(
        entries,
        [
            ("GIT_SSL_CAINFO".into(), ca.clone().into_os_string()),
            ("HOME".into(), home.clone().into_os_string()),
            ("HTTPS_PROXY".into(), "http://proxy.example:3128".into()),
            ("NO_PROXY".into(), "internal.example, .corp.example".into()),
            ("SSH_AUTH_SOCK".into(), agent.clone().into()),
        ]
    );
}

// The snapshot's agent is an `ssh-agent` on a Unix socket, or on Windows the pipe agent fixture of step 1.7, which the
// runtime reaches through the product's pipe channel (plan steps 3.2b and 3.3, joined with 3.4's signing).
use super::driver_tests::{commit, common, endpoint_home, fixture_url};
use std::{path::Path, process::Command};

/// Set in the child that runs the snapshot test, to the directory that holds
/// the child's own `HOME` and agent path.
const CHILD: &str = "GWZ_S6_1_SNAPSHOT_CHILD";

/// The runtime uses the snapshot's `HOME` and `SSH_AUTH_SOCK`, not the
/// process's, which differ from them. The body runs in a child of this test
/// binary whose own environment holds a home without `known_hosts`, an agent
/// nobody serves, and a proxy and CA file the runtime would refuse.
#[test]
fn the_runtime_takes_home_and_the_agent_from_the_snapshot_not_the_process() {
    let Some(decoys) = std::env::var_os(CHILD) else {
        return run_in_child(
            "the_runtime_takes_home_and_the_agent_from_the_snapshot_not_the_process",
        );
    };
    let decoys = PathBuf::from(decoys);
    let (process_home, process_agent) = (decoys.join("home"), decoy_agent(&decoys));
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
        let options = TransportOptions::default();
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
        .env("SSH_AUTH_SOCK", decoy_agent(decoys.path()))
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

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::{
            process::{Child, Stdio},
            thread,
            time::Instant,
        };

        /// The agent the process environment of the child names, which nobody serves: a socket path.
        fn decoy_agent(dir: &Path) -> PathBuf {
            dir.join("agent.sock")
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
    } else {
        use crate::git::endpoint::ssh_tests::agent_fixture_pipe::{
            Behavior, Identity, PipeAgent, pipe_name,
        };

        /// The agent the process environment of the child names, which nobody serves: a pipe nobody created.
        fn decoy_agent(dir: &Path) -> PathBuf {
            PathBuf::from(format!(r"\\.\pipe\gwz-decoy-{}", dir.file_name().unwrap().to_string_lossy()))
        }

        /// The pipe agent fixture of step 1.7, serving one key on a pipe of its own: the product's pipe channel
        /// reaches it, and `agent_auth` signs through it (plan steps 3.3 and 3.4).
        struct Agent {
            socket: PathBuf,
            _agent: PipeAgent,
        }

        impl Agent {
            fn start(dir: &Path, key: &Path) -> Self {
                let name = pipe_name(dir);
                // The fixture's own key file, beside its `.pub`: the endpoint's home holds a copy of the key alone.
                let key = dir.join(key.file_name().unwrap());
                let entries = vec![(Identity::from_key_file(&key), Behavior::Signs)];
                let agent = PipeAgent::start(&name, entries).unwrap();
                Self {
                    socket: PathBuf::from(name),
                    _agent: agent,
                }
            }
        }
    }
}

// Plan step 3.1, GwzTransportWindowsParityDesign.md section 3: the endpoint's SSH home is chosen from the snapshot by the
// home contract (`git::endpoint::ssh_home`), whose rows are tested there; these rows show that the endpoint takes its
// home, and the source it came from, from that choice.
mod ssh_home_rows {
    use super::*;
    use crate::git::endpoint::ssh_home::HomeSource;

    fn home_of(
        pairs: &[(&str, &OsStr)],
    ) -> Result<(PathBuf, HomeSource), crate::model::ModelError> {
        let (local, _) = endpoint_config(&snapshot(pairs))?;
        let ssh = local.ssh.expect("an SSH home was chosen");
        Ok((ssh.home, ssh.home_source))
    }

    #[test]
    fn with_home_unset_the_userprofile_is_the_home() {
        // B03: a `HOME`-less environment works.
        let root = tempfile::TempDir::new().unwrap();
        let profile = root.path().join("profile with spaces");
        std::fs::create_dir(&profile).unwrap();
        let (home, source) = home_of(&[("USERPROFILE", profile.as_os_str())]).unwrap();
        assert_eq!((home, source), (profile, HomeSource::UserProfile));
    }

    #[test]
    fn home_comes_first_and_says_it_is_home() {
        let root = tempfile::TempDir::new().unwrap();
        let (a, b) = (root.path().join("a"), root.path().join("b"));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let (home, source) =
            home_of(&[("HOME", a.as_os_str()), ("USERPROFILE", b.as_os_str())]).unwrap();
        assert_eq!((home, source), (a, HomeSource::Home));
    }

    #[test]
    fn an_empty_home_is_refused_with_words_that_name_it() {
        let root = tempfile::TempDir::new().unwrap();
        let profile = root.path().join("profile");
        std::fs::create_dir(&profile).unwrap();
        let refused = home_of(&[
            ("HOME", OsStr::new("")),
            ("USERPROFILE", profile.as_os_str()),
        ])
        .unwrap_err();
        assert!(
            refused.message.contains("HOME is set but empty"),
            "{}",
            refused.message
        );
    }

    #[test]
    fn no_home_at_all_is_refused_with_words_that_name_the_variables() {
        let refused = home_of(&[("PATH", OsStr::new("x"))]).unwrap_err();
        assert!(
            refused.message.contains("no SSH home was found") && refused.message.contains("HOME"),
            "{}",
            refused.message
        );
    }

    #[test]
    fn the_runtimes_driver_knows_the_home_and_its_source_that_a_trust_message_prints() {
        // The driver words a host-key failure (design section 14, item 13, U8) and expands `~/` identities: the home
        // and its source are those of the endpoint, the one place `known_hosts` is read from.
        let root = tempfile::TempDir::new().unwrap();
        let profile = root.path().join("profile");
        std::fs::create_dir(&profile).unwrap();
        let (local, https) =
            endpoint_config(&snapshot(&[("USERPROFILE", profile.as_os_str())])).unwrap();
        let runtime = TransportRuntime::with_https(local, https, HelperSlots::new()).unwrap();
        let home = runtime
            .driver_home_for_test()
            .expect("an SSH runtime names its home");
        assert_eq!((home.path, home.source), (profile, HomeSource::UserProfile));
    }

    // The rows of each platform's rules: Windows' extra candidates, its requirement that a candidate exist, and its
    // relative home; and, elsewhere, a home taken as written, no relative home, and native bytes.
    cfg_if::cfg_if! {
        if #[cfg(windows)] {
            #[test]
            fn homedrive_and_homepath_are_the_home_when_home_is_unset() {
                let root = tempfile::TempDir::new().unwrap();
                let profile = root.path().join("profile");
                let share = root.path().join("share");
                std::fs::create_dir(&profile).unwrap();
                std::fs::create_dir(&share).unwrap();
                let text = share.to_str().unwrap();
                let (drive, path) = text.split_at(2);
                let (home, source) = home_of(&[
                    ("HOMEDRIVE", OsStr::new(drive)),
                    ("HOMEPATH", OsStr::new(path)),
                    ("USERPROFILE", profile.as_os_str()),
                ])
                .unwrap();
                assert_eq!((home, source), (share, HomeSource::HomeDrivePath));
            }

            #[test]
            fn a_home_that_is_not_a_directory_is_skipped_on_windows() {
                let root = tempfile::TempDir::new().unwrap();
                let profile = root.path().join("profile");
                std::fs::create_dir(&profile).unwrap();
                let (home, source) = home_of(&[
                    ("HOME", root.path().join("missing").as_os_str()),
                    ("USERPROFILE", profile.as_os_str()),
                ])
                .unwrap();
                assert_eq!((home, source), (profile, HomeSource::UserProfile));
            }

            #[test]
            fn a_relative_home_that_exists_is_kept_as_an_absolute_path() {
                // OQ7 (a): resolved against the current directory its driver captured, so that a later change of the
                // process directory cannot redirect trust.
                let root = tempfile::TempDir::new().unwrap();
                std::fs::create_dir(root.path().join("rel")).unwrap();
                let environment = snapshot(&[("HOME", OsStr::new("rel"))]);
                let (local, _) = endpoint_config_in(&environment, root.path()).unwrap();
                let ssh = local.ssh.unwrap();
                assert_eq!(ssh.home, root.path().join("rel"));
                assert_eq!(ssh.home_source, HomeSource::Home);
                // A driver that captured no directory leaves a relative home out.
                let refused = endpoint_config(&environment).err().unwrap();
                assert!(refused.message.contains("no SSH home was found"), "{}", refused.message);
            }
        } else if #[cfg(unix)] {
            #[test]
            fn a_relative_home_names_no_home_on_unix() {
                let refused = home_of(&[("HOME", OsStr::new("."))]).unwrap_err();
                assert!(
                    refused.message.contains("no SSH home was found"),
                    "{}",
                    refused.message
                );
                // Even with a captured directory: a relative home is the other platforms' notion.
                let root = tempfile::TempDir::new().unwrap();
                std::fs::create_dir(root.path().join("rel")).unwrap();
                let refused = endpoint_config_in(&snapshot(&[("HOME", OsStr::new("rel"))]), root.path())
                    .err()
                    .unwrap();
                assert!(
                    refused.message.contains("no SSH home was found"),
                    "{}",
                    refused.message
                );
            }

            #[test]
            fn a_home_that_is_not_utf8_reaches_the_endpoint_unchanged() {
                // TD6 (Unix review): native bytes, never a lossy conversion.
                use std::os::unix::ffi::{OsStrExt, OsStringExt};
                let name = OsString::from_vec(b"/home/\xff\xfe".to_vec());
                let (home, _) = home_of(&[("HOME", name.as_os_str())]).unwrap();
                assert_eq!(home.as_os_str().as_bytes(), name.as_bytes());
            }
        }
    }
}

// Step 1.5 of dev-docs/GwzTransportReleaseConvergencePlan.md (contract section 7; plan row R4): one capture gives the
// operation its profile and its authentication context. HOME, the agent, the CA file's content and the proxy are
// profile fields; the helper's environment (`GIT_CONFIG_GLOBAL`, `GH_TOKEN`) is the context's alone.
mod profile_partition {
    use super::super::endpoint_environment::{MachineProxy, OperationCapture, capture_operation};
    use super::snapshot;
    use crate::git::endpoint::https_auth::OperationAuth;
    use gwz_connection::{
        AuthContext, Clock, Completion, ConnectCommand, DomainConfig, EndpointAdapter,
        EndpointFactory, EndpointRoute, EndpointSpec, ExchangeIo, ExchangeSpec, FactoryEnv,
        FactoryError, GitService, HostConfig, HostHandle, HostLimits, HostNamespace, HostPorts,
        OperationPolicy, OwnerSeed, PreparedOperation, Progress, ProgressUnavailable, RetainedWork,
        RetentionPort, RetentionRefused, Sleep, Tick,
    };
    use gwz_transport::{
        pool::{ConnectionId, Identity, Key},
        protocol::Disposition,
    };
    use std::{
        collections::VecDeque,
        ffi::{OsStr, OsString},
        future::Future,
        io,
        path::PathBuf,
        pin::pin,
        sync::{Arc, Mutex},
        task::{Context, Poll, Waker},
    };

    /// The variables one operation's snapshot holds, by name.
    #[derive(Clone)]
    struct Variables(Vec<(&'static str, OsString)>);
    impl Variables {
        fn with(&self, name: &'static str, value: impl Into<OsString>) -> Self {
            let mut next = self.clone();
            next.0.retain(|(other, _)| *other != name);
            next.0.push((name, value.into()));
            next
        }
        fn capture(&self, machine: MachineProxy) -> OperationCapture {
            let pairs: Vec<(&str, &OsStr)> = self
                .0
                .iter()
                .map(|(name, value)| (*name, value.as_os_str()))
                .collect();
            capture_operation(&snapshot(&pairs), None, Some(machine)).unwrap()
        }
    }

    /// An agent address of kind `n`, the same on every platform: a socket path to Unix, and to Windows a value that is not
    /// a local pipe, which it records as a refusal that shows the value (so two of them differ). No Pageant may be
    /// visible on a Windows desktop that runs these rows, as for the rows above.
    fn agent(n: &str) -> OsString {
        OsString::from(format!("gwz-profile-agent-{n}.sock"))
    }

    /// A home, an agent, a CA file, a global Git configuration and a token, each with a second value.
    struct World {
        /// Holds the files the variables name for the test's life.
        _root: tempfile::TempDir,
        base: Variables,
        changes: Vec<(&'static str, Variables)>,
        ca: PathBuf,
        ca_a: Vec<u8>,
        ca_b: Vec<u8>,
    }
    fn world() -> World {
        let root = tempfile::TempDir::new().unwrap();
        for home in ["home-a", "home-b"] {
            std::fs::create_dir(root.path().join(home)).unwrap();
        }
        let ca = root.path().join("ca.pem");
        let ca_a = super::super::https_tests::unrelated_ca(root.path(), "profile-ca-a");
        let ca_b = super::super::https_tests::unrelated_ca(root.path(), "profile-ca-b");
        std::fs::write(&ca, &ca_a).unwrap();
        let base = Variables(vec![
            ("HOME", root.path().join("home-a").into_os_string()),
            ("SSH_AUTH_SOCK", agent("a")),
            ("GIT_SSL_CAINFO", ca.clone().into_os_string()),
            (
                "GIT_CONFIG_GLOBAL",
                root.path().join("gitconfig-a").into_os_string(),
            ),
            ("GH_TOKEN", OsString::from("gho_profile_token_a")),
        ]);
        let changes = vec![
            ("HOME", base.with("HOME", root.path().join("home-b"))),
            ("agent", base.with("SSH_AUTH_SOCK", agent("b"))),
            (
                "GIT_CONFIG_GLOBAL",
                base.with("GIT_CONFIG_GLOBAL", root.path().join("gitconfig-b")),
            ),
            ("GH_TOKEN", base.with("GH_TOKEN", "gho_profile_token_b")),
        ];
        World {
            _root: root,
            base,
            changes,
            ca,
            ca_a,
            ca_b,
        }
    }

    #[test]
    fn the_profile_holds_home_agent_ca_content_and_proxy_and_the_context_holds_the_helper_environment()
     {
        let world = world();
        let base = world.base.capture(MachineProxy::Direct);
        let again = world.base.capture(MachineProxy::Direct);
        assert_eq!(base.profile, again.profile, "one environment, one profile");
        assert_eq!(base.auth.scope(), again.auth.scope());
        for (name, variables) in &world.changes {
            let changed = variables.capture(MachineProxy::Direct);
            // Every change gives the helper another environment, so another context.
            assert_ne!(base.auth.scope(), changed.auth.scope(), "{name}");
            let profile_field = matches!(*name, "HOME" | "agent");
            assert_eq!(
                base.profile != changed.profile,
                profile_field,
                "{name} is {}a profile field",
                if profile_field { "" } else { "not " }
            );
        }
        // The CA file's content, not its name: the same path with other certificates is another profile.
        std::fs::write(&world.ca, &world.ca_b).unwrap();
        assert_ne!(
            world.base.capture(MachineProxy::Direct).profile,
            base.profile
        );
        std::fs::write(&world.ca, &world.ca_a).unwrap();
        assert_eq!(
            world.base.capture(MachineProxy::Direct).profile,
            base.profile
        );
        // The proxy: the machine setting where one is captured (Windows always captures it). The environment's proxy, which
        // Unix reads where none is, is the same profile field; `profile::the_proxy_and_bypass_list_are_profile_fields`
        // holds that on every platform.
        let refused = MachineProxy::Refused("the machine proxy setting is refused".into());
        assert_ne!(world.base.capture(refused).profile, base.profile);
        // The helper's view is the context's: the token reaches it there, and the profile's inputs carry no helper.
        let auth = OperationAuth::of(&base.auth).expect("core's provider");
        assert!(
            auth.helper()
                .environment
                .iter()
                .any(|(name, value)| { name == "GH_TOKEN" && value == "gho_profile_token_a" })
        );
        let inputs = base.profile.inputs().expect("the factory's inputs");
        assert!(
            inputs
                .downcast_ref::<super::super::endpoint_environment::EndpointInputs>()
                .is_some()
        );
    }

    // A connection host made of the composition's real gate and inert ports, whose one adapter connects at once and
    // records whose token each connect and exchange was given.

    struct Still;
    impl Clock for Still {
        fn now(&self) -> Tick {
            Tick::new(0)
        }
        fn sleep_until(&self, _: Tick) -> Sleep {
            Box::pin(std::future::pending())
        }
        fn epoch(&self) -> u64 {
            0
        }
    }
    struct Seed;
    impl OwnerSeed for Seed {
        fn namespace(&self) -> HostNamespace {
            HostNamespace::new(15)
        }
    }
    /// An executor that keeps the host's work and never runs it: this adapter's connects complete inside the submit's
    /// own pump, so no row here needs the driver to run.
    struct Held(Mutex<Vec<gwz_connection::DriverWork>>);
    impl Progress for Held {
        fn check(&self) -> Result<(), ProgressUnavailable> {
            Ok(())
        }
        fn spawn(&self, work: gwz_connection::DriverWork) -> Result<(), ProgressUnavailable> {
            self.0.lock().unwrap().push(work);
            Ok(())
        }
    }
    struct Keep(Mutex<Vec<RetainedWork>>);
    impl RetentionPort for Keep {
        fn retain(&self, work: RetainedWork) -> Result<(), RetentionRefused> {
            self.0.lock().unwrap().push(work);
            Ok(())
        }
    }
    type Seen = Arc<Mutex<Vec<(&'static str, Option<OsString>)>>>;
    struct Factory {
        seen: Seen,
        made: Arc<Mutex<usize>>,
    }
    impl EndpointFactory for Factory {
        fn create(
            &self,
            spec: &EndpointSpec,
            _: &FactoryEnv<'_>,
        ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
            // The factory gets the profile's inputs and nothing of any operation's helper.
            assert!(spec.profile.inputs().is_some());
            *self.made.lock().unwrap() += 1;
            Ok(Box::new(Adapter {
                seen: self.seen.clone(),
                completions: VecDeque::new(),
            }))
        }
    }
    struct Adapter {
        seen: Seen,
        completions: VecDeque<Completion>,
    }
    fn token(auth: &AuthContext) -> Option<OsString> {
        let auth = OperationAuth::of(auth)?;
        auth.helper()
            .environment
            .iter()
            .find(|(name, _)| name == "GH_TOKEN")
            .map(|(_, value)| value.clone())
    }
    struct Loopback(VecDeque<u8>);
    impl ExchangeIo for Loopback {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.extend(data);
            Ok(data.len())
        }
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let n = out.len().min(self.0.len());
            for (slot, byte) in out.iter_mut().zip(self.0.drain(..n)) {
                *slot = byte;
            }
            Ok(n)
        }
        fn poll_finish(&mut self, _: &mut Context<'_>) -> Poll<Disposition> {
            Poll::Ready(Disposition::Reusable)
        }
        fn abort(&mut self) {}
    }
    impl EndpointAdapter for Adapter {
        fn connect(&mut self, command: ConnectCommand) {
            let given = command.auth.as_ref().and_then(token);
            self.seen.lock().unwrap().push(("connect", given));
            self.completions.push_back(Completion::Connected {
                connection: command.connection,
                result: Ok(Some(command.identity)),
            });
        }
        fn cancel_connect(&mut self, _: ConnectionId) {}
        fn close(&mut self, connection: ConnectionId) {
            self.completions
                .push_back(Completion::Closed { connection });
        }
        fn start_exchange(
            &mut self,
            _: ConnectionId,
            _: &ExchangeSpec,
            auth: &AuthContext,
        ) -> io::Result<Box<dyn ExchangeIo>> {
            self.seen.lock().unwrap().push(("exchange", token(auth)));
            Ok(Box::new(Loopback(VecDeque::new())))
        }
        fn poll_completion(&mut self, _: &mut Context<'_>) -> Poll<Completion> {
            match self.completions.pop_front() {
                Some(completion) => Poll::Ready(completion),
                None => Poll::Pending,
            }
        }
        fn poll_dispose(&mut self, _: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
    /// A future that the inert ports leave ready on its first poll.
    fn ready<F: Future>(future: F) -> F::Output {
        match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("the future was not ready"),
        }
    }
    /// One operation over `capture`, one exchange: whether it reused a connection.
    fn one_operation(handle: &HostHandle, name: &str, capture: OperationCapture) -> bool {
        let input = PreparedOperation::new(
            name,
            EndpointRoute::ssh_and_https(),
            capture.profile,
            capture.auth,
            OperationPolicy::new(4, 32, 3).unwrap(),
        )
        .unwrap();
        let operation = ready(handle.admit(input).unwrap().acquire())
            .unwrap()
            .register();
        let spec = ExchangeSpec::git(
            Key::ssh("git", "example.invalid", 22),
            Identity::Ambient,
            GitService::UploadPackExchange,
            "/owner/repo.git",
        )
        .unwrap();
        let exchange = ready(operation.open(spec).unwrap().wait()).unwrap();
        let reused = exchange.reused();
        assert_eq!(ready(exchange.finish()).disposition, Disposition::Reusable);
        ready(operation.finish());
        reused
    }

    #[test]
    fn through_a_connection_host_a_changed_profile_or_context_reuses_nothing_and_reuse_then_resumes()
     {
        let world = world();
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let made = Arc::new(Mutex::new(0));
        let (_domain_owner, domain) = gwz_connection::cleanup_domain(
            DomainConfig::default(),
            Arc::new(Keep(Mutex::new(Vec::new()))),
        )
        .unwrap();
        let config = HostConfig {
            domain,
            safety: Arc::new(crate::transport_host::composition::process_safety()),
            limits: HostLimits::default(),
        };
        let ports = HostPorts::new(
            Arc::new(Still),
            Arc::new(Seed),
            Arc::new(Held(Mutex::new(Vec::new()))),
            Arc::new(Factory {
                seen: seen.clone(),
                made: made.clone(),
            }),
        );
        let (owner, handle) = gwz_connection::build(config, ports).unwrap();
        let base = || world.base.capture(MachineProxy::Direct);
        assert!(!one_operation(&handle, "base-1", base()));
        assert!(
            one_operation(&handle, "base-2", base()),
            "an equal capture reuses"
        );
        let mut tokens = vec!["gho_profile_token_a"; 2];
        let mut changed: Vec<(&str, Box<dyn Fn() -> OperationCapture + '_>)> = world
            .changes
            .iter()
            .map(|(name, variables)| {
                let capture: Box<dyn Fn() -> OperationCapture + '_> =
                    Box::new(move || variables.capture(MachineProxy::Direct));
                (*name, capture)
            })
            .collect();
        changed.push((
            "proxy",
            Box::new(|| world.base.capture(MachineProxy::Refused("refused".into()))),
        ));
        for (name, capture) in &changed {
            assert!(
                !one_operation(&handle, name, capture()),
                "{name} changed: reused"
            );
            assert!(
                one_operation(&handle, name, capture()),
                "{name} again: not reused"
            );
            let token = if *name == "GH_TOKEN" {
                "gho_profile_token_b"
            } else {
                "gho_profile_token_a"
            };
            tokens.extend([token; 2]);
        }
        std::fs::write(&world.ca, &world.ca_b).unwrap();
        assert!(
            !one_operation(&handle, "ca-1", base()),
            "CA content changed: reused"
        );
        assert!(
            one_operation(&handle, "ca-2", base()),
            "CA content again: not reused"
        );
        tokens.extend(["gho_profile_token_a"; 2]);
        // HOME, the agent, the proxy and the CA content each made an instance; the helper's environment made none.
        assert_eq!(*made.lock().unwrap(), 5);
        assert_eq!(handle.snapshot().endpoint_instances, 5);
        // Every connect and exchange was given its own operation's helper environment, never an earlier one's.
        let seen = seen.lock().unwrap();
        let exchanges: Vec<_> = seen
            .iter()
            .filter(|(kind, _)| *kind == "exchange")
            .map(|(_, token)| token.clone())
            .collect();
        let expected: Vec<_> = tokens
            .iter()
            .map(|token| Some(OsString::from(token)))
            .collect();
        assert_eq!(exchanges, expected);
        for (kind, token) in seen.iter().filter(|(kind, _)| *kind == "connect") {
            assert!(token.is_some(), "{kind} had no context");
        }
        drop(seen);
        drop(owner);
    }
}
