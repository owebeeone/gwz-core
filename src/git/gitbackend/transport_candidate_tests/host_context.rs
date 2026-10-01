//! TR2.11 (transport release plan amendment 2, §3.5): only a host context
//! reaches the transport. `Git2Backend::new()` carries none, so its SSH
//! operations take libgit2's native route, as 1.0.17's do, and construct no
//! transport endpoint. The same operations inside `with_local_transport` take
//! the transport.
//!
//! Both routes find `known_hosts` through `HOME`: libgit2 itself, and the
//! endpoint `with_local_transport` builds. So the test runs in a child of this
//! test binary whose `HOME` is a temporary directory that trusts the fixture's
//! host key, and this process's environment is never changed.
use super::*;

const CHILD: &str = "GWZ_TR2_11_ROUTE_CHILD";

#[test]
fn candidate_ssh_takes_the_transport_only_inside_a_host_context() {
    if std::env::var_os(CHILD).is_none() {
        run_in_child("candidate_ssh_takes_the_transport_only_inside_a_host_context");
        return;
    }
    let f = common::SshdFixture::new();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("the parent sets HOME"));
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::copy(&f.known_hosts, home.join(".ssh/known_hosts")).unwrap();
    // The fixture's own repository path needs percent escapes in a URL, which
    // the transport decodes and libgit2's native SSH passes on as written. A
    // plain path reads the same on both routes.
    let repository = f.temp.path().join("plain.git");
    commit(&git2::Repository::init_bare(&repository).unwrap(), "first");
    let url = format!(
        "ssh://{}@127.0.0.1:{}{}",
        f.user,
        f.port,
        repository.display()
    );
    let meta = crate::RequestMeta {
        request_id: "route".into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(crate::TransportOptions {
            default_identity: Some(
                f.temp
                    .path()
                    .join("client_ed25519")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..Default::default()
        }),
        ..Default::default()
    };

    // The control: inside `with_local_transport` the endpoint's receipts name
    // the connection, and the fetch reuses the clone's pooled connection
    // without offering a credential. That is what a transport endpoint shows.
    let (rows, cleanup) =
        crate::transport_host::with_local_transport(meta.clone(), "route".into(), |backend| {
            clone_then_fetch(backend, &f, &url, &meta, "transport")
        })
        .unwrap();
    assert_eq!(cleanup.pending_local_work, 0);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(
        rows.iter()
            .all(|r| r.endpoint_id.is_some() && r.connection_id.is_some()),
        "{rows:?}"
    );
    assert_eq!(rows[1].connection_id, rows[0].connection_id, "{rows:?}");
    assert_eq!(
        (rows[1].reused, rows[1].credential_offered),
        (Some(true), false),
        "{rows:?}"
    );

    // Without a host context: libgit2's own SSH transport, which offers the
    // selected key through the credential callback on a connection of its own
    // for every operation. No endpoint receipt, no pooled connection.
    let rows = clone_then_fetch(&Git2Backend::new(), &f, &url, &meta, "native");
    assert_eq!(rows.len(), 2, "{rows:?}");
    for row in &rows {
        assert_eq!(
            (
                row.endpoint_id.as_deref(),
                row.connection_id.as_deref(),
                row.stream_id,
                row.reused
            ),
            (None, None, None, None),
            "no transport endpoint may open a stream: {rows:?}"
        );
        assert!(
            row.credential_offered && row.authenticated == Some(true),
            "each native operation authenticates its own connection; a transport endpoint \
             would have pooled the clone's connection for the fetch: {rows:?}"
        );
    }
}

fn clone_then_fetch(
    backend: &Git2Backend,
    f: &common::SshdFixture,
    url: &str,
    meta: &crate::RequestMeta,
    name: &str,
) -> Vec<crate::TransportObservation> {
    let scoped = backend
        .with_transport(f.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let target = f.temp.path().join(name);
    scoped.clone_repo(url, &target).unwrap();
    scoped.fetch(&target, "origin").unwrap();
    scoped.transport_observations().unwrap().snapshot()
}

/// Runs this module's test `name` in a child of this test binary, with a
/// clean environment whose `HOME` is a new temporary directory, and asserts
/// that the child ran exactly that test and passed.
fn run_in_child(name: &str) {
    let home = tempfile::TempDir::new().unwrap();
    let module = module_path!()
        .split_once("::")
        .map_or(module_path!(), |(_, path)| path);
    let test = format!("{module}::{name}");
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .env(CHILD, "1")
        .env("HOME", home.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    for key in ["PATH", "TMPDIR", "GWZ_TEST_GIT", "GWZ_TEST_FS"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let output = command
        .args(["--exact", &test, "--nocapture", "--test-threads", "1"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the child failed:\nstdout={stdout}\nstderr={stderr}"
    );
    assert_eq!(
        stdout.matches(&format!("test {test} ... ok")).count(),
        1,
        "the child did not run exactly one test: {stdout}"
    );
}
