//! Step 1.5 of dev-docs/GwzTransportReleaseConvergencePlan.md (contract section 7; plan row R4): one capture gives the
//! operation its profile and its authentication context.
use super::super::endpoint_environment::{MachineProxy, OperationCapture, capture_operation};
use super::snapshot;
use crate::git::endpoint::https_auth::OperationAuth;
use gwz_connection::{
    AuthContext, Clock, Completion, ConnectCommand, DomainConfig, EndpointAdapter, EndpointFactory,
    EndpointRoute, EndpointSpec, ExchangeIo, ExchangeSpec, FactoryEnv, FactoryError, GitService,
    HostConfig, HostHandle, HostLimits, HostNamespace, HostPorts, OperationPolicy, OwnerSeed,
    PreparedOperation, Progress, ProgressUnavailable, RetainedWork, RetentionPort,
    RetentionRefused, Sleep, Tick,
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

mod host;
use host::{one_operation, one_operation_on, test_host};

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
        capture_operation(&snapshot(&pairs), None, Some(machine), None).unwrap()
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

#[test]
fn through_a_connection_host_a_changed_profile_or_context_reuses_nothing_and_reuse_then_resumes() {
    let world = world();
    let (_domain_owner, owner, handle, seen, made) = test_host();
    let mut tokens: Vec<&str> = Vec::new();
    let mut run = |name: &str, capture: OperationCapture, https: bool, token: &'static str| {
        tokens.push(token);
        one_operation_on(&handle, name, capture, https)
    };
    let base = || world.base.capture(MachineProxy::Direct);
    let (a, b) = ("gho_profile_token_a", "gho_profile_token_b");
    assert!(!run("base-1", base(), false, a));
    assert!(run("base-2", base(), false, a), "an equal capture reuses");
    // Profile fields: another instance, then reuse there.
    let refused = || world.base.capture(MachineProxy::Refused("refused".into()));
    let mut profile_changes: Vec<(&str, Box<dyn Fn() -> OperationCapture + '_>)> = world
        .changes
        .iter()
        .filter(|(name, _)| matches!(*name, "HOME" | "agent"))
        .map(|(name, variables)| {
            let capture: Box<dyn Fn() -> OperationCapture + '_> =
                Box::new(move || variables.capture(MachineProxy::Direct));
            (*name, capture)
        })
        .collect();
    profile_changes.push(("proxy", Box::new(refused)));
    for (name, capture) in &profile_changes {
        assert!(!run(name, capture(), false, a), "{name} changed: reused");
        assert!(run(name, capture(), false, a), "{name} again: not reused");
    }
    std::fs::write(&world.ca, &world.ca_b).unwrap();
    assert!(!run("ca-1", base(), false, a), "CA content changed: reused");
    assert!(
        run("ca-2", base(), false, a),
        "CA content again: not reused"
    );
    std::fs::write(&world.ca, &world.ca_a).unwrap();
    // The helper's environment is not a profile field. On SSH it does not partition connections (the lane
    // owner's ruling on C-P3-2; SSH reuse authorization is step 1.6's possession proof).
    let context = |name: &str| {
        let (_, variables) = world.changes.iter().find(|(n, _)| *n == name).unwrap();
        variables.capture(MachineProxy::Direct)
    };
    let gh_token = |name: &str| if name == "GH_TOKEN" { b } else { a };
    for name in ["GIT_CONFIG_GLOBAL", "GH_TOKEN"] {
        assert!(
            run(name, context(name), false, gh_token(name)),
            "SSH with {name} changed reuses"
        );
    }
    // On HTTPS it does: zero reuse across the change, then reuse resumes.
    assert!(!run("https-base-1", base(), true, a));
    assert!(run("https-base-2", base(), true, a));
    for name in ["GIT_CONFIG_GLOBAL", "GH_TOKEN"] {
        assert!(
            !run(name, context(name), true, gh_token(name)),
            "HTTPS with {name} changed: reused"
        );
        // Then reuse resumes under the new context, while the old context's connection is still idle.
        assert!(
            run(name, context(name), true, gh_token(name)),
            "HTTPS with {name} again: not reused"
        );
    }
    // HOME, the agent, the proxy and the CA content each made an instance; the helper's environment made none.
    assert_eq!(*made.lock().unwrap(), 5);
    assert_eq!(handle.snapshot().endpoint_instances, 5);
    // Every exchange was given its own operation's helper environment, never an earlier one's.
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

#[test]
fn the_trust_content_behind_ssl_cert_file_is_in_the_profile() {
    // S-P2-3: on OpenSSL builds `SSL_CERT_FILE` selects the platform roots, read once at capture; on the native
    // stacks it is the CA file. Either way its content, not its path, partitions: same path, other certificates.
    let world = world();
    let file = world._root.path().join("trust.pem");
    std::fs::write(&file, &world.ca_a).unwrap();
    let variables = Variables(vec![
        ("HOME", world._root.path().join("home-a").into_os_string()),
        ("SSL_CERT_FILE", file.clone().into_os_string()),
    ]);
    let (_domain_owner, owner, handle, _seen, made) = test_host();
    let capture = || variables.capture(MachineProxy::Direct);
    assert!(!one_operation(&handle, "trust-a-1", capture()));
    assert!(one_operation(&handle, "trust-a-2", capture()));
    std::fs::write(&file, &world.ca_b).unwrap();
    assert!(
        !one_operation(&handle, "trust-b-1", capture()),
        "changed content: reused"
    );
    assert!(one_operation(&handle, "trust-b-2", capture()));
    std::fs::write(&file, &world.ca_a).unwrap();
    assert!(
        one_operation(&handle, "trust-a-3", capture()),
        "restored content: the first instance"
    );
    assert_eq!(*made.lock().unwrap(), 2);
    drop(owner);
}

#[test]
fn a_long_machine_bypass_list_fits_the_profile_and_each_rule_partitions() {
    // C-P2-3 = S-P2-2: an 8192-character bypass list (the grammar's longest) is captured, not refused.
    let world = world();
    let rule = |n: usize| format!("*.svc{n:05}.corp.example");
    let list = |changed: Option<usize>| {
        let mut list = String::new();
        let mut n = 0;
        loop {
            let entry = if Some(n) == changed {
                rule(99_999)
            } else {
                rule(n)
            };
            if list.len() + entry.len() + 1 > 8192 {
                break;
            }
            list.push_str(&entry);
            list.push(';');
            n += 1;
        }
        list
    };
    let first = list(None);
    assert!(first.len() > 8100 && first.len() <= 8192);
    let named = |bypass: &str| MachineProxy::named_for_test("proxy.corp.example:8080", bypass);
    assert!(matches!(named(&first), MachineProxy::Named(_)));
    let base = world.base.capture(named(&first));
    assert_eq!(base.profile, world.base.capture(named(&first)).profile);
    assert_ne!(
        base.profile,
        world.base.capture(named(&list(Some(7)))).profile
    );
}
