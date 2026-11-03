//! The server address (plan step 2.3e): the grammar of the parked server design §3 row by row, with `auto` refused as
//! `GwzSplitCommandChannel.md` §4.1 says, judged under every platform's grammar on every host, and Windows' checks after
//! the grammar with injected answers. The walk's rows, and the real Windows checks, are in `address_walk_rows`.
use super::super::local::address::{
    AddressRefused, PIPE_DEVICE, PIPE_NAME_MAX, Platform, Reason, SERVER_ADDRESS_REFUSED,
    ServerAddress, Source, SshDestination, SshPart, WindowsFault, judge_full_path,
    judge_pipe_device, parse_for,
};
use std::ffi::OsStr;

const PLATFORMS: [Platform; 3] = [Platform::Linux, Platform::Macos, Platform::Windows];
const SOURCES: [Source; 3] = [Source::Flag, Source::Environment, Source::Bridge];

fn refused(platform: Platform, source: Source, text: &str) -> Reason {
    match parse_for(platform, source, text.as_bytes()) {
        Ok(address) => panic!("{text:?} ({platform:?}, {source:?}) was accepted as {address:?}"),
        Err(reason) => reason,
    }
}

fn accepted(platform: Platform, source: Source, text: &str) -> ServerAddress {
    parse_for(platform, source, text.as_bytes()).unwrap_or_else(|reason| {
        panic!("{text:?} ({platform:?}, {source:?}) was refused: {reason}")
    })
}

#[test]
fn each_source_accepts_its_forms() {
    for source in SOURCES {
        for platform in [Platform::Linux, Platform::Macos] {
            assert_eq!(
                accepted(platform, source, "/run/user/501/gwz/ws-0123456789abcdef"),
                ServerAddress::Socket(b"/run/user/501/gwz/ws-0123456789abcdef".to_vec())
            );
        }
        assert_eq!(
            accepted(
                Platform::Windows,
                source,
                r"\\.\pipe\gwz-0123456789abcdef0123456789abcdef"
            ),
            ServerAddress::Pipe(r"\\.\pipe\gwz-0123456789abcdef0123456789abcdef".to_owned())
        );
    }
    for platform in PLATFORMS {
        assert_eq!(
            accepted(platform, Source::Flag, "stdio"),
            ServerAddress::Stdio
        );
        assert_eq!(
            accepted(platform, Source::Environment, "stdio"),
            ServerAddress::Stdio
        );
        assert_eq!(
            accepted(platform, Source::Flag, "ssh://git@build.example/srv/ws"),
            ServerAddress::Ssh(SshDestination {
                user: Some("git".to_owned()),
                host: "build.example".to_owned(),
                port: None,
                path: "/srv/ws".to_owned(),
            })
        );
    }
}

#[test]
fn auto_is_refused_in_every_source_with_its_reason() {
    for platform in PLATFORMS {
        for source in SOURCES {
            assert_eq!(refused(platform, source, "auto"), Reason::Auto);
        }
    }
    let refusal = ServerAddress::parse(Source::Flag, OsStr::new("auto")).unwrap_err();
    assert_eq!(refusal.code(), SERVER_ADDRESS_REFUSED);
    let text = refusal.to_string();
    assert!(
        text.contains("--server") && text.contains("\"auto\""),
        "{text}"
    );
    assert!(text.contains("no auto address in this release"), "{text}");
}

#[test]
fn an_empty_flag_is_refused_and_an_empty_environment_is_unset() {
    for platform in PLATFORMS {
        for source in SOURCES {
            assert_eq!(refused(platform, source, ""), Reason::Empty);
        }
    }
    assert_eq!(ServerAddress::from_environment(None), Ok(None));
    assert_eq!(
        ServerAddress::from_environment(Some(OsStr::new(""))),
        Ok(None)
    );
    assert_eq!(
        ServerAddress::from_environment(Some(OsStr::new("stdio"))),
        Ok(Some(ServerAddress::Stdio))
    );
    // A set value the grammar refuses is refused, not skipped.
    let refusal = ServerAddress::from_environment(Some(OsStr::new("auto"))).unwrap_err();
    assert_eq!(
        (refusal.source, refusal.reason),
        (Source::Environment, Reason::Auto)
    );
}

