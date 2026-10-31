//! R3a of the Phase 1 remediation (Code P2-2; contract section 4.1): the production factory makes each endpoint instance's
//! adapter from the profile of the operation that needs it, and from nothing of the runtime that built the host. The rows
//! run through a `ConnectionHost` on a real `CoreFactory`, with profiles that real captures
//! (`endpoint_environment::capture_operation`) made from environment snapshots that differ in one variable:
//! - `SSL_CERT_FILE` against a fixture HTTPS server whose certificate the fixture CA (CA B) signed: the operation whose
//!   capture trusts B is served, and the one whose capture trusts another CA (A) is refused, before and after B's instance
//!   exists;
//! - `SSH_AUTH_SOCK` against the fixture `sshd`: the operation whose agent holds the authorized key is served, and the one
//!   whose agent holds another key is refused, whichever instance was made first;
//! - a profile with no inputs (or inputs of another composition) is refused `Unavailable` at admission.
use super::{
    ConnectionHost,
    https_tests::{ADVERTISEMENT, body, port_of},
    tests::{advertisement, block_on, is_advertisement},
};
use crate::{
    git::endpoint::{
        agent_source::AgentSource,
        https_auth::HelperSlots,
        https_fixture::{Handler, Server, response},
        shared_reservation::Authority,
        ssh_fixture::{SshdFixture, server_path},
        ssh_handoff::Handoff,
        ssh_key_snapshot::Registry,
        ssh_tests::reuse_agent::{Agent, ROW_KEY, keygen_kind},
    },
    session_host::EnvironmentSnapshot,
    transport_host::{
        connection_factory::{CoreFactory, HttpsHost, SshHost},
        driver_tests,
        endpoint_environment::{MachineProxy, OperationCapture, capture_operation},
    },
};
use gwz_connection::{
    AdmissionError, AuthContext, AuthScope, EndpointProfile, EndpointRoute, Exchange, ExchangeSpec,
    FactoryReason, GitService, HostHandle, Operation, OperationPolicy, PreparedOperation,
};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{self, Disposition},
};
use std::{
    ffi::{OsStr, OsString},
    path::Path,
    sync::Arc,
};

/// A connection host on the production factory over `ssh` and `https` host parts.
fn host_over(ssh: Option<SshHost>, https: Option<HttpsHost>) -> ConnectionHost {
    let factory = CoreFactory::new(ssh, https);
    let registry = factory.registry();
    ConnectionHost::build(
        Arc::new(factory),
        registry,
        Arc::new(crate::transport_host::composition::process_safety()),
    )
    .unwrap()
}
fn https_host() -> HttpsHost {
    HttpsHost {
        helper_slots: HelperSlots::new(),
        authority: Authority::new(256, 256),
    }
}
fn ssh_host() -> SshHost {
    let authority = Authority::new(256, 256);
    SshHost {
        registry: Registry::with_supervisor(authority.supervisor().clone()),
        authority,
        trust_log: Handoff::with_agent(AgentSource::None),
        helpers: None,
    }
}
/// One operation's capture, from the variables its snapshot holds.
fn capture(variables: &[(&str, &OsStr)]) -> OperationCapture {
    let snapshot = EnvironmentSnapshot::from_os_pairs(
        variables
            .iter()
            .map(|(name, value)| (OsString::from(name), value.to_os_string())),
    )
    .unwrap();
    capture_operation(&snapshot, None, Some(MachineProxy::Direct), None).unwrap()
}
/// An operation over `capture`'s profile and context, registered on `handle`.
fn admitted(handle: &HostHandle, name: &str, capture: OperationCapture) -> Operation {
    let input = PreparedOperation::new(
        name,
        EndpointRoute::ssh_and_https(),
        capture.profile,
        capture.auth,
        OperationPolicy::new(4, 32, 3).unwrap(),
    )
    .unwrap();
    block_on(handle.admit(input).unwrap().acquire())
        .unwrap()
        .register()
}
/// The exchange `spec` gets on `operation`, or none when the opening ended without one.
fn opened(operation: &Operation, spec: ExchangeSpec) -> Option<Exchange> {
    block_on(operation.open(spec).ok()?.wait()).ok()
}

#[test]
fn two_captures_differing_in_ssl_cert_file_make_instances_with_their_own_trust() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handler: Handler = Arc::new(|_| {
        Box::pin(async {
            response(
                200,
                protocol::GitService::UploadPackAdvertisement,
                ADVERTISEMENT,
            )
        })
    });
    let server = runtime.block_on(Server::start(handler));
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    // CA B is the CA that signed the server's certificate; CA A signed nothing it serves.
    let ca_b = dir.path().join("ca-b.pem");
    std::fs::write(&ca_b, &server.ca).unwrap();
    let ca_a = dir.path().join("ca-a.pem");
    std::fs::write(
        &ca_a,
        super::super::https_tests::unrelated_ca(dir.path(), "r3a-ca-a"),
    )
    .unwrap();
    let capture_with = |ca: &Path| {
        capture(&[
            ("HOME", home.as_os_str()),
            ("SSL_CERT_FILE", ca.as_os_str()),
        ])
    };
    let mut host = host_over(None, Some(https_host()));
    let handle = host.handle();
    let spec = || {
        ExchangeSpec::git(
            Key::https("localhost", port_of(&server)),
            Identity::Https,
            GitService::UploadPackAdvertisement,
            "/repo",
        )
        .unwrap()
    };
    // A's trust does not hold the server's CA: its instance refuses the server's certificate.
    let a = admitted(&handle, "op-a", capture_with(&ca_a));
    assert!(
        opened(&a, spec()).is_none(),
        "an instance with A's trust served a server B signed"
    );
    block_on(a.finish());
    // B's instance trusts B's CA, whatever instance was made before it.
    let b = admitted(&handle, "op-b", capture_with(&ca_b));
    let mut exchange = opened(&b, spec()).expect("an instance with B's trust refused B's server");
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(b.finish());
    // And B's instance does not make A's any wider.
    let again = admitted(&handle, "op-a-again", capture_with(&ca_a));
    assert!(
        opened(&again, spec()).is_none(),
        "A's instance took on B's trust"
    );
    block_on(again.finish());
    assert_eq!(handle.snapshot().endpoint_instances, 2, "one per profile");
    assert!(block_on(host.close()));
    drop(runtime);
}

