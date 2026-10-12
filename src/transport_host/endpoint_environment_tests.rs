//! 1.1.0 S6.1's environment snapshot: the runtime takes its endpoint
//! configuration, TLS and proxy included, from the snapshot its caller passes,
//! and reads no process environment (gwz-core
//! `dev-docs/GwzTransportReleasePlanAmendment-2.md` §3.17; gwz-py
//! `dev-docs/GwzPyPerOperationTransportDesign.md` §2.2).

use super::endpoint_environment::{endpoint_config, endpoint_config_in};
use super::*;
use crate::git::GitBackend;
use crate::git::endpoint::agent_source::AgentSource;
use crate::session_host::EnvironmentSnapshot;
use crate::{TransportOptions, TransportPlacement};
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
    let der = |roots: &[native_tls::Certificate]| -> Vec<Vec<u8>> {
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
