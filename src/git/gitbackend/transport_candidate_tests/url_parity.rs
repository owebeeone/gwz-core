//! TR2.16: a URL reaches the same SSH repository on the transport as on
//! libgit2's native route, 1.0.17's network path, and where libgit2 refuses a
//! URL before connecting, the transport refuses it before any open. Each URL
//! is cloned from the disposable SSH fixture on both routes. The repository
//! names hold the URLs' literal characters, and a decoy holds the name that
//! decoding the path would reach. As TR2.11's test does, it runs in a child
//! whose `HOME` trusts the fixture, here also at its IPv4-mapped IPv6 address,
//! which an IPv6 socket reaches on the fixture's 127.0.0.1 listener.
use super::*;

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
    // From the remote user's home up to the root, from any depth up to 16, and down to the fixture. Where the fixture's
    // directory cannot be reached from the home (Windows, on another drive), the home-relative forms are left out.
    let up = "/..".repeat(16);
    let from_home = home_relative_dir(&dir, f.temp.path());
    // URLs that neither route reaches a repository through, for want of the server's help (see `USER_HOME_REACHES`).
    let mut unreached: Vec<String> = Vec::new();
    let mut reached = vec![
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
            format!("[{user}@127.0.0.1:{port}]:{dir}/a%20b.git"),
            "a%20b.git",
        ),
    ];
    if let Some(below) = &from_home {
        reached.extend([
            (
                format!("ssh://{user}@127.0.0.1:{port}/~{up}{below}/plain.git"),
                "plain.git",
            ),
            (
                format!("[{user}@127.0.0.1:{port}]:/~{up}{below}/plain.git"),
                "plain.git",
            ),
        ]);
        let user_home = format!("[{user}@127.0.0.1:{port}]:~{user}{up}{below}/plain.git");
        if USER_HOME_REACHES {
            reached.push((user_home, "plain.git"));
        } else {
            unreached.push(user_home);
        }
    } else {
        eprintln!("skipped the home-relative forms: the fixture is not on the remote home's drive");
    }
    // Each with the reason libgit2 gives before it connects.
    let refused = [
        (
            format!("ssh://{user}@[::ffff:127.0.0.1]:{port}{dir}/plain.git"),
            "malformed hostname",
        ),
        (
            format!("[{user}@[::ffff:127.0.0.1]:{port}]:{dir}/plain.git"),
            RESOLVE_REFUSAL,
        ),
        (
            format!("[{user}@127.0.0.1:{port}]:-plain.git"),
            "ambiguous with command-line option",
        ),
    ];
    let urls: Vec<&str> = reached
        .iter()
        .map(|(url, _)| url.as_str())
        .chain(unreached.iter().map(String::as_str))
        .chain(refused.iter().map(|(url, _)| url.as_str()))
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
    let after = reached.len() + unreached.len();
    for (url, (native, transport)) in unreached.iter().zip(&outcomes[reached.len()..after]) {
        if name_of(native).is_some() || name_of(transport).is_some() {
            differences.push(format!(
                "{url} reaches nothing on either route: native {native:?}, transport {transport:?}"
            ));
        }
    }
    for ((url, reason), (native, transport)) in refused.iter().zip(&outcomes[after..]) {
        if !refusal(native, reason) || !refusal(transport, "invalid SSH destination") {
            differences.push(format!(
                "{url} is refused: native {native:?}, transport {transport:?}"
            ));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// Where libgit2 on Windows fails to connect to a bracketed IPv4-mapped IPv6 address of an scp-style URL: it
        /// reports the connect, where the others report the resolution.
        const RESOLVE_REFUSAL: &str = "failed to connect";

        /// Whether `~user/..` reaches the user's home on the server: not on Windows, where Git for Windows has no
        /// account database to find another user's home in, so both routes are told the repository is not there.
        const USER_HOME_REACHES: bool = false;

        /// `dir` (`/c/Users/..`, as the server's shell spells it) as a path below the root that a home-relative walk
        /// reaches: the walk from the profile (`C:\Users\..`) up ends at the drive's root, so the path leaves out the
        /// drive. `None` when the fixture is on another drive than the profile.
        fn home_relative_dir(dir: &str, temp: &Path) -> Option<String> {
            let profile = std::env::var("USERPROFILE").ok()?;
            let (profile_drive, temp_drive) = (profile.chars().next()?, temp.display().to_string().chars().next()?);
            profile_drive
                .eq_ignore_ascii_case(&temp_drive)
                .then(|| dir.get(2..).unwrap_or(dir).to_owned())
        }
    } else {
        /// Where libgit2 refuses a bracketed IPv4-mapped IPv6 address of an scp-style URL before connecting.
        const RESOLVE_REFUSAL: &str = "failed to resolve address";

        /// Whether `~user/..` reaches the user's home on the server.
        const USER_HOME_REACHES: bool = true;

        /// `dir` as a home-relative walk from the remote user's home reaches it: up to the root, then down.
        fn home_relative_dir(dir: &str, _temp: &Path) -> Option<String> {
            Some(dir.to_owned())
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