#[test]
fn two_captures_differing_in_ssh_auth_sock_make_instances_with_their_own_agent() {
    use std::io::Write;
    let fixture = SshdFixture::new();
    let home = driver_tests::endpoint_home(&fixture);
    let dir = tempfile::tempdir().unwrap();
    // The agent behind "good" holds the key the server authorizes; the one behind "bad" holds another.
    let (good_dir, bad_dir) = (dir.path().join("good"), dir.path().join("bad"));
    std::fs::create_dir(&good_dir).unwrap();
    std::fs::create_dir(&bad_dir).unwrap();
    let (good_key, bad_key) = (good_dir.join("key"), bad_dir.join("key"));
    keygen_kind(&good_key, ROW_KEY);
    keygen_kind(&bad_key, ROW_KEY);
    let mut authorized = std::fs::OpenOptions::new()
        .append(true)
        .open(fixture.temp.path().join("authorized_keys"))
        .unwrap();
    write!(
        authorized,
        "\n{}\n",
        std::fs::read_to_string(good_key.with_extension("pub"))
            .unwrap()
            .trim()
    )
    .unwrap();
    drop(authorized);
    let good = Agent::start(&good_dir, &[good_key.as_path()]);
    let bad = Agent::start(&bad_dir, &[bad_key.as_path()]);
    let socket = |agent: &Agent| match agent.source() {
        AgentSource::Path(path) => path.into_os_string(),
        AgentSource::NamedPipe(pipe) => OsString::from(pipe.as_str()),
        other => panic!("the row's agent is not a socket or a pipe: {other:?}"),
    };
    let (good_socket, bad_socket) = (socket(&good), socket(&bad));
    let capture_with =
        |agent: &OsStr| capture(&[("HOME", home.as_os_str()), ("SSH_AUTH_SOCK", agent)]);
    let mut host = host_over(Some(ssh_host()), None);
    let handle = host.handle();
    let spec = || {
        ExchangeSpec::git(
            Key::ssh(&fixture.user, "127.0.0.1", fixture.port),
            Identity::Ambient,
            GitService::UploadPackAdvertisement,
            &server_path(&fixture.url_repository),
        )
        .unwrap()
    };
    // The first instance has the agent that cannot authenticate.
    let a = admitted(&handle, "op-bad", capture_with(&bad_socket));
    assert!(
        opened(&a, spec()).is_none(),
        "an instance whose agent holds another key authenticated"
    );
    block_on(a.finish());
    assert_eq!(
        good.signs(),
        0,
        "the good agent was asked before it was named"
    );
    // The second names the good agent, and is served by it, whatever instance was made first.
    let b = admitted(&handle, "op-good", capture_with(&good_socket));
    let mut exchange =
        opened(&b, spec()).expect("an instance whose agent holds the authorized key was refused");
    let advertised = advertisement(&mut exchange).unwrap();
    assert!(is_advertisement(&advertised), "{advertised:?}");
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(b.finish());
    assert!(good.signs() >= 1, "the named agent signed nothing");
    assert_eq!(handle.snapshot().endpoint_instances, 2, "one per profile");
    assert!(block_on(host.close()));
}

#[test]
fn a_profile_without_inputs_is_refused_unavailable() {
    let mut host = host_over(Some(ssh_host()), Some(https_host()));
    let handle = host.handle();
    let refused = |profile: EndpointProfile| {
        let input = PreparedOperation::new(
            "op-refused",
            EndpointRoute::ssh_and_https(),
            profile,
            AuthContext::new(64, AuthScope::new([0; 32]), Arc::new(())),
            OperationPolicy::new(4, 32, 3).unwrap(),
        )
        .unwrap();
        match block_on(handle.admit(input).unwrap().acquire()) {
            Err(AdmissionError::UnavailableAdapter(error)) => error.reason(),
            other => panic!(
                "expected the factory's refusal, got {:?}",
                other.map(|_| ())
            ),
        }
    };
    // No inputs at all, and inputs of another composition's type: neither is one this factory makes an adapter from.
    assert_eq!(
        refused(EndpointProfile::new(b"no-inputs".to_vec(), [9; 32]).unwrap()),
        FactoryReason::Unavailable
    );
    assert_eq!(
        refused(
            EndpointProfile::with_inputs(b"other-inputs".to_vec(), [8; 32], Arc::new(5u8)).unwrap()
        ),
        FactoryReason::Unavailable
    );
    assert_eq!(handle.snapshot().endpoint_instances, 0, "nothing was made");
    assert!(block_on(host.close()));
}
