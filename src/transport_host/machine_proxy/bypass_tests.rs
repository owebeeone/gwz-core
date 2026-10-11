//! The bypass list and the implicit bypass against an https destination (design §6, "Bypass list", "HTTPS origins"
//! and "Implicit bypass"; plan step 4.6). Names are matched as text and never resolved.

use super::bypass::Bypass;
use std::net::IpAddr;

/// The machine's own addresses in these tests: two IPv4 on different interfaces and one IPv6.
fn own() -> Vec<IpAddr> {
    ["10.1.2.3", "192.168.0.9", "fd00::5"]
        .iter()
        .map(|text| text.parse().unwrap())
        .collect()
}

fn bypass(list: &str) -> Bypass {
    Bypass::parse(list, own()).unwrap_or_else(|reason| panic!("{list:?} was refused: {reason}"))
}

/// `(host, port)` -> is it sent around the proxy, under `list`.
fn goes_direct(list: &str, host: &str, port: u16) -> bool {
    bypass(list).bypasses(host, port)
}

#[test]
fn the_implicit_bypass_holds_with_no_list_and_with_an_unrelated_one() {
    for list in [
        "",
        "unrelated.example",
        "<-loopback>",
        "*.corp.example;intranet.example:8443",
    ] {
        for host in [
            "localhost",
            "LOCALHOST",
            "127.0.0.1",
            "127.9.9.9",
            "127.255.255.254",
            "::1",
        ] {
            // B09: numeric loopback origins bypass even with an empty list (OQ11, decided and asserted here).
            assert!(goes_direct(list, host, 443), "{list:?} {host}");
        }
        // The machine's own addresses, more than one, IPv4 and IPv6 (S9, b35, b36).
        for host in ["10.1.2.3", "192.168.0.9", "fd00::5"] {
            assert!(goes_direct(list, host, 443), "{list:?} {host}");
            assert!(goes_direct(list, host, 8443), "{list:?} {host}");
        }
    }
}

#[test]
fn what_the_implicit_bypass_does_not_cover_goes_to_the_proxy() {
    for host in [
        // `localhost.` with a trailing dot (x01, b22) and a name that merely resolves to loopback (b04).
        "localhost.",
        "gwz-tr18.localhost",
        "localhost.example",
        // Addresses that are not this machine's own (x05, x06, x07) and the unspecified address.
        "10.1.2.4",
        "192.0.2.1",
        "2001:db8::1",
        "fd00::6",
        "0.0.0.0",
        "128.0.0.1",
        // An IPv4-mapped IPv6 loopback is not the IPv4 loopback as text.
        "::ffff:127.0.0.1",
        "example.org",
    ] {
        assert!(!goes_direct("", host, 443), "{host}");
    }
}

#[test]
fn the_own_address_rule_uses_the_addresses_it_was_given_and_never_a_name() {
    let none = Bypass::parse("", vec![]).unwrap();
    assert!(!none.bypasses("10.1.2.3", 443));
    assert!(none.bypasses("127.0.0.1", 443));
    let one = Bypass::parse("", vec!["203.0.113.7".parse().unwrap()]).unwrap();
    assert!(one.bypasses("203.0.113.7", 443));
    assert!(!one.bypasses("203.0.113.8", 443));
    // A host name is not looked up, so a name that stands for an own address goes to the proxy.
    assert!(!one.bypasses("this-machine.example", 443));
}

#[test]
fn names_in_the_list_match_without_case_and_across_the_whole_host() {
    assert!(goes_direct("github.com", "github.com", 443)); // hb01
    assert!(goes_direct("GITHUB.COM", "github.com", 443)); // hb10
    assert!(goes_direct("github.com", "GitHub.Com", 443));
    assert!(!goes_direct("github.com", "www.github.com", 443));
    assert!(!goes_direct("github.com", "github.com.evil.example", 443));
    assert!(!goes_direct("hub.com", "github.com", 443));
}

#[test]
fn a_wildcard_is_a_glob_over_the_complete_host_and_spans_dots() {
    assert!(goes_direct("git*.com", "github.com", 443)); // hb06
    assert!(goes_direct("*", "anything.example", 443)); // hb07
    assert!(goes_direct("*", "10.9.9.9", 443));
    assert!(goes_direct("*", "2001:db8::1", 443));
    assert!(goes_direct("*.localhost", "gwz-tr18.localhost", 443)); // b09
    assert!(goes_direct("*localhost", "gwz-tr18.localhost", 443)); // b11
    assert!(goes_direct("gwz-tr18.*", "gwz-tr18.localhost", 443)); // b13
    assert!(goes_direct("gwz-*.localhost", "gwz-tr18.localhost", 443)); // b15
    assert!(goes_direct("*.example", "a.b.c.example", 443)); // spans labels
    assert!(!goes_direct("*.github.com", "github.com", 443)); // hb11
    assert!(goes_direct("*.github.com", "api.github.com", 443));
    // Numeric hosts are matched as text (b33, x22).
    assert!(goes_direct("127.*", "127.5.5.5", 443));
    assert!(goes_direct("192.0.2.*", "192.0.2.1", 443));
    assert!(!goes_direct("192.0.2.*", "192.0.3.1", 443));
    // A leading dot and a bare `localhost` do not match the dotted name (x12, x14).
    assert!(!goes_direct(".localhost", "gwz-tr18.localhost", 443));
    assert!(!goes_direct("localhost", "gwz-tr18.localhost", 443));
}

