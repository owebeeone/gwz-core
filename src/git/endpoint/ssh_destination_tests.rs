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
fn a_password_beside_a_user_is_kept_as_libgit2_decodes_it() {
    for (url, user, password) in [
        ("ssh://u:pw@host/a", "u", &b"pw"[..]),
        ("git+ssh://u:p%40w%3A%2F@host/a", "u", b"p@w:/"),
        // The password follows the userinfo's last ':', and a user may hold '@'.
        ("ssh://u:p:w@host/a", "u:p", b"w"),
        ("ssh://a@b:pw@host/a", "a@b", b"pw"),
        // Bytes, as libgit2 decodes them; libssh2 gets a C string.
        ("ssh://u:%FF%FE@host/a", "u", b"\xff\xfe"),
        ("ssh://u:p%00w@host/a", "u", b"p"),
        ("ssh://u:%zz@host/a", "u", b"%zz"),
    ] {
        let target = Destination::parse(url).unwrap().unwrap();
        assert_eq!(target.key, Key::ssh(user, "host", 22), "{url}");
        assert_eq!(
            target.password.as_ref().map(|p| p.bytes()),
            Some(password),
            "{url}"
        );
    }
    // No user: libgit2 asks the callback for one and drops the password. No
    // password: none is offered. An scp-like spelling has no password.
    for url in [
        "ssh://:pw@host/a",
        "ssh://u:@host/a",
        "ssh://u@host/a",
        "us:pw@host:a",
    ] {
        let target = Destination::parse(url).unwrap().unwrap();
        assert!(target.password.is_none(), "{url}");
    }
}

#[test]
fn no_debug_form_or_error_shows_a_password() {
    let target = Destination::parse("ssh://u:sentinel@host/a")
        .unwrap()
        .unwrap();
    assert!(!format!("{target:?}").contains("sentinel"));
    for url in ["ssh://u:sentinel@host:0/a", "ssh://u:sentinel@[::1x]/a"] {
        let error = Destination::parse(url).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("sentinel"), "{url}");
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
        "ssh://u:sentinel@host/a\n",
        "ssh://u:sentinel@host:0/a",
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