#[test]
fn stdio_and_ssh_are_refused_outside_their_sources() {
    for platform in PLATFORMS {
        assert_eq!(
            refused(platform, Source::Bridge, "stdio"),
            Reason::NotFromSource
        );
        for source in [Source::Environment, Source::Bridge] {
            assert_eq!(
                refused(platform, source, "ssh://host/srv/ws"),
                Reason::NotFromSource
            );
        }
    }
}

#[test]
fn relative_paths_and_other_words_are_refused() {
    for text in [
        "run/gwz/ws",
        "./ws",
        "../ws",
        "ws",
        "server",
        "Auto",
        "STDIO",
        "host:srv/ws",
        "SSH://h/p",
        "~/ws",
    ] {
        for platform in [Platform::Linux, Platform::Macos] {
            assert_eq!(
                refused(platform, Source::Flag, text),
                Reason::Relative,
                "{text}"
            );
        }
        assert_eq!(
            refused(Platform::Windows, Source::Flag, text),
            Reason::NotAPipe,
            "{text}"
        );
    }
}

#[test]
fn empty_dot_and_dot_dot_components_are_refused() {
    for text in [
        "/",
        "/run//gwz",
        "/run/./gwz",
        "/run/../gwz",
        "/run/gwz/",
        "//run/gwz",
        "/run/gwz/.",
        "/run/gwz/..",
    ] {
        for platform in [Platform::Linux, Platform::Macos] {
            assert_eq!(
                refused(platform, Source::Flag, text),
                Reason::Component,
                "{text}"
            );
        }
    }
    // A name that only starts with dots is an ordinary component.
    accepted(Platform::Linux, Source::Flag, "/run/.gwz/..ws");
}

#[test]
fn a_nul_and_an_abstract_name_are_refused() {
    assert_eq!(
        refused(Platform::Linux, Source::Flag, "@gwz"),
        Reason::Abstract
    );
    assert_eq!(
        refused(Platform::Linux, Source::Flag, "\0gwz"),
        Reason::Abstract
    );
    assert_eq!(refused(Platform::Macos, Source::Flag, "\0gwz"), Reason::Nul);
    assert_eq!(
        refused(Platform::Macos, Source::Flag, "@gwz"),
        Reason::Relative
    );
    for platform in [Platform::Linux, Platform::Macos] {
        assert_eq!(refused(platform, Source::Flag, "/run/g\0wz"), Reason::Nul);
    }
}

#[test]
fn a_socket_path_holds_at_most_the_platform_limit() {
    for (platform, limit) in [(Platform::Linux, 107), (Platform::Macos, 103)] {
        assert_eq!(platform.socket_path_max(), Some(limit));
        let fits = format!("/{}", "a".repeat(limit - 1));
        accepted(platform, Source::Flag, &fits);
        let over = format!("/{}", "a".repeat(limit));
        assert_eq!(
            refused(platform, Source::Flag, &over),
            Reason::TooLong {
                length: limit + 1,
                limit
            }
        );
    }
}

#[test]
fn macos_refuses_the_paths_its_kernel_rewrites() {
    for text in [
        "/.vol/16777220/2/ws",
        "/.nofollow/tmp/ws",
        "/.resolve/1/tmp/ws",
    ] {
        assert_eq!(
            refused(Platform::Macos, Source::Flag, text),
            Reason::Rewritten,
            "{text}"
        );
        accepted(Platform::Linux, Source::Flag, text);
    }
    // Only the prefix with its separator: the directory itself is ordinary.
    accepted(Platform::Macos, Source::Flag, "/.volume/ws");
}

