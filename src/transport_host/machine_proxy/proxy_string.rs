//! The proxy-server string of the machine setting: which entry serves an https URL, or why the string is refused.
//!
//! The string is what `WinHttpGetDefaultProxyConfiguration` returns, after `netsh` normalised it (design §6): entries
//! separate on `;` or white space, and an entry is `host`, `host:port` or `http://host:port`, bare or after
//! `scheme=`. The entry for the URL's scheme applies; a bare entry applies to a scheme with no entry of its own.

use std::{collections::BTreeMap, net::Ipv6Addr};

/// The proxy that serves https connections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProxyEntry {
    /// The host in the text form the connector resolves: an IPv6 literal has no brackets.
    pub(crate) host: String,
    pub(crate) port: u16,
}

/// The port WinHTTP is taken to use for an entry that names none. The number was not measured on 1.0.17 (design
/// §6, the Proxy string bullet); a wrong guess fails to connect, and nothing falls back to a direct connection.
pub(super) const DEFAULT_PORT: u16 = 80;

/// The longest proxy string read; a longer one is outside the grammar.
const MAX_LEN: usize = 4096;

/// The scheme names WinHTTP's proxy list uses for its mapped entries.
const MAPPED_SCHEMES: [&str; 4] = ["http", "https", "ftp", "socks"];

/// What the string says about https.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Selection {
    Https(ProxyEntry),
    /// A well-formed list with no entry for https (`http=` only, `socks=` only): refused (decision D1).
    NoHttpsEntry,
}

/// The entry for an https URL in `proxy`, by WinHTTP's rule, or the reason the string is outside the grammar. Every
/// entry is read, used or not: a form GWZ cannot read is refused, not skipped.
pub(super) fn select_https(proxy: &str) -> Result<Selection, String> {
    if proxy.len() > MAX_LEN {
        return Err(format!("is longer than {MAX_LEN} characters"));
    }
    if proxy.chars().any(|c| c.is_control() && !c.is_whitespace()) {
        return Err("has a control character".into());
    }
    let mut bare = Vec::new();
    let mut mapped: BTreeMap<String, ProxyEntry> = BTreeMap::new();
    let mut seen = false;
    for token in proxy
        .split(|c: char| c == ';' || c.is_whitespace())
        .filter(|token| !token.is_empty())
    {
        seen = true;
        match split_mapping(token) {
            (Some(key), value) => {
                let key = key.to_ascii_lowercase();
                if !MAPPED_SCHEMES.contains(&key.as_str()) {
                    return Err(if key.is_empty() {
                        "has an entry with no scheme before its \"=\"".into()
                    } else {
                        format!(
                            "has an entry for the scheme \"{}\", which GWZ does not read",
                            key.chars().take(32).collect::<String>()
                        )
                    });
                }
                let entry = endpoint(value)?;
                if mapped.insert(key.clone(), entry).is_some() {
                    return Err(format!("names the scheme {key} more than once"));
                }
            }
            (None, value) => bare.push(endpoint(value)?),
        }
    }
    if !seen {
        return Err("is empty: it names no proxy".into());
    }
    if bare.len() > 1 {
        return Err("has more than one entry with no scheme, which is ambiguous".into());
    }
    Ok(match mapped.remove("https").or_else(|| bare.pop()) {
        Some(entry) => Selection::Https(entry),
        None => Selection::NoHttpsEntry,
    })
}

/// `scheme=value` into its parts; a token that is a URL (`http://host:80`) or has no `=` is a bare value.
fn split_mapping(token: &str) -> (Option<&str>, &str) {
    match token.split_once('=') {
        Some((key, value)) if !key.contains(['/', ':', '@', '[', '?', '#']) => (Some(key), value),
        _ => (None, token),
    }
}

/// One `[http://]host[:port]` value.
fn endpoint(value: &str) -> Result<ProxyEntry, String> {
    if value.is_empty() {
        return Err("has an empty entry".into());
    }
    let value = match value.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
        Some(_) => {
            return Err(
                "has a proxy URL scheme other than http://, which GWZ does not support".into(),
            );
        }
        None => value,
    };
    if value.contains('@') {
        return Err("has a user name or password (userinfo) in a proxy entry".into());
    }
    if value.contains(['/', '?', '#', '\\']) {
        return Err("has a path, query or fragment in a proxy entry".into());
    }
    let (host, port) = match value.strip_prefix('[') {
        Some(rest) => bracketed(rest)?,
        None => plain(value)?,
    };
    Ok(ProxyEntry {
        host,
        port: port.map_or(Ok(DEFAULT_PORT), port_number)?,
    })
}

/// An IPv6 literal after its `[`: `addr]` or `addr]:port`.
fn bracketed(rest: &str) -> Result<(String, Option<&str>), String> {
    let Some((inside, after)) = rest.split_once(']') else {
        return Err("has a host with an unclosed bracket".into());
    };
    if inside.parse::<Ipv6Addr>().is_err() {
        return Err("has a host in brackets that is not an IPv6 address".into());
    }
    match after {
        "" => Ok((inside.to_ascii_lowercase(), None)),
        _ => match after.strip_prefix(':') {
            Some(port) => Ok((inside.to_ascii_lowercase(), Some(port))),
            None => Err("has text after the bracketed host that is not a port".into()),
        },
    }
}

/// A host name or IPv4 literal, with an optional port.
fn plain(value: &str) -> Result<(String, Option<&str>), String> {
    let mut parts = value.split(':');
    let host = parts.next().unwrap_or_default();
    let port = parts.next();
    if parts.next().is_some() {
        return Err("has a malformed port or an IPv6 address with no brackets".into());
    }
    if !host_name(host) {
        return Err(if host.is_empty() {
            "has an entry with no host".into()
        } else {
            "has a host that is not a host name or an address".into()
        });
    }
    Ok((host.to_ascii_lowercase(), port))
}

/// Letters, digits, `-` and `_` in non-empty labels, with at most one trailing dot.
fn host_name(host: &str) -> bool {
    let name = host.strip_suffix('.').unwrap_or(host);
    !name.is_empty()
        && host.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

fn port_number(text: &str) -> Result<u16, String> {
    let digits = !text.is_empty() && text.len() <= 5 && text.bytes().all(|b| b.is_ascii_digit());
    match text.parse::<u16>() {
        Ok(port) if digits && port != 0 => Ok(port),
        _ => Err("has a malformed port: it must be a number from 1 to 65535".into()),
    }
}
