//! The SSH destination against libgit2 1.9.7, whose SSH transport is 1.0.17's
//! network path: its parsers' own test vectors (`tests/util/url/parse.c` and
//! `scp.c`), `_git_ssh_setup_conn`'s refusal of an option-shaped path and
//! `gen_proto`'s `/~` rule and quoting (`transports/ssh_libssh2.c`).
use super::{ssh_channel::GitService, ssh_destination::Destination};
use gwz_transport::pool::Key;

fn reaches(url: &str, user: &str, host: &str, port: u16, path: &str) {
    let target = Destination::parse(url)
        .unwrap_or_else(|error| panic!("{url}: {error}"))
        .unwrap_or_else(|| panic!("{url} took the native route"));
    assert_eq!(
        (target.key, target.path.as_str()),
        (Key::ssh(user, host, port), path),
        "{url}"
    );
}

#[test]
fn spellings_share_a_pool_key_and_keep_their_paths() {
    for url in [
        "ssh://git@Host:22/a",
        "git+ssh://git@Host/a",
        "ssh+git://git@Host/a",
        "git@Host:/a",
        "Host:/a",
    ] {
        reaches(url, "git", "host", 22, "/a");
    }
}

#[test]
fn the_host_as_written_stays_beside_the_lowercased_key() {
    for (url, written) in [
        ("ssh://git@GitHost.Example:22/a", "GitHost.Example"),
        ("SSH://git@GITHOST.example/a", "GITHOST.example"),
        ("ssh://git@GitHost%2EExample/a", "GitHost.Example"),
        ("git@GitHost.Example:a", "GitHost.Example"),
        ("[GitHost.Example:22]:a", "GitHost.Example"),
        ("ssh://git@[FE80::1]/a", "FE80::1"),
    ] {
        let target = Destination::parse(url).unwrap().unwrap();
        assert_eq!(target.written_host, written, "{url}");
        assert_eq!(target.key.host, written.to_ascii_lowercase(), "{url}");
    }
}

#[test]
fn paths_pass_as_written_but_a_url_query_and_the_slash_before_a_tilde() {
    for (url, path) in [
        (
            "ssh://git@host/a/../b//c%20d%2520%23%3F",
            "/a/../b//c%20d%2520%23%3F",
        ),
        (
            "ssh://host/repo'$(touch%20sentinel)'",
            "/repo'$(touch%20sentinel)'",
        ),
        ("ssh://host/%7Euser/repo", "/%7Euser/repo"),
        ("ssh://host/~user/repo", "~user/repo"),
        ("ssh://host/a?b#c", "/a"),
        ("ssh://host/a#b?c", "/a"),
        ("ssh://host", "/"),
        ("host:a%20b?c#d", "a%20b?c#d"),
        ("host:~user/repo", "~user/repo"),
        ("host:/~user/repo", "~user/repo"),
        ("host:/", "/"),
    ] {
        reaches(url, "git", "host", 22, path);
    }
}

#[test]
fn url_users_and_hosts_decode_as_libgit2_does() {
    for (url, user, host, port) in [
        (
            "ssh://my.email.address@gmail.com@source.developers.google.com:2022/a",
            "my.email.address@gmail.com",
            "source.developers.google.com",
            2022,
        ),
        ("ssh://us%65r%zz%4@host/a", "user%zz%4", "host", 22),
        ("ssh://@host/a", "git", "host", 22),
        ("ssh://u:@host/a", "u", "host", 22),
        ("ssh://:@host/a", "git", "host", 22),
        ("ssh://127.0.0.%31:/a", "git", "127.0.0.1", 22),
        ("SSH://host:0022/a", "git", "host", 22),
        ("ssh://[FE80::1]:99/a", "git", "fe80::1", 99),
        ("git+ssh://[cafe]/a", "git", "cafe", 22),
    ] {
        reaches(url, user, host, port, "/a");
    }
}

#[test]
fn scp_authorities_follow_libgit2s_brackets() {
    for (url, user, host, port, path) in [
        ("[host]:/a", "git", "host", 22, "/a"),
        ("[host:42]:/a", "git", "host", 42, "/a"),
        ("[user@host:42]:/a", "user", "host", 42, "/a"),
        ("[192.168.99.88]:/a", "git", "192.168.99.88", 22, "/a"),
        ("[fe:22]:/a", "git", "fe", 22, "/a"),
        ("[[host:22]:a", "git", "host", 22, "a"),
        ("fe80::1]:/a", "git", "fe80", 22, ":1]:/a"),
        ("ü@host:ä", "ü", "host", 22, "ä"),
    ] {
        reaches(url, user, host, port, path);
    }
}

#[test]
fn what_libgit2_refuses_or_cannot_resolve_is_refused() {
    for url in [
        // An IPv6 scp host keeps its brackets in libgit2, and resolves to nothing.
        "[::1]:/a",
        "git@[::1]:a",
        "[[fe80::1]:99]:/a",
        "[user@[fe80::1]:99]:/a",
        "user@[host:42]:/a",
        "[fe80::1]:42]:/a",
        "ssh://[::ffff:127.0.0.1]/a",
        "ssh://[fe80::1%25en0]/a",
        "ssh://x[::1]/a",
        "ssh://[::1]x/a",
        "ssh://host:22:22/a",
        "ssh://host:xx/a",
        "ssh://host\\x/a",
        "host:-a",
        "[host:22]:--upload-pack=a",
        "example.com:",
        "[example.com:42]:",
        ":foo",
        "git@:foo",
        "[]:",
        "git@[]:",
        "@example.com:foo",
        "[@localhost:22]:foo",
        "[example.com:]:foo",
        "[fe:]:foo",
        "[git@[[fe80::1]]:42]:foo",
        "[[git@[fe80::1]:42]]:foo",
        "[fe80::1:/a",
        "[[fe80::1]:42:/a",
        "[git@[fe80::1:42]:/a",
        "git@[::1:a",
        "host]:a",
        // The transport's bounds, which libgit2 does not have.
        "ssh://u:sentinel@host/a",
        "ssh://host/a\n",
        "host:a\tb",
        "ssh://u%00@host/a",
        "ssh://%FF@host/a",
        "ssh://host:0/a",
        "ssh://host:65536/a",
        "[host:ssh]:a",
        "hö:a",
        "ssh://h%C3%B6/a",
    ] {
        let error = Destination::parse(url).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{url}");
        assert!(!error.to_string().contains("sentinel"));
    }
    for url in [
        "https://host/a",
        "file:///a",
        "/tmp/a:b",
        "a/b:c",
        "C:\\a",
        "C:/a",
        "[a:b]",
        "a",
    ] {
        assert!(Destination::parse(url).unwrap().is_none(), "{url}");
    }
}

#[test]
fn no_spelling_of_up_to_four_characters_panics() {
    let alphabet = ["a", "1", "@", ":", "[", "]", "/", "%", "ü"];
    let mut spellings = vec![String::new()];
    for _ in 0..4 {
        spellings = spellings
            .iter()
            .flat_map(|s| alphabet.iter().map(move |c| format!("{s}{c}")))
            .collect();
        for spelling in &spellings {
            let _ = Destination::parse(spelling);
            let _ = Destination::parse(&format!("ssh://{spelling}"));
        }
    }
}

#[test]
fn the_command_quotes_as_gen_proto_does() {
    assert_eq!(
        GitService::UploadPack.command("/a'b!c").unwrap(),
        r"git-upload-pack '/a'\''b'\!'c'"
    );
}