#[test]
fn every_windows_spelling_that_reaches_another_machine_is_refused() {
    for text in [
        r"\\host\share\ws",
        r"\\host\pipe\gwz",
        r"\\localhost\pipe\gwz",
        "//host/pipe/gwz",
        r"\/host\pipe\gwz",
        r"/\host\pipe\gwz",
        r"\\?\UNC\host\share",
        r"\\.\UNC\host\share",
        r"\??\UNC\host\share",
        r"\\?\unc\host\share",
        r"\\?\GLOBALROOT\Device\Mup\host\share",
        r"\\.\GLOBALROOT\Device\NamedPipe\gwz",
        r"\\;LanmanRedirector\host\share",
        r"\\;WebDavRedirector\host\share",
        r"\\host@SSL\share",
        r"\\host@SSL@443\share",
        r"\\host@8080\share",
        r"\\host\DavWWWRoot\share",
        r"\\.\pipe\..\UNC\h\s",
        r"\\.\pipe\x\..\..\UNC\h\s",
    ] {
        assert_eq!(
            refused(Platform::Windows, Source::Flag, text),
            Reason::Remote,
            "{text}"
        );
    }
}

#[test]
fn windows_drives_and_other_devices_are_refused() {
    for text in [r"\\.\Z:\pipe\gwz", r"\\?\C:\ws", r"\??\z:\ws", "//./Z:/ws"] {
        assert_eq!(
            refused(Platform::Windows, Source::Flag, text),
            Reason::Drive,
            "{text}"
        );
    }
    for text in [r"\\.\COM1", r"\\.\mailslot\gwz", r"\\?\Volume{0}\ws"] {
        assert_eq!(
            refused(Platform::Windows, Source::Flag, text),
            Reason::Device,
            "{text}"
        );
    }
    for text in [r"C:\gwz\ws", "/tmp/gwz/ws", r"\gwz"] {
        assert_eq!(
            refused(Platform::Windows, Source::Flag, text),
            Reason::NotAPipe,
            "{text}"
        );
    }
}

#[test]
fn every_other_spelling_of_a_local_pipe_is_refused() {
    for text in [
        r"\\?\pipe\gwz",
        r"\??\pipe\gwz",
        r"\\.\PIPE\gwz",
        r"\\.\Pipe\gwz",
        r"\\.\pipe/gwz",
        r"\\.\pipe\gwz.",
        r"\\.\pipe\gwz ",
        r"\\.\pipe\a\b",
        r"\\.\pipe\a/b",
        r"\\.\pipe\",
        r"\\.\pipe\a b",
        "\\\\.\\pipe\\a\u{ff3c}b",
        "\\\\.\\pipe\\a\0",
        r"\\.\pipe\a:b",
    ] {
        assert_eq!(
            refused(Platform::Windows, Source::Flag, text),
            Reason::PipeSpelling,
            "{text:?}"
        );
    }
}

#[test]
fn a_pipe_name_holds_at_most_256_characters() {
    let name = |total: usize| format!(r"\\.\pipe\{}", "a".repeat(total - 9));
    accepted(Platform::Windows, Source::Flag, &name(PIPE_NAME_MAX));
    assert_eq!(
        refused(Platform::Windows, Source::Flag, &name(PIPE_NAME_MAX + 1)),
        Reason::TooLong {
            length: PIPE_NAME_MAX + 1,
            limit: PIPE_NAME_MAX
        }
    );
    // A pipe name on Linux or macOS is a relative path.
    assert_eq!(
        refused(Platform::Linux, Source::Flag, r"\\.\pipe\gwz"),
        Reason::Relative
    );
}