#[test]
fn a_trailing_dot_compares_literally() {
    // x08, x09, x10 bypass; b19, b21 do not; b20, x11 do not.
    assert!(goes_direct("github.com.", "github.com.", 443));
    assert!(goes_direct("*", "github.com.", 443));
    assert!(!goes_direct("github.com", "github.com.", 443));
    assert!(!goes_direct("github.com.", "github.com", 443));
    assert!(!goes_direct("*.github.com", "github.com.", 443));
}

#[test]
fn a_port_in_an_entry_matches_only_that_origin_port() {
    assert!(goes_direct("github.com:443", "github.com", 443)); // hb04
    assert!(!goes_direct("github.com:1", "github.com", 443)); // hb05
    assert!(!goes_direct("github.com:443", "github.com", 8443));
    assert!(
        goes_direct("github.com", "github.com", 8443),
        "no port: any port"
    );
    assert!(goes_direct("[2001:db8::1]:8080", "2001:db8::1", 8080)); // x15
    assert!(!goes_direct("[2001:db8::1]:8080", "2001:db8::1", 443));
    assert!(goes_direct("[2001:db8::1]", "2001:db8::1", 443));
}

#[test]
fn the_scheme_prefix_reverses_for_an_https_url() {
    assert!(goes_direct("https://github.com", "github.com", 443)); // hb02
    assert!(!goes_direct("http://github.com", "github.com", 443)); // hb03
    assert!(goes_direct("https://github.com:443", "github.com", 443));
    assert!(!goes_direct("https://github.com:1", "github.com", 443));
    assert!(goes_direct("HTTPS://GitHub.com", "github.com", 443));
    assert!(!goes_direct("http://*", "github.com", 443));
}

#[test]
fn local_matches_a_name_with_no_dot_and_nothing_else() {
    assert!(goes_direct("<local>", "intranet", 443)); // b08
    assert!(goes_direct("<LOCAL>", "intranet", 443));
    assert!(!goes_direct("<local>", "github.com", 443)); // hb08, b06
    assert!(!goes_direct("<local>", "localhost.", 443)); // b22
    assert!(!goes_direct("<local>", "192.0.2.1", 443)); // x06
    assert!(!goes_direct("<local>", "2001:db8::1", 443));
}

#[test]
fn the_loopback_token_is_accepted_and_has_no_effect() {
    // x18, x19, x20: it does not remove the implicit bypass, and it bypasses nothing of its own.
    assert!(goes_direct("<-loopback>", "localhost", 443));
    assert!(goes_direct("<-loopback>", "127.0.0.1", 443));
    assert!(!goes_direct("<-loopback>", "example.org", 443));
}

#[test]
fn entries_separate_on_semicolons_commas_and_white_space() {
    for list in [
        "a.example;b.example",
        "a.example,b.example",
        "a.example b.example",
        "a.example\tb.example",
        " a.example ;; b.example , ",
        "a.example; b.example",
    ] {
        assert!(goes_direct(list, "a.example", 443), "{list:?}");
        assert!(goes_direct(list, "b.example", 443), "{list:?}");
        assert!(!goes_direct(list, "c.example", 443), "{list:?}");
    }
}

#[test]
fn a_list_outside_the_grammar_is_refused_and_never_read_as_direct() {
    for (list, why) in [
        ("http://", "empty"),
        ("https://", "empty"),
        ("ftp://host.example", "scheme"),
        ("host.example/path", "character"),
        ("user@host.example", "character"),
        ("host.example?x", "character"),
        ("host%41.example", "character"),
        ("<foo>", "<"),
        ("<local", "<"),
        ("host.example:port", "port"),
        ("host.example:0", "port"),
        ("host.example:65536", "port"),
        ("host.example:*", "port"),
        ("host.example:1:2", "port"),
        ("[::1", "bracket"),
        ("[]:80", "empty"),
        ("2001:db8::1", "port"),
        ("host.example:", "port"),
        (":80", "empty"),
    ] {
        match Bypass::parse(list, own()) {
            Err(reason) => assert!(reason.contains(why), "{list:?}: {reason:?} lacks {why:?}"),
            Ok(_) => panic!("{list:?} was not refused"),
        }
    }
}

#[test]
fn an_oversized_list_is_refused() {
    let list = vec!["host.example"; 4096].join(";");
    assert!(Bypass::parse(&list, own()).is_err());
}
