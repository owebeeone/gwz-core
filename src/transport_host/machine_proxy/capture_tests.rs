//! Capturing the machine proxy from an injected source (plan step 4.6): the refusals and their texts, a read made once,
//! and the machine setting's place beside the environment in the endpoint configuration.

use super::testing::Fixed;
use super::{Access, MachineProxy};
use crate::session_host::EnvironmentSnapshot;
use std::ffi::OsString;

fn refusal(machine: &MachineProxy) -> &str {
    match machine {
        MachineProxy::Refused(text) => text,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn direct_access_is_direct_and_enumerates_no_addresses() {
    let source = Fixed::new(Access::Direct, None, None);
    assert!(matches!(source.capture(), MachineProxy::Direct));
    assert_eq!((source.reads.get(), source.enumerations.get()), (1, 0));
}

#[test]
fn a_named_proxy_is_read_once_and_the_addresses_are_listed_once() {
    let source = Fixed::named("proxy.example:8080", Some("intranet;10.1.2.3"));
    let MachineProxy::Named(named) = source.capture() else {
        panic!("a named proxy");
    };
    assert_eq!(
        (named.proxy.host.as_str(), named.proxy.port),
        ("proxy.example", 8080)
    );
    assert_eq!((source.reads.get(), source.enumerations.get()), (1, 1));
    // The addresses the capture listed are the own-address rule's, with more than one (S9).
    assert!(named.bypass.bypasses("10.1.2.3", 443));
    assert!(named.bypass.bypasses("192.168.0.9", 443));
    assert!(!named.bypass.bypasses("10.1.2.4", 443));
    assert!(named.bypass.bypasses("intranet", 443));
}

#[test]
fn a_direct_setting_that_also_names_a_proxy_or_a_bypass_list_is_refused() {
    for (proxy, bypass) in [
        (Some("proxy.example:80"), None),
        (None, Some("*.corp")),
        (Some(""), None),
    ] {
        let text = refusal(&Fixed::new(Access::Direct, proxy, bypass).capture()).to_owned();
        assert!(
            text.starts_with("The Windows machine proxy setting"),
            "{text}"
        );
    }
}

#[test]
fn automatic_configuration_and_an_unreadable_setting_are_refused_naming_the_setting() {
    let automatic = Fixed::new(Access::Automatic, None, None).capture();
    let text = refusal(&automatic);
    assert!(
        text.starts_with("The Windows machine proxy setting"),
        "{text}"
    );
    assert!(text.contains("automatic"), "{text}");
    assert!(text.contains("PAC"), "{text}");
    let unreadable = Fixed::new(Access::Unreadable, None, None).capture();
    let text = refusal(&unreadable);
    assert!(
        text.starts_with("The Windows machine proxy setting"),
        "{text}"
    );
    assert!(text.contains("could not be read"), "{text}");
}

#[test]
fn a_named_setting_with_no_proxy_string_is_refused() {
    let text = refusal(&Fixed::new(Access::Named, None, None).capture()).to_owned();
    assert!(
        text.starts_with("The Windows machine proxy setting"),
        "{text}"
    );
    assert!(text.contains("names no proxy"), "{text}");
}

#[test]
fn a_mapping_with_no_entry_for_https_is_refused_with_text_t1_before_any_open() {
    for setting in ["http=proxy:8080", "socks=proxy:1080", "http=a:1;ftp=b:2"] {
        let source = Fixed::named(setting, Some("*.corp.example"));
        let machine = source.capture();
        // T1 of design §14 item 13, verbatim: the administrator's fix first, `--transport native` only if a direct
        // connection is acceptable, no fallback.
        let expected = format!(
            "The Windows machine proxy setting has no entry for https: \"{setting}\". GWZ does not send https traffic around a proxy that was set up for other protocols only. Ask your administrator to add an https entry, for example netsh winhttp set proxy proxy-server=\"http=proxy:8080;https=proxy:8080\". Only if a direct connection is acceptable on your network, --transport native connects directly as gwz 1.0 did."
        );
        assert_eq!(refusal(&machine), expected);
        let text = refusal(&machine);
        assert!(
            text.find("Ask your administrator").unwrap() < text.find("--transport native").unwrap()
        );
    }
}

#[test]
fn a_value_outside_the_grammar_is_refused_naming_the_setting_and_the_form() {
    for (setting, bypass, form) in [
        ("a:1;b:2", None, "more than one"),
        ("proxy:eighty", None, "port"),
        ("user:pw@proxy:80", None, "user"),
        ("proxy:80/x", None, "path"),
        ("https://proxy:80", None, "scheme"),
        ("proxy:80", Some("host/path"), "character"),
        ("proxy:80", Some("<foo>"), "<"),
    ] {
        let machine = Fixed::named(setting, bypass).capture();
        let text = refusal(&machine);
        assert!(
            text.starts_with("The Windows machine proxy setting"),
            "{text}"
        );
        assert!(text.contains(form), "{setting} {bypass:?}: {text}");
        assert!(
            text.contains(setting) || bypass.is_some_and(|b| text.contains(b)),
            "{text}"
        );
        assert!(
            text.contains("never connects around"),
            "no direct fallback is promised: {text}"
        );
    }
}

#[test]
fn a_refusal_quotes_the_setting_as_bounded_escaped_text() {
    let machine = Fixed::named(&format!("http=a\u{1b}[31m\n{}", "a".repeat(2000)), None).capture();
    let text = refusal(&machine);
    assert!(!text.chars().any(char::is_control), "{text:?}");
    assert!(text.len() < 1500, "{}", text.len());
    assert!(text.contains("\\u{1b}"), "{text}");
}

#[test]
fn the_machine_setting_wins_over_the_environment_and_git_configuration() {
    // On Windows no environment proxy is read at all; on Unix the snapshot below would set one. With a machine
    // setting, whichever it is, `endpoint_config_native` takes the proxy from it alone and a bad variable is moot.
    let snapshot = EnvironmentSnapshot::from_os_pairs([
        (
            OsString::from("HOME"),
            std::env::temp_dir().into_os_string(),
        ),
        (OsString::from("HTTPS_PROXY"), OsString::from("not a url")),
        (
            OsString::from("ALL_PROXY"),
            OsString::from("http://env.example:1"),
        ),
        (OsString::from("NO_PROXY"), OsString::from("*")),
        (
            OsString::from("http.proxy"),
            OsString::from("http://git.example:2"),
        ),
    ])
    .unwrap();
    let direct = Fixed::new(Access::Direct, None, None).capture();
    let (_, https) = crate::transport_host::endpoint_environment::endpoint_config_native(
        &snapshot,
        None,
        Some(direct),
    )
    .expect("a direct machine setting ignores the environment");
    assert!(https.tls.proxy.is_none());
    assert!(https.tls.no_proxy.is_empty());
}

#[test]
fn a_refused_setting_refuses_the_endpoint_configuration_naming_the_setting() {
    let snapshot = EnvironmentSnapshot::from_os_pairs([(
        OsString::from("HOME"),
        std::env::temp_dir().into_os_string(),
    )])
    .unwrap();
    let machine = Fixed::named("http=proxy:8080", None).capture();
    let error = crate::transport_host::endpoint_environment::endpoint_config_native(
        &snapshot,
        None,
        Some(machine),
    )
    .err()
    .expect("T1 refuses the configuration");
    assert!(
        error
            .message
            .starts_with("The Windows machine proxy setting has no entry for https"),
        "{}",
        error.message
    );
}

#[test]
fn a_named_machine_proxy_is_the_endpoints_proxy_and_the_environment_is_not_read() {
    // A bad HTTPS_PROXY would refuse the Unix configuration; with a machine setting it is never reached.
    let snapshot = EnvironmentSnapshot::from_os_pairs([
        (
            OsString::from("HOME"),
            std::env::temp_dir().into_os_string(),
        ),
        (OsString::from("HTTPS_PROXY"), OsString::from("not a url")),
        (
            OsString::from("ALL_PROXY"),
            OsString::from("http://env.example:1"),
        ),
        (OsString::from("NO_PROXY"), OsString::from("proxy.example")),
    ])
    .unwrap();
    let machine = Fixed::named("proxy.example:8080", Some("intranet")).capture();
    let (_, https) = crate::transport_host::endpoint_environment::endpoint_config_native(
        &snapshot,
        None,
        Some(machine),
    )
    .expect("the machine setting alone decides");
    let proxy = https.tls.proxy.as_ref().expect("the machine proxy");
    assert_eq!(
        (proxy.host.as_str(), proxy.port, proxy.tls),
        ("proxy.example", 8080, false)
    );
    assert!(proxy.authorization.is_none(), "no proxy credential exists");
    assert!(https.tls.no_proxy.is_empty(), "NO_PROXY is not merged");
    // The machine bypass list decides, not NO_PROXY: `proxy.example` is not on it, `intranet` is.
    assert!(https.tls.proxy_for("proxy.example", 443).is_some());
    assert!(https.tls.proxy_for("intranet", 443).is_none());
    assert!(https.tls.proxy_for("localhost", 443).is_none());
}

#[test]
fn a_direct_machine_setting_clears_any_proxy_already_in_the_configuration() {
    use crate::git::endpoint::https_connection::{Config, Proxy};
    let mut config = Config {
        proxy: Some(Proxy {
            host: "env.example".into(),
            port: 1,
            tls: false,
            authorization: None,
        }),
        no_proxy: vec!["x".into()],
        ..Config::default()
    };
    Fixed::new(Access::Direct, None, None)
        .capture()
        .apply(&mut config)
        .unwrap();
    assert!(
        config.proxy.is_none() && config.no_proxy.is_empty() && config.machine_bypass.is_none()
    );
    assert!(config.proxy_for("example.org", 443).is_none());
}

#[test]
fn the_selection_sends_this_machines_own_addresses_direct_and_every_other_address_to_the_proxy() {
    // The addresses are the injected source's two (10.1.2.3 and 192.168.0.9): the own-address rule is tested with
    // more than one (S9), through the endpoint's own selection and not the matcher alone.
    use crate::git::endpoint::https_connection::Config;
    let mut config = Config::default();
    Fixed::named("proxy.example:8080", None)
        .capture()
        .apply(&mut config)
        .unwrap();
    for own in ["10.1.2.3", "192.168.0.9", "127.0.0.1", "::1", "localhost"] {
        assert!(config.proxy_for(own, 443).is_none(), "{own}");
    }
    for other in [
        "10.1.2.4",
        "192.168.0.10",
        "192.0.2.1",
        "2001:db8::1",
        "localhost.",
        "github.com",
    ] {
        let proxy = config.proxy_for(other, 443).expect(other);
        assert_eq!((proxy.host.as_str(), proxy.port), ("proxy.example", 8080));
    }
}
