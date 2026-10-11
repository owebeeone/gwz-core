//! The proxy-server grammar against an https URL (design §6, "Proxy string" and "HTTPS origins"; plan step 4.6).

use super::proxy_string::{DEFAULT_PORT, ProxyEntry, Selection, select_https};

fn entry(host: &str, port: u16) -> Selection {
    Selection::Https(ProxyEntry {
        host: host.into(),
        port,
    })
}

#[test]
fn the_forms_winhttp_applies_to_an_https_url_select_their_entry() {
    for (text, host, port) in [
        // A bare entry (RX/h01, g01, g02), with a URL scheme (h07), upper-case (g15), a trailing `;` (x16).
        ("proxy.example:8080", "proxy.example", 8080),
        ("10.0.0.1:3128", "10.0.0.1", 3128),
        ("http://proxy.example:8080", "proxy.example", 8080),
        ("HTTP://PROXY.EXAMPLE:8080", "proxy.example", 8080),
        ("proxy.example:8080;", "proxy.example", 8080),
        // An explicit entry, alone or beside another, in either order (h02, h04, h05), with a URL scheme (h08).
        ("https=proxy.example:8080", "proxy.example", 8080),
        (
            "http=other.example:1;https=proxy.example:2",
            "proxy.example",
            2,
        ),
        (
            "https=proxy.example:2;http=other.example:1",
            "proxy.example",
            2,
        ),
        ("https=http://proxy.example:2", "proxy.example", 2),
        ("HTTPS=PROXY.EXAMPLE:8080", "proxy.example", 8080),
        // What `netsh` stores for a bare entry beside a mapping (h09) and the explicit entry that wins (h10).
        (
            "http=proxy.example:8080;https=proxy.example:8080;ftp=proxy.example:8080",
            "proxy.example",
            8080,
        ),
        ("http=other.example:1;proxy.example:2", "proxy.example", 2),
        ("proxy.example:3;https=other.example:4", "other.example", 4),
        ("https=other.example:4;proxy.example:3", "other.example", 4),
        // Bracketed IPv6 literals, bare and in a mapping (g11, h11), which the connector takes without brackets.
        ("[::1]:8080", "::1", 8080),
        ("https=[2001:db8::1]:8080", "2001:db8::1", 8080),
        // WinHTTP separates on `;` and on white space.
        (
            "http=other.example:1 https=proxy.example:2",
            "proxy.example",
            2,
        ),
    ] {
        assert_eq!(select_https(text), Ok(entry(host, port)), "{text}");
    }
}

#[test]
fn an_entry_with_no_port_takes_the_default_port_and_is_never_direct() {
    // g10 shows the attempt and not the number (design §6); the transport's choice is a connection it can fail.
    assert_eq!(
        select_https("proxy.example"),
        Ok(entry("proxy.example", DEFAULT_PORT))
    );
    assert_eq!(
        select_https("https=proxy.example"),
        Ok(entry("proxy.example", DEFAULT_PORT))
    );
    assert_eq!(
        select_https("[2001:db8::1]"),
        Ok(entry("2001:db8::1", DEFAULT_PORT))
    );
}

#[test]
fn a_mapping_with_no_entry_for_https_is_well_formed_and_selects_nothing() {
    // h03, h06 and g05, g14: 1.0.17 goes direct. The transport reports it so that the capture can refuse (D1).
    for text in [
        "http=proxy.example:8080",
        "socks=proxy.example:1080",
        "http=a.example:1;socks=b.example:2",
        "http=a.example:1;ftp=b.example:2",
        "ftp=proxy.example:21",
    ] {
        assert_eq!(select_https(text), Ok(Selection::NoHttpsEntry), "{text}");
    }
}

#[test]
fn a_value_outside_the_grammar_is_refused_with_its_reason() {
    for (text, why) in [
        // More than one bare entry: ambiguous (g12, g13).
        ("a.example:1;b.example:2", "more than one"),
        ("a.example:1 b.example:2", "more than one"),
        // The same scheme named twice: more than one applicable proxy.
        ("https=a.example:1;https=b.example:2", "more than once"),
        ("http=a.example:1;http=b.example:2", "more than once"),
        // A malformed port.
        ("proxy.example:eighty", "port"),
        ("proxy.example:0", "port"),
        ("proxy.example:65536", "port"),
        ("proxy.example:", "port"),
        ("proxy.example:80:90", "port"),
        ("https=proxy.example:-1", "port"),
        // Userinfo, a path, a query or a fragment.
        ("user@proxy.example:8080", "user"),
        ("user:secret@proxy.example:8080", "user"),
        ("https=user@proxy.example:8080", "user"),
        ("proxy.example:8080/path", "path"),
        ("http://proxy.example:8080/", "path"),
        ("proxy.example:8080?x=1", "path"),
        ("proxy.example:8080#x", "path"),
        // A proxy URL scheme other than http://.
        ("https://proxy.example:8080", "scheme"),
        ("socks5://proxy.example:1080", "scheme"),
        ("https=https://proxy.example:8080", "scheme"),
        ("ftp://proxy.example:21", "scheme"),
        // A mapping key that is not a scheme WinHTTP knows, or that is empty.
        ("gopher=proxy.example:70", "scheme"),
        ("=proxy.example:8080", "scheme"),
        ("https=", "empty"),
        // A host that is empty or not a host.
        (":8080", "host"),
        ("[::1", "host"),
        ("[]:8080", "host"),
        ("[::1]x:8080", "host"),
        ("::1", "port"),
        ("proxy..example", "host"),
        // Nothing at all.
        ("", "empty"),
        (" ; ", "empty"),
    ] {
        match select_https(text) {
            Err(reason) => assert!(reason.contains(why), "{text:?}: {reason:?} lacks {why:?}"),
            other => panic!("{text:?} was not refused: {other:?}"),
        }
    }
}

#[test]
fn control_characters_and_oversized_values_are_refused() {
    for text in ["proxy.example:80\u{7}", "proxy\0.example:80", "p\u{85}:80"] {
        assert!(select_https(text).is_err(), "{text:?}");
    }
    let long = format!("{}.example:80", "a".repeat(4096));
    assert!(select_https(&long).is_err());
}
