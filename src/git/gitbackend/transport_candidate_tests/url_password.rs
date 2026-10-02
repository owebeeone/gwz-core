//! TR2.18: a password beside the user in an `ssh://` URL, used as 1.0.17's
//! libgit2 uses it. libgit2 lists the server's methods first, offers the
//! password only when `password` is among them and before any key, and on
//! the password's refusal offers the keys gwz's credential callback gives it,
//! when `publickey` is among them. Each URL is cloned over both routes, and
//! the server's log of authentication requests must read the same on both.
//! The key-only server is the loopback `sshd`; the others are
//! [`PasswordSshd`], which accepts one fixed test password. As url_parity's
//! test does, each runs in a child whose `HOME` trusts the servers.
use super::*;
use crate::git::endpoint::ssh_password_fixture::{self as password_fixture, PasswordSshd};
use std::path::Path;

const PASSWORD: &str = "fixture-pass-7b1d";
const WRONG: &str = "wrong-pass-91c3";

/// The fixture's key-only `sshd` and its repository, an RSA key the password
/// servers take, and `HOME`'s `known_hosts` trusting the `sshd`.
fn setup() -> (common::SshdFixture, git2::Oid, PathBuf, PathBuf) {
    let f = common::SshdFixture::new();
    let head = commit(
        &git2::Repository::open_bare(&f.repository).unwrap(),
        "first",
    );
    let rsa = f.temp.path().join("client_rsa");
    common::run(
        std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "rsa", "-b", "2048", "-N", ""])
            .arg("-f")
            .arg(&rsa),
    );
    let home = PathBuf::from(std::env::var_os("HOME").expect("the parent sets HOME"));
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::copy(&f.known_hosts, home.join(".ssh/known_hosts")).unwrap();
    (f, head, rsa, home)
}

/// A password server for `methods`, trusted in `home`.
fn server(f: &common::SshdFixture, home: &Path, rsa: &Path, methods: &[&str]) -> PasswordSshd {
    let public = std::fs::read_to_string(rsa.with_extension("pub")).unwrap();
    let server = PasswordSshd::start(
        &f.temp.path().join(methods.join("-")),
        PASSWORD,
        methods,
        &[public],
    );
    let known = home.join(".ssh/known_hosts");
    let mut text = std::fs::read_to_string(&known).unwrap();
    text.push_str(&server.known_host("127.0.0.1"));
    text.push('\n');
    std::fs::write(known, text).unwrap();
    server
}

