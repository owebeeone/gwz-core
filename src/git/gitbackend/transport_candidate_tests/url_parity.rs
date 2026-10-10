//! TR2.16: a URL reaches the same SSH repository on the transport as on
//! libgit2's native route, 1.0.17's network path, and where libgit2 refuses a
//! URL before connecting, the transport refuses it before any open. Each URL
//! is cloned from the disposable SSH fixture on both routes. The repository
//! names hold the URLs' literal characters, and a decoy holds the name that
//! decoding the path would reach. As TR2.11's test does, it runs in a child
//! whose `HOME` trusts the fixture, here also at its IPv4-mapped IPv6 address,
//! which an IPv6 socket reaches on the fixture's 127.0.0.1 listener.
use super::*;

// A differential against 1.0.17's own route (libgit2 with libssh2), which on Windows cannot negotiate with the
// fixture's sshd.exe at all ("Unable to exchange encryption keys", dabeest, 2026-10-11), and whose `~`-relative paths
// reach a Windows profile, not a root. Its Windows twin waits for the key-exchange and host-key work of step 3.8.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn candidate_ssh_urls_reach_on_the_transport_what_libgit2_reaches() {
            if std::env::var_os(host_context::CHILD).is_none() {
                host_context::run_in_child(
                    module_path!(),
                    "candidate_ssh_urls_reach_on_the_transport_what_libgit2_reaches",
                );
                return;
            }
            let f = common::SshdFixture::new();
            let home = PathBuf::from(std::env::var_os("HOME").expect("the parent sets HOME"));
            std::fs::create_dir_all(home.join(".ssh")).unwrap();
            let trusted = std::fs::read_to_string(&f.known_hosts).unwrap();
            let mapped = trusted.replacen("[127.0.0.1]", "[::ffff:127.0.0.1]", 1);
            let known_hosts = format!("{}\n{}\n", trusted.trim_end(), mapped.trim_end());
            std::fs::write(home.join(".ssh/known_hosts"), known_hosts).unwrap();
            let mut names = std::collections::BTreeMap::new();
            for name in ["a%20b.git", "a b.git", "plain.git"] {
                let repository = git2::Repository::init_bare(f.temp.path().join(name)).unwrap();
                names.insert(commit(&repository, name), name);
            }
            let (user, port, dir) = (&f.user, f.port, common::server_path(f.temp.path()));
            // From the remote user's home up to the root, from any depth up to 16.
            let up = "/..".repeat(16);
            let reached = [
                (
                    format!("ssh://{user}@127.0.0.1:{port}{dir}/a%20b.git"),
                    "a%20b.git",
                ),
                (
                    format!("ssh://{user}@127.0.0.%31:{port}{dir}/plain.git"),
                    "plain.git",
                ),
                (
                    format!("ssh://{user}@127.0.0.1:{port}{dir}/plain.git?q#f"),
                    "plain.git",
                ),
                (
                    format!("ssh://{user}:@127.0.0.1:{port}{dir}/plain.git"),
                    "plain.git",
                ),
                (
                    format!("ssh://{user}@127.0.0.1:{port}/~{up}{dir}/plain.git"),
                    "plain.git",
                ),
                (
                    format!("[{user}@127.0.0.1:{port}]:{dir}/a%20b.git"),
                    "a%20b.git",
                ),
                (
                    format!("[{user}@127.0.0.1:{port}]:/~{up}{dir}/plain.git"),
                    "plain.git",
                ),
                (
                    format!("[{user}@127.0.0.1:{port}]:~{user}{up}{dir}/plain.git"),
                    "plain.git",
                ),
            ];
            // Each with the reason libgit2 gives before it connects.
            let refused = [
                (
                    format!("ssh://{user}@[::ffff:127.0.0.1]:{port}{dir}/plain.git"),
                    "malformed hostname",
                ),
                (
                    format!("[{user}@[::ffff:127.0.0.1]:{port}]:{dir}/plain.git"),
                    "failed to resolve address",
                ),
                (
                    format!("[{user}@127.0.0.1:{port}]:-plain.git"),
                    "ambiguous with command-line option",
                ),
            ];
            let urls: Vec<&str> = reached
                .iter()
                .chain(&refused)
                .map(|(url, _)| url.as_str())
                .collect();
            let meta = crate::RequestMeta {
                request_id: "urls".into(),
                schema_version: "gwz.protocol/v0".into(),
                transport: Some(crate::TransportOptions {
                    default_identity: Some(f.temp.path().join("client_ed25519").display().to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let (transport, cleanup) =
                crate::transport_host::with_local_transport(meta.clone(), "urls".into(), |backend| {
                    clone_each(backend, &f, &meta, &urls, "transport")
                })
                .unwrap();
            assert_eq!(cleanup.pending_local_work, 0);
            let native = clone_each(&Git2Backend::new(), &f, &meta, &urls, "native");
            let name_of = |head: &Result<git2::Oid, String>| {
                head.as_ref().ok().and_then(|head| names.get(head).copied())
            };
            let refusal = |head: &Result<git2::Oid, String>, reason: &str| {
                head.as_ref().is_err_and(|error| error.contains(reason))
            };
            // Every URL whose outcome differs from its expectation, on either route.
            let mut differences = Vec::new();
            let outcomes: Vec<_> = native.iter().zip(&transport).collect();
            for ((url, name), (native, transport)) in reached.iter().zip(&outcomes) {
                if name_of(native) != Some(*name) || name_of(transport) != Some(*name) {
                    let (n, t) = (name_of(native), name_of(transport));
                    differences.push(format!(
                        "{url} reaches {name}: native {n:?} {native:?}, transport {t:?} {transport:?}"
                    ));
                }
            }
            for ((url, reason), (native, transport)) in refused.iter().zip(&outcomes[reached.len()..]) {
                if !refusal(native, reason) || !refusal(transport, "invalid SSH destination") {
                    differences.push(format!(
                        "{url} is refused: native {native:?}, transport {transport:?}"
                    ));
                }
            }
            assert!(differences.is_empty(), "{}", differences.join("\n"));
        }
    }
}

/// Clones each URL into a directory of its own, returning each clone's head.
fn clone_each(
    backend: &Git2Backend,
    f: &common::SshdFixture,
    meta: &crate::RequestMeta,
    urls: &[&str],
    route: &str,
) -> Vec<Result<git2::Oid, String>> {
    let scoped = backend
        .with_transport(f.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let clone = |(index, url): (usize, &&str)| -> Result<git2::Oid, String> {
        let target = f.temp.path().join(format!("{route}-{index}"));
        let result = scoped
            .clone_repo(url, &target)
            .map_err(|error| error.to_string())?;
        Ok(git2::Repository::open(result.path)
            .unwrap()
            .head()
            .unwrap()
            .target()
            .unwrap())
    };
    urls.iter().enumerate().map(clone).collect()
}
