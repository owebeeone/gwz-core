//! The bypass list of the machine setting and the implicit bypass: which https destinations skip the proxy.
//!
//! The destination is the canonical host and port of the connection, matched as text and never resolved: a name that
//! stands for a loopback or an own address goes to the proxy, because deciding otherwise would take a lookup (design
//! §6). Nothing here goes direct for a form it could not read; the list is parsed whole or refused.

use std::net::{IpAddr, Ipv6Addr};

/// The longest bypass list read; a longer one is outside the grammar.
const MAX_LEN: usize = 8192;

/// The destinations that go direct: the stated list, and loopback and this machine's own addresses.
#[derive(Debug)]
pub(crate) struct Bypass {
    rules: Vec<Rule>,
    own: Vec<IpAddr>,
}

#[derive(Debug)]
enum Rule {
    /// `<local>`: a name with no dot.
    Local,
    /// `<-loopback>`: accepted and given no effect, as WinHTTP gives it (design §6, reading R12).
    LoopbackToken,
    Host {
        /// False for an `http://` entry, which does not match an https URL (RX/hb03).
        https: bool,
        /// A glob over the complete host: `*` spans dots and labels. Lower case.
        pattern: Vec<u8>,
        port: Option<u16>,
    },
}

impl Bypass {
    /// The rules of `list` (empty for none), with `own` the machine's interface addresses, or why the list is
    /// outside the grammar.
    pub(super) fn parse(list: &str, own: Vec<IpAddr>) -> Result<Self, String> {
        if list.len() > MAX_LEN {
            return Err(format!("is longer than {MAX_LEN} characters"));
        }
        if list.chars().any(|c| c.is_control() && !c.is_whitespace()) {
            return Err("has a control character".into());
        }
        let rules = list
            .split(|c: char| c == ';' || c == ',' || c.is_whitespace())
            .filter(|entry| !entry.is_empty())
            .map(rule)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { rules, own })
    }

    /// Whether a connection to `host:port` skips the proxy. `host` is the canonical destination host as text (an
    /// IPv6 literal has no brackets), and is never resolved.
    pub(crate) fn bypasses(&self, host: &str, port: u16) -> bool {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        self.implicit(host) || self.rules.iter().any(|rule| matches_rule(rule, host, port))
    }

    /// The implicit bypass, which no list removes: `localhost` in any case, every address in 127/8, `::1`, and an
    /// IP-literal host equal to one of this machine's own addresses (RP/b01 to b03, b35, b36; the own-address rule
    /// is the design's S9). `localhost.` with a trailing dot is not covered (b22).
    fn implicit(&self, host: &str) -> bool {
        if host.eq_ignore_ascii_case("localhost") {
            return true;
        }
        match host.parse::<IpAddr>() {
            Ok(address) => {
                let loopback = match address {
                    IpAddr::V4(v4) => v4.octets()[0] == 127,
                    IpAddr::V6(v6) => v6 == Ipv6Addr::LOCALHOST,
                };
                loopback || self.own.contains(&address)
            }
            Err(_) => false,
        }
    }
}

fn matches_rule(rule: &Rule, host: &str, port: u16) -> bool {
    match rule {
        Rule::LoopbackToken => false,
        Rule::Local => !host.is_empty() && !host.contains('.') && host.parse::<IpAddr>().is_err(),
        Rule::Host {
            https,
            pattern,
            port: wanted,
        } => *https && wanted.is_none_or(|wanted| wanted == port) && glob(pattern, host.as_bytes()),
    }
}

/// `*` matches any run of characters, dots included; everything else matches itself without regard to ASCII case.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p].eq_ignore_ascii_case(&text[t]) {
            p += 1;
            t += 1;
        } else if let Some((star_at, from)) = star {
            p = star_at + 1;
            t = from + 1;
            star = Some((star_at, from + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&b| b == b'*')
}

/// One entry of the list.
fn rule(entry: &str) -> Result<Rule, String> {
    if entry.starts_with('<') {
        return if entry.eq_ignore_ascii_case("<local>") {
            Ok(Rule::Local)
        } else if entry.eq_ignore_ascii_case("<-loopback>") {
            Ok(Rule::LoopbackToken)
        } else {
            Err(
                "has an entry in <...> that GWZ does not read (only <local> and <-loopback> are)"
                    .into(),
            )
        };
    }
    let (https, rest) = match entry.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => (false, rest),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => (true, rest),
        Some(_) => {
            return Err("has an entry with a URL scheme other than http:// or https://".into());
        }
        None => (true, entry),
    };
    if rest.is_empty() {
        return Err("has an entry with a scheme and an empty host".into());
    }
    let (host, port) = match rest.strip_prefix('[') {
        Some(inside) => bracketed(inside)?,
        None => plain(rest)?,
    };
    let port = port.map(port_number).transpose()?;
    if host.is_empty() {
        return Err("has an entry with an empty host".into());
    }
    Ok(Rule::Host {
        https,
        pattern: host.to_ascii_lowercase().into_bytes(),
        port,
    })
}

/// `addr]` or `addr]:port` after a `[`.
fn bracketed(rest: &str) -> Result<(&str, Option<&str>), String> {
    let Some((inside, after)) = rest.split_once(']') else {
        return Err("has an entry with an unclosed bracket".into());
    };
    if !inside
        .chars()
        .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.' | '*'))
    {
        return Err("has a character in brackets that is not part of an IPv6 address".into());
    }
    match after {
        "" => Ok((inside, None)),
        _ => after
            .strip_prefix(':')
            .map(|port| (inside, Some(port)))
            .ok_or_else(|| "has text after the bracketed host that is not a port".to_owned()),
    }
}

/// A name pattern with an optional `:port`.
fn plain(rest: &str) -> Result<(&str, Option<&str>), String> {
    let mut parts = rest.split(':');
    let host = parts.next().unwrap_or_default();
    let port = parts.next();
    if parts.next().is_some() {
        return Err("has a malformed port or an IPv6 address with no brackets".into());
    }
    if host
        .chars()
        .any(|c| !(c.is_alphanumeric() || matches!(c, '-' | '.' | '_' | '*')))
    {
        return Err("has a character that is not part of a host name pattern".into());
    }
    Ok((host, port))
}

fn port_number(text: &str) -> Result<u16, String> {
    let digits = !text.is_empty() && text.len() <= 5 && text.bytes().all(|b| b.is_ascii_digit());
    match text.parse::<u16>() {
        Ok(port) if digits && port != 0 => Ok(port),
        _ => Err("has a malformed port: it must be a number from 1 to 65535".into()),
    }
}

impl crate::git::endpoint::https_connection::ProxyBypass for Bypass {
    fn bypasses(&self, host: &str, port: u16) -> bool {
        Bypass::bypasses(self, host, port)
    }
}