fn meta(key: &Path) -> crate::RequestMeta {
    crate::RequestMeta {
        request_id: "password".into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(crate::TransportOptions {
            default_identity: Some(key.display().to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Clones `url` into `target` with `backend`'s identity for `meta`: the head
/// it reached, or its error, and the operation's transport rows.
fn clone(
    backend: &Git2Backend,
    f: &common::SshdFixture,
    meta: &crate::RequestMeta,
    url: &str,
    target: &str,
) -> (Result<git2::Oid, String>, Vec<crate::TransportObservation>) {
    let scoped = backend
        .with_transport(f.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let reached = scoped
        .clone_repo(url, &f.temp.path().join(target))
        .map_err(|error| error.to_string())
        .map(|result| {
            git2::Repository::open(result.path)
                .unwrap()
                .head()
                .unwrap()
                .target()
                .unwrap()
        });
    (reached, scoped.transport_observations().unwrap().snapshot())
}

#[test]
fn candidate_url_passwords_authenticate_as_libgit2_uses_them() {
    if std::env::var_os(host_context::CHILD).is_none() {
        host_context::run_in_child_with(
            module_path!(),
            "candidate_url_passwords_authenticate_as_libgit2_uses_them",
            &[(password_fixture::PYTHON, password_fixture::interpreter())],
        );
        return;
    }
    let (f, head, rsa, home) = setup();
    let both = server(&f, &home, &rsa, &["password", "publickey"]);
    let password_only = server(&f, &home, &rsa, &["password"]);
    let keys_only = server(&f, &home, &rsa, &["publickey"]);
    let ed25519 = f.temp.path().join("client_ed25519");
    // The server, the URL's password, whether the clone succeeds, and the
    // requests the server sees, on both routes alike.
    let cases: [(Option<&PasswordSshd>, &str, bool, &[&str]); 6] = [
        // The key-only sshd: the password is not offered, so keys authenticate.
        (None, WRONG, true, &[]),
        (Some(&both), PASSWORD, true, &["none", "password:accepted"]),
        (
            Some(&both),
            WRONG,
            true,
            &[
                "none",
                "password:refused",
                "publickey:query",
                "publickey:accepted",
            ],
        ),
        (
            Some(&password_only),
            WRONG,
            false,
            &["none", "password:refused"],
        ),
        (
            Some(&password_only),
            PASSWORD,
            true,
            &["none", "password:accepted"],
        ),
        (
            Some(&keys_only),
            PASSWORD,
            true,
            &["none", "publickey:query", "publickey:accepted"],
        ),
    ];
    let mut differences = Vec::new();
    for (index, (server, password, succeeds, expected)) in cases.into_iter().enumerate() {
        let (port, key) = server.map_or((f.port, &ed25519), |s| (s.port, &rsa));
        let url = format!(
            "ssh://{}:{password}@127.0.0.1:{port}{}",
            f.user,
            f.repository.display()
        );
        let meta = meta(key);
        let attempts = || {
            server.map(|server| {
                let seen = server.attempts();
                server.clear();
                seen
            })
        };
        let ((transport, rows), cleanup) = crate::transport_host::with_local_transport(
            meta.clone(),
            format!("password-{index}"),
            |backend| clone(backend, &f, &meta, &url, &format!("transport-{index}")),
        )
        .unwrap();
        assert_eq!(cleanup.pending_local_work, 0);
        let on_transport = attempts();
        let (native, _) = clone(
            &Git2Backend::new(),
            &f,
            &meta,
            &url,
            &format!("native-{index}"),
        );
        let on_native = attempts();
        let expected = server.map(|_| expected.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        for (route, outcome, seen) in [
            ("transport", &transport, &on_transport),
            ("native", &native, &on_native),
        ] {
            if outcome.as_ref().is_ok_and(|reached| *reached == head) != succeeds
                || *seen != expected
            {
                differences.push(format!(
                    "case {index}: {route} {outcome:?} saw {seen:?}, expected success={succeeds} {expected:?}"
                ));
            }
        }
        // The password is a secret: no error and no transport row holds it.
        let shown = format!("{transport:?} {rows:?}");
        if shown.contains(PASSWORD) || shown.contains(WRONG) {
            differences.push(format!("case {index}: the transport showed the password"));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn candidate_a_url_password_authenticates_only_its_own_connection() {
    if std::env::var_os(host_context::CHILD).is_none() {
        host_context::run_in_child_with(
            module_path!(),
            "candidate_a_url_password_authenticates_only_its_own_connection",
            &[(password_fixture::PYTHON, password_fixture::interpreter())],
        );
        return;
    }
    let (f, head, rsa, home) = setup();
    let both = server(&f, &home, &rsa, &["password", "publickey"]);
    let at = |password: &str| {
        format!(
            "ssh://{}{password}@127.0.0.1:{}{}",
            f.user,
            both.port,
            f.repository.display()
        )
    };
    let meta = meta(&rsa);
    // A password open, an open of the same user and host without one, and
    // the password open again: no connection serves another of them.
    let (rows, cleanup) =
        crate::transport_host::with_local_transport(meta.clone(), "isolated".into(), |backend| {
            let mut rows = Vec::new();
            for (index, url) in [
                at(&format!(":{PASSWORD}")),
                at(""),
                at(&format!(":{PASSWORD}")),
            ]
            .iter()
            .enumerate()
            {
                let (reached, seen) = clone(backend, &f, &meta, url, &format!("isolated-{index}"));
                assert_eq!(reached, Ok(head), "clone {index}");
                rows.extend(seen);
            }
            rows
        })
        .unwrap();
    assert_eq!(cleanup.pending_local_work, 0);
    assert_eq!(rows.len(), 3, "{rows:?}");
    let connections: std::collections::BTreeSet<_> =
        rows.iter().map(|row| row.connection_id.clone()).collect();
    assert_eq!(connections.len(), 3, "{rows:?}");
    assert!(rows.iter().all(|row| row.reused == Some(false)), "{rows:?}");
    assert_eq!(
        both.attempts(),
        [
            "none",
            "password:accepted",
            "publickey:query",
            "publickey:accepted",
            "none",
            "password:accepted"
        ]
    );
}
