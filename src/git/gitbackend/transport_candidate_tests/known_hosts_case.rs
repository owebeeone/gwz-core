//! TR2.18: host-name case in `known_hosts`, on both routes. 1.0.17's libgit2
//! gives libssh2 the host as the URL wrote it, and libssh2 compares a plain
//! name byte for byte and hashes a hashed name's host as given. The transport
//! matches a plain name ignoring case, and a hashed name on the host as the
//! URL wrote it or lowercased. So it trusts every pairing 1.0.17 trusts, and
//! a lowercase name, plain or hashed, under a mixed-case URL host too. As
//! url_parity's test does, it runs in a child whose `HOME` holds each
//! `known_hosts` in turn; `LocalHost` and `localhost` reach the fixture's
//! 127.0.0.1 listener.
use super::*;

#[test]
fn candidate_known_hosts_names_match_as_written_or_ignoring_case() {
    if std::env::var_os(host_context::CHILD).is_none() {
        host_context::run_in_child(
            module_path!(),
            "candidate_known_hosts_names_match_as_written_or_ignoring_case",
        );
        return;
    }
    let f = common::SshdFixture::new();
    let home = PathBuf::from(std::env::var_os("HOME").expect("the parent sets HOME"));
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    let head = commit(
        &git2::Repository::open_bare(&f.repository).unwrap(),
        "first",
    );
    let meta = crate::RequestMeta {
        request_id: "case".into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(crate::TransportOptions {
            default_identity: Some(f.temp.path().join("client_ed25519").display().to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    // The known_hosts name, whether it is hashed, the URL's host, and
    // whether 1.0.17 and the transport trust the server.
    let cases = [
        ("LocalHost", false, "LocalHost", true, true),
        ("localhost", false, "LocalHost", false, true),
        ("LocalHost", false, "localhost", false, true),
        ("LOCALHOST", false, "LocalHost", false, true),
        ("LocalHost", true, "LocalHost", true, true),
        ("localhost", true, "LocalHost", false, true),
        ("localhost", true, "localhost", true, true),
        ("LocalHost", true, "localhost", false, false),
        ("LocalHost", true, "LOCALHOST", false, false),
    ];
    let mut differences = Vec::new();
    for (index, (name, hashed, host, native, transport)) in cases.into_iter().enumerate() {
        std::fs::write(
            home.join(".ssh/known_hosts"),
            f.known_host(name, f.port, hashed) + "\n",
        )
        .unwrap();
        let url = format!(
            "ssh://{}@{host}:{}{}",
            f.user,
            f.port,
            f.repository.display()
        );
        let clone = |backend: &Git2Backend, route: &str| -> Result<git2::Oid, String> {
            let scoped = backend
                .with_transport(f.temp.path(), meta.transport.as_ref())
                .unwrap()
                .unwrap();
            let target = f.temp.path().join(format!("{route}-{index}"));
            let result = scoped
                .clone_repo(&url, &target)
                .map_err(|error| error.to_string())?;
            Ok(git2::Repository::open(result.path)
                .unwrap()
                .head()
                .unwrap()
                .target()
                .unwrap())
        };
        let (on_transport, cleanup) = crate::transport_host::with_local_transport(
            meta.clone(),
            format!("case-{index}"),
            |backend| clone(backend, "transport"),
        )
        .unwrap();
        assert_eq!(cleanup.pending_local_work, 0);
        let on_native = clone(&Git2Backend::new(), "native");
        let hashing = if hashed { "hashed " } else { "" };
        for (route, outcome, trusted) in [
            ("native", &on_native, native),
            ("transport", &on_transport, transport),
        ] {
            if outcome.as_ref().is_ok_and(|reached| *reached == head) != trusted {
                differences.push(format!(
                    "{hashing}{name} for {host}: {route} {outcome:?}, expected trusted={trusted}"
                ));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
