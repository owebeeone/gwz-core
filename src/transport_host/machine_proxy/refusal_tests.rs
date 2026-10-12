//! A machine proxy setting that cannot be used refuses only an https open (design section 6, decision D6), and a direct
//! one words a failed direct connection (decision D7, text U2). The setting comes from an injected source, so no row
//! reads or changes this host's WinHTTP settings.

use super::MachineProxy;
use super::testing::Fixed;
use super::tunnel_tests::{discover, plain_server, run};
use crate::git::endpoint::https_connection::{Config, MachineSetting};
use crate::git::endpoint::ssh_fixture::SshdFixture;
use crate::session_host::EnvironmentSnapshot;
use crate::transport_host::driver_tests::{
    block_on, commit, endpoint_home, fixture_url, local_meta,
};
use crate::transport_host::endpoint_environment::endpoint_config_native;
use crate::transport_host::{HelperSlots, HttpsOpenFailure, TransportRuntime};
use crate::{OperationPolicy, RequestMeta};
use gwz_transport::protocol::{ErrorCode, GitService};
use std::{
    ffi::OsString,
    path::Path,
    sync::{Arc, Mutex, atomic::Ordering},
};

const T1: &str = "The Windows machine proxy setting has no entry for https: \"http=proxy:8080\". GWZ does not send https traffic around a proxy that was set up for other protocols only.";
const DIRECT_WORDS: &str = "says direct access, so GWZ connected to the server directly";

/// An `http=`-only mapping: usable for http, and with no entry that applies to https (text T1).
fn http_only() -> MachineProxy {
    Fixed::named("http=proxy:8080", None).capture()
}

/// The runtime a command builds under `machine`, from a snapshot whose `HOME` is `home`: the configuration is built
/// the way a command builds it.
fn runtime_under(machine: Option<MachineProxy>, home: &Path) -> TransportRuntime {
    let environment =
        EnvironmentSnapshot::from_os_pairs([(OsString::from("HOME"), home.as_os_str().to_owned())])
            .unwrap();
    let (ssh, https) = endpoint_config_native(&environment, None, machine)
        .expect("a machine setting never refuses the configuration");
    TransportRuntime::with_https(ssh, https, HelperSlots::new()).unwrap()
}

fn meta(id: &str) -> RequestMeta {
    RequestMeta {
        schema_version: "gwz.protocol/v0".into(),
        request_id: id.into(),
        policy: Some(OperationPolicy {
            max_retries: Some(0),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The failure an https open of `url` ends in on `runtime`.
fn https_open_failure(runtime: &TransportRuntime, id: &str, url: &str) -> String {
    let request = block_on(runtime.request(meta(id), "fetch".into())).unwrap();
    let error = request
        .context
        .open_https_recording(
            url,
            GitService::UploadPackAdvertisement,
            None,
            Arc::new(|_, _| {}),
            Arc::new(|_| {}),
            Arc::new(Mutex::new(None)),
        )
        .err()
        .expect("the open fails");
    let failure = error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<HttpsOpenFailure>())
        .expect("a failed https open");
    let reason = failure.reason();
    block_on(request.finish());
    reason
}

fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

#[test]
fn the_setting_is_recorded_beside_the_proxy_it_chose() {
    let mut config = Config::default();
    assert_eq!(config.machine, MachineSetting::Unread);
    Fixed::new(super::Access::Direct, None, None)
        .capture()
        .apply(&mut config);
    assert_eq!(config.machine, MachineSetting::Direct);
    Fixed::named("proxy.example:8080", None)
        .capture()
        .apply(&mut config);
    assert_eq!(config.machine, MachineSetting::Named);
    http_only().apply(&mut config);
    assert!(
        matches!(&config.machine, MachineSetting::Refused(text) if text.starts_with(T1)),
        "{:?}",
        config.machine
    );
}

#[test]
fn a_refused_setting_ends_an_https_open_at_the_connector_before_any_connection() {
    run(async {
        let server = plain_server().await;
        let mut config = server.config();
        http_only().apply(&mut config);
        let failure = discover(&server, config, &server.url)
            .await
            .expect_err("a refused setting refuses the open");
        assert_eq!(failure.code, ErrorCode::UnsupportedOperation);
        assert_eq!(server.connections.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn an_https_open_under_an_http_only_mapping_fails_with_t1_and_connects_nowhere() {
    // The origin runs on its own threads, so that a connection attempt would be counted.
    let origin = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let server = origin.block_on(plain_server());
    let home = tempfile::tempdir().unwrap();
    let runtime = runtime_under(Some(http_only()), home.path());
    let reason = https_open_failure(&runtime, "refused-https", &server.url);
    assert!(reason.starts_with(T1), "{reason}");
    assert!(
        reason.ends_with("connects directly as gwz 1.0 did."),
        "{reason}"
    );
    assert_eq!(server.connections.load(Ordering::SeqCst), 0);
    block_on(runtime.shutdown());
}

#[test]
fn an_http_only_mapping_still_lets_an_ssh_fetch_and_a_local_command_run() {
    let fixture = SshdFixture::new();
    commit(
        &git2::Repository::open_bare(&fixture.repository).unwrap(),
        "first",
    );
    let home = endpoint_home(&fixture);
    let runtime = runtime_under(Some(http_only()), &home);
    let meta = local_meta("refused-ssh", &home);
    let request = block_on(runtime.request(meta.clone(), "clone".into())).unwrap();
    let backend = request
        .backend()
        .with_transport(fixture.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    use crate::git::GitBackend;
    // An SSH fetch.
    let fetched = fixture.temp.path().join("by-ssh");
    backend
        .clone_repo(&fixture_url(&fixture), &fetched)
        .expect("the SSH fetch runs");
    assert_eq!(
        std::fs::read_to_string(fetched.join("payload")).unwrap(),
        "first"
    );
    // A local command: it reads the repository just fetched, and touches no transport.
    let paths = backend
        .repository_paths(&fetched)
        .expect("the local command runs");
    assert!(paths.worktree.is_some());
    assert_eq!(block_on(request.finish()).pending_local_work, 0);
    block_on(runtime.shutdown());
}

#[test]
fn the_no_proxy_hint_comes_only_from_a_direct_setting_and_a_failed_direct_connection() {
    let home = tempfile::tempdir().unwrap();
    let refused_url = |port: u16| format!("https://127.0.0.1:{port}/repo");
    // A direct setting: the connection to the server is refused, and the open says WinHTTP names no proxy.
    let direct = runtime_under(
        Some(Fixed::new(super::Access::Direct, None, None).capture()),
        home.path(),
    );
    let reason = https_open_failure(&direct, "direct", &refused_url(closed_port()));
    assert!(
        reason.starts_with("HTTPS endpoint request failed: Unavailable"),
        "{reason}"
    );
    assert!(reason.contains(DIRECT_WORDS), "{reason}");
    block_on(direct.shutdown());
    // A named proxy: the connection to the proxy is refused, and nothing says direct. (A configuration with no machine
    // setting at all, which only a Unix build makes, is worded by `a_failed_direct_connection_says_winhttp_has_no_proxy_u2`.)
    let port = closed_port();
    let named = runtime_under(
        Some(Fixed::named(&format!("127.0.0.1:{port}"), None).capture()),
        home.path(),
    );
    let reason = https_open_failure(&named, "named", "https://server.test/repo");
    assert!(
        reason.starts_with("HTTPS endpoint request failed: Unavailable"),
        "{reason}"
    );
    assert!(!reason.contains(DIRECT_WORDS), "{reason}");
    block_on(named.shutdown());
}
