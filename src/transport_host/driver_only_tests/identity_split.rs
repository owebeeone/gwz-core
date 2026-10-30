use super::*;

// State round-2 P2-1: the route and the identity rules classify a URL once. A bracketed IPv6 SCP spelling is SSH to the
// route; identity selection must agree, and no selected identity may be dropped without a word.

/// Spellings `bound_route` sends to the SSH route that the old SCP test read as a non-SSH destination (a one-character
/// host `[`), and one it always read as SSH.
const IPV6_SCP: [&str; 3] = ["[::1]:repo", "[2001:db8::1]:repo", "git@[::1]:repo"];

/// The request's backend scoped with `options` (the cli placement, so the identity paths stay opaque to the core and
/// the client's endpoint checks them), as the `with_transport` of a command's own options makes it.
fn scoped_backend(
    harness: &Harness,
    request: &TransportRequest,
    options: TransportOptions,
) -> crate::git::Git2Backend {
    let options = TransportOptions {
        placement: Some(TransportPlacement::Cli),
        ..options
    };
    request
        .backend()
        .with_transport(harness.temp.path(), Some(&options))
        .unwrap()
        .unwrap()
}

/// What the selection of an identity did to `url`: the errors of the operations that read the remote, and the
/// selection source each attempt recorded.
struct Observed {
    errors: Vec<(&'static str, ModelError)>,
    sources: Vec<crate::TransportSelectionSource>,
}
fn observe(
    harness: &Harness,
    backend: &crate::git::Git2Backend,
    url: &str,
    index: usize,
) -> Observed {
    let receiver = receiver_repository(harness.temp.path(), &format!("ipv6-{index}"), url);
    let errors = vec![
        (
            "ls_remote_url",
            backend
                .ls_remote_url(&receiver, url, "origin", None)
                .unwrap_err(),
        ),
        ("fetch", backend.fetch(&receiver, "origin").unwrap_err()),
    ];
    let sources = backend
        .transport_observations()
        .unwrap()
        .snapshot()
        .into_iter()
        .map(|row| row.selection_source)
        .collect();
    Observed { errors, sources }
}
impl Observed {
    /// The outcome as the harness measured it, for the record.
    fn report(&self, what: &str, url: &str) {
        for (operation, error) in &self.errors {
            eprintln!(
                "OBSERVED {what} {url}: {operation} -> {:?}: {}",
                error.code, error.message
            );
        }
        eprintln!(
            "OBSERVED {what} {url}: attempt selection sources {:?}",
            self.sources
        );
    }
    /// No path dropped a selected identity: every operation ended in an error that does not call the destination a
    /// non-SSH one, and each attempt that began recorded the selected identity's source.
    fn assert_identity_kept(&self, what: &str, url: &str, source: crate::TransportSelectionSource) {
        for (operation, error) in &self.errors {
            assert!(
                !error.message.contains("non-SSH destination"),
                "{what} {url} {operation}: {}",
                error.message
            );
        }
        for recorded in &self.sources {
            assert_eq!(
                *recorded, source,
                "{what} {url}: the selected identity was dropped"
            );
        }
    }
}

#[test]
fn an_invocation_identity_is_kept_for_a_bracketed_ipv6_scp_spelling() {
    let harness = Harness::new();
    let key = harness.temp.path().join("selected-key");
    std::fs::write(&key, b"fixture key").unwrap();
    let (request, client) = harness.request("identity-ipv6-default");
    let backend = scoped_backend(
        &harness,
        &request,
        TransportOptions {
            default_identity: Some(key.to_string_lossy().into_owned()),
            ..Default::default()
        },
    );
    // Every spelling is observed and recorded before any is asserted.
    let observed: Vec<_> = IPV6_SCP
        .iter()
        .enumerate()
        .map(|(index, url)| (url, observe(&harness, &backend, url, index)))
        .collect();
    for (url, observed) in &observed {
        observed.report("default identity", url);
    }
    for (url, observed) in &observed {
        observed.assert_identity_kept(
            "default identity",
            url,
            crate::TransportSelectionSource::InvocationDefault,
        );
    }
    block_on(request.finish());
    block_on(client.finish());
}

#[test]
fn a_per_remote_identity_is_kept_for_a_bracketed_ipv6_scp_spelling() {
    let harness = Harness::new();
    let key = harness.temp.path().join("selected-key");
    std::fs::write(&key, b"fixture key").unwrap();
    let (request, client) = harness.request("identity-ipv6-remote");
    let backend = scoped_backend(
        &harness,
        &request,
        TransportOptions {
            remote_identities: vec![crate::RemoteSshIdentity {
                remote: "origin".into(),
                private_key_path: key.to_string_lossy().into_owned(),
            }],
            ..Default::default()
        },
    );
    // Every spelling is observed and recorded before any is asserted.
    let observed: Vec<_> = IPV6_SCP
        .iter()
        .enumerate()
        .map(|(index, url)| (url, observe(&harness, &backend, url, index)))
        .collect();
    for (url, observed) in &observed {
        observed.report("per-remote identity", url);
    }
    for (url, observed) in &observed {
        observed.assert_identity_kept(
            "per-remote identity",
            url,
            crate::TransportSelectionSource::InvocationRemote,
        );
    }
    block_on(request.finish());
    block_on(client.finish());
}