#[test]
fn the_ssh_form_admits_only_its_allowlists() {
    let ssh = |text: &str| match accepted(Platform::Linux, Source::Flag, text) {
        ServerAddress::Ssh(destination) => destination,
        other => panic!("{text} is {other:?}"),
    };
    let full = ssh("ssh://a.b-c_d@host-1.example:2222/srv/my ws");
    assert_eq!(
        (
            full.user.as_deref(),
            full.host.as_str(),
            full.port,
            full.path.as_str()
        ),
        (Some("a.b-c_d"), "host-1.example", Some(2222), "/srv/my ws")
    );
    let literal = ssh("ssh://[2001:db8::1]:22/ws");
    assert_eq!(
        (literal.host.as_str(), literal.port),
        ("2001:db8::1", Some(22))
    );
    assert_eq!(ssh("ssh://[::1]/ws").port, None);
    for (text, part) in [
        ("ssh://host", SshPart::Path),
        ("ssh://host:22", SshPart::Path),
        ("ssh://host/ws?x", SshPart::Path),
        ("ssh://host/ws#x", SshPart::Path),
        ("ssh://host/w\0s", SshPart::Path),
        ("ssh://@host/ws", SshPart::User),
        ("ssh://-oProxyCommand=x@host/ws", SshPart::User),
        ("ssh://us$er@host/ws", SshPart::User),
        ("ssh://u@@host/ws", SshPart::Host),
        ("ssh://-oProxyCommand=x/ws", SshPart::Host),
        ("ssh:///ws", SshPart::Host),
        ("ssh://ho;st/ws", SshPart::Host),
        ("ssh://h\u{f8}st/ws", SshPart::Host),
        ("ssh://[fe80::1%25eth0]/ws", SshPart::Host),
        ("ssh://[fe80::1%eth0]/ws", SshPart::Host),
        ("ssh://[not-v6]/ws", SshPart::Host),
        ("ssh://[::1]x/ws", SshPart::Host),
        ("ssh://host:/ws", SshPart::Port),
        ("ssh://host:0/ws", SshPart::Port),
        ("ssh://host:65536/ws", SshPart::Port),
        ("ssh://host:+22/ws", SshPart::Port),
        ("ssh://host:2a/ws", SshPart::Port),
    ] {
        assert_eq!(
            refused(Platform::Linux, Source::Flag, text),
            Reason::Ssh(part),
            "{text:?}"
        );
    }
}

#[test]
fn a_refusal_names_its_source_and_escapes_the_address() {
    let refusal =
        ServerAddress::parse(Source::Environment, OsStr::new("ws\u{1b}[2J\nx")).unwrap_err();
    let AddressRefused {
        source,
        shown,
        reason,
    } = refusal.clone();
    assert_eq!(source, Source::Environment);
    assert_ne!(reason, Reason::Auto);
    assert_eq!(shown, r"ws\x1b[2J\x0ax");
    let text = refusal.to_string();
    assert!(
        text.contains("GWZ_SERVER") && text.contains(r"\x1b"),
        "{text}"
    );
    assert!(!text.contains('\u{1b}') && !text.contains('\n'), "{text:?}");
    let bridge = ServerAddress::parse(Source::Bridge, OsStr::new("stdio")).unwrap_err();
    assert!(bridge.to_string().contains("SocketCoreBridge"), "{bridge}");
    // The ssh:// part is named in words.
    let ssh = ServerAddress::parse(Source::Flag, OsStr::new("ssh://us$er@host/ws")).unwrap_err();
    assert!(
        ssh.to_string()
            .ends_with("the ssh:// form's user name is not accepted"),
        "{ssh}"
    );
}

#[test]
fn windows_checks_after_the_grammar_judge_the_full_path_and_the_pipe_device() {
    assert_eq!(judge_full_path(r"\\.\pipe\gwz", r"\\.\pipe\gwz"), Ok(()));
    assert!(matches!(
        judge_full_path(r"\\.\pipe\a\..\b", r"\\.\pipe\b"),
        Err(WindowsFault::Renamed { .. })
    ));
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    assert_eq!(judge_pipe_device(&wide(PIPE_DEVICE)), Ok(()));
    let mut both = wide(PIPE_DEVICE);
    both.extend(wide(r"\Device\Other"));
    assert_eq!(judge_pipe_device(&both), Ok(()));
    // A redefinition in the caller's logon session is listed first, and is refused.
    let mut redefined = wide(r"\??\C:\squat");
    redefined.extend(wide(PIPE_DEVICE));
    assert!(matches!(
        judge_pipe_device(&redefined),
        Err(WindowsFault::PipeDevice { .. })
    ));
    assert!(matches!(
        judge_pipe_device(&[]),
        Err(WindowsFault::PipeDevice { .. })
    ));
    // A refusal after the grammar is `server_address_refused` too.
    let fault = judge_pipe_device(&redefined).unwrap_err();
    assert_eq!(fault.code(), SERVER_ADDRESS_REFUSED);
    assert_eq!(
        fault.to_string(),
        r"the PIPE device is \\??\\C:\\squat, not \Device\NamedPipe"
    );
}
