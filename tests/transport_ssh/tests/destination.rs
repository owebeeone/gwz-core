#![allow(dead_code)]
#[path = "../../../src/git/endpoint/ssh_destination.rs"]
mod ssh_destination;
use ssh_destination::Destination;

#[test]
fn ssh_spellings_share_pool_keys_but_keep_repository_operands() {
    for url in [
        "ssh://git@Host:22/a",
        "git+ssh://git@Host/a",
        "ssh+git://git@Host/a",
        "git@Host:/a",
    ] {
        let target = Destination::parse(url).unwrap().unwrap();
        assert_eq!(target.key.username.as_deref(), Some("git"));
        assert_eq!(target.key.host, "host");
        assert_eq!(target.key.port, 22);
        assert_eq!(target.path, "/a");
    }
    let one = Destination::parse("git@host:one").unwrap().unwrap();
    let two = Destination::parse("git@host:two").unwrap().unwrap();
    assert_eq!(one.key, two.key);
    assert_ne!(one.path, two.path);
    assert_eq!(
        Destination::parse("host:repo")
            .unwrap()
            .unwrap()
            .key
            .username
            .as_deref(),
        Some("git")
    );
}
#[test]
fn paths_preserve_shell_characters_dot_segments_and_decode_urls_exactly_once() {
    for (url, path) in [
        (
            "ssh://git@host/a/../b//c%20d%2520%23%3F",
            "/a/../b//c d%20#?",
        ),
        ("host:a%20b?c#d", "a%20b?c#d"),
        ("ssh://host/~user/repo", "~user/repo"),
        ("host:~user/repo", "~user/repo"),
        (
            "ssh://host/repo'$(touch%20sentinel)'",
            "/repo'$(touch sentinel)'",
        ),
    ] {
        assert_eq!(Destination::parse(url).unwrap().unwrap().path, path);
    }
}
#[test]
fn ipv6_and_nondefault_ports_are_unambiguous() {
    let one = Destination::parse("ssh://git@[::1]:2222/a")
        .unwrap()
        .unwrap();
    assert_eq!(one.key.host, "::1");
    assert_eq!(one.key.port, 2222);
    assert_eq!(
        Destination::parse("git@[::1]:a").unwrap().unwrap().key.port,
        22
    );
    assert_eq!(
        Destination::parse("ssh://us%65r@host/a")
            .unwrap()
            .unwrap()
            .key
            .username
            .as_deref(),
        Some("user")
    );
}
#[test]
fn invalid_ssh_refuses_without_echoing_secrets_and_nonssh_stays_native() {
    for url in [
        "ssh://u:sentinel@host/a",
        "ssh://host/a?sentinel",
        "ssh://host/a#sentinel",
        "ssh://host/",
        "host:",
        "ssh://host:0/a",
        "ssh://host:65536/a",
        "ssh://host/%00",
        "ssh://host/%FF",
        "ssh://host/%x1",
        "ssh://host/a\n",
        "ssh://@host/a",
        "ssh://host",
        "ssh://[bad]/a",
        "ssh://host:xx/a",
        "ssh://host\\bad/a",
        "git@[::1:repo",
    ] {
        let error = Destination::parse(url).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{url}");
        assert!(!error.to_string().contains("sentinel"));
    }
    for url in [
        "https://host/a",
        "http://host/a",
        "git://host/a",
        "file:///a",
        "/tmp/a:b",
        "./a:b",
        "../a:b",
        "C:\\repo",
        "C:/repo",
        "repo",
        "",
    ] {
        assert!(Destination::parse(url).unwrap().is_none(), "{url}");
    }
}
