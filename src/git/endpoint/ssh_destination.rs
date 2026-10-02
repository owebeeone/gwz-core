//! Side-effect-free SSH admission. Paths are operands, never shell commands.
//! The grammar is libgit2's (1.9.7 `util/net.c` and `transports/ssh_libssh2.c`),
//! 1.0.17's network path: a URL's path passes as written, never decoded.
use super::ssh_handoff::UrlPassword;
use gwz_transport::pool::Key;
use std::{io, net::Ipv6Addr};

#[derive(Debug)]
pub(crate) struct Destination {
    /// The pool key. Its host is the URL's, lowercased.
    pub(crate) key: Key,
    /// The host as the URL wrote it, which a hashed `known_hosts` name may
    /// hash, as libssh2 does for 1.0.17 (TR2.18).
    pub(crate) written_host: String,
    pub(crate) path: String,
    /// The password beside the URL's user, which libgit2 offers when the
    /// server offers password authentication (TR2.18). A secret.
    pub(crate) password: Option<UrlPassword>,
}
/// libgit2's parse: user, password, host, port ("" for the default) and path.
type Parts<'a> = (Option<&'a str>, Option<&'a str>, &'a str, &'a str, &'a str);
impl Destination {
    /// None leaves a non-SSH URL on its existing native/local route. Once an SSH
    /// spelling is recognized, malformed input is an error, never a fallback.
    pub(crate) fn parse(input: &str) -> io::Result<Option<Self>> {
        let ((user, password, host, port, path), escaped) =
            if let Some((scheme, rest)) = input.split_once("://") {
                if !["ssh", "git+ssh", "ssh+git"]
                    .iter()
                    .any(|s| scheme.eq_ignore_ascii_case(s))
                {
                    return Ok(None);
                }
                (url(rest)?, true)
            } else if local(input) {
                return Ok(None);
            } else {
                (scp(input)?, false)
            };
        if input.len() > 18_000 || input.chars().any(char::is_control) {
            return Err(invalid());
        }
        let decoded = |text: &str| {
            if escaped {
                decode(text)
            } else {
                Ok(text.to_owned())
            }
        };
        // libgit2 decodes a URL's password to bytes, as it decodes the user,
        // and hands it to libssh2 as a C string.
        let password = password.map(|password| UrlPassword::new(decode_bytes(password)));
        // libgit2 asks the credential callback for a missing or empty user,
        // and GWZ's callback answers "git".
        let user = match user {
            Some(user) if !user.is_empty() => decoded(user)?,
            _ => "git".to_owned(),
        };
        if user.len() > 128 || user.chars().any(char::is_control) {
            return Err(invalid());
        }
        // A host that keeps brackets, as an scp IPv6 host does, resolves to
        // nothing in libgit2; one with a ':' resolves only as an IPv6 address.
        let host = decoded(host)?;
        if host.is_empty()
            || host.len() > 255
            || !host.is_ascii()
            || host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "@/?#\\%[]".contains(c))
            || (host.contains(':') && host.parse::<Ipv6Addr>().is_err())
        {
            return Err(invalid());
        }
        let port = match port {
            "" => 22,
            value if value.bytes().all(|c| c.is_ascii_digit()) => {
                value.parse::<u16>().map_err(|_| invalid())?
            }
            _ => return Err(invalid()),
        };
        // libgit2 refuses an option-shaped path before it connects, then
        // passes /~user as ~user, where Git resolves home-relative operands.
        if port == 0 || path.starts_with('-') {
            return Err(invalid());
        }
        let path = path
            .strip_prefix('/')
            .filter(|p| p.starts_with('~'))
            .unwrap_or(path);
        if path.is_empty() || path.len() > 16_384 {
            return Err(invalid());
        }
        Ok(Some(Self {
            key: Key::ssh(user, host.to_ascii_lowercase(), port),
            written_host: host,
            path: path.to_owned(),
            password,
        }))
    }
}
fn invalid() -> io::Error {
    // Do not echo a possibly credential-bearing URL in an error or observation.
    io::Error::new(io::ErrorKind::InvalidInput, "invalid SSH destination")
}
/// libgit2's `git_net_url_parse` for an SSH scheme. The authority runs to the
/// first '/' and is read from its end, so a user may contain '@'. The path
/// runs from there to a '?' or '#'; it is "/" when the URL has none.
fn url(rest: &str) -> io::Result<Parts<'_>> {
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
        None => (None, authority),
    };
    // The port is the digits after a last ':', which may be none.
    let (host, port) = match hostport.rfind(|c: char| !c.is_ascii_digit()) {
        Some(colon) if hostport.as_bytes()[colon] == b':' => {
            (&hostport[..colon], &hostport[colon + 1..])
        }
        _ => (hostport, ""),
    };
    let host = match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        Some(inner) if inner.bytes().all(|c| c.is_ascii_hexdigit() || c == b':') => inner,
        _ if host.contains(['[', ']', ':']) => return Err(invalid()),
        _ => host,
    };
    // The userinfo's password follows its last ':'. libgit2 uses a password
    // only beside a user: with no user it asks the credential callback for
    // one, and drops the password.
    let (user, password) = match userinfo.map(|info| info.rsplit_once(':').unwrap_or((info, ""))) {
        Some((user, password)) if !user.is_empty() && !password.is_empty() => {
            (Some(user), Some(password))
        }
        other => (other.map(|(user, _)| user), None),
    };
    Ok((
        user,
        password,
        host,
        port,
        if path.is_empty() { "/" } else { path },
    ))
}
/// A colon after a slash or a Windows drive designates a local path, as does
/// input without a colon outside brackets. Unbalanced brackets name SSH.
fn local(input: &str) -> bool {
    if !input.contains(':') {
        return true;
    }
    let mut depth = 0_usize;
    for (index, byte) in input.bytes().enumerate() {
        match byte {
            b'[' => depth += 1,
            b']' if depth == 0 => return false,
            b']' => depth -= 1,
            b':' if depth == 0 => {
                return input[..index].contains(['/', '\\'])
                    || (index == 1 && input.as_bytes()[0].is_ascii_alphabetic());
            }
            b'/' | b'\\' if depth == 0 => return true,
            _ => {}
        }
    }
    depth == 0
}
/// libgit2's `git_net_url_parse_scp`, state for state. Nothing is decoded, and
/// an IPv6 host keeps its brackets.
fn scp(input: &str) -> io::Result<Parts<'_>> {
    #[derive(Clone, Copy)]
    enum At {
        None,
        User,
        HostStart,
        Host,
        HostEnd,
        Ipv6,
        Ipv6End,
        PortStart,
        Port,
        PortEnd,
        PathStart,
    }
    let (mut at, mut bracket, mut start) = (At::None, 0, 0);
    let (mut user, mut host, mut port) = (None, "", "");
    for (index, byte) in input.bytes().enumerate() {
        // Only at a char boundary: the start, or after an ASCII '[' or ':'.
        let rest = || &input[index..];
        at = match (at, byte) {
            (At::None, b'@' | b':') => return Err(invalid()),
            (At::None, b'[') if ipv6(rest()) => {
                start = index;
                At::Ipv6
            }
            (At::None, b'[') if bracket < 2 => {
                bracket += 1;
                At::None
            }
            (At::None, b'[') => return Err(invalid()),
            (At::None, _) => {
                start = index;
                if has_at(rest()) { At::User } else { At::Host }
            }
            (At::User, b'@') => {
                user = Some(&input[start..index]);
                At::HostStart
            }
            (At::HostStart, _) => {
                start = index;
                if byte == b'[' { At::Ipv6 } else { At::Host }
            }
            (At::Host | At::Ipv6End, b':') => {
                host = &input[start..index];
                if bracket > 0 {
                    At::PortStart
                } else {
                    At::PathStart
                }
            }
            (At::Host, b']') if bracket > 0 => {
                bracket -= 1;
                host = &input[start..index];
                At::HostEnd
            }
            (At::Port, b']') if bracket > 0 => {
                bracket -= 1;
                port = &input[start..index];
                At::PortEnd
            }
            (At::Host | At::Port, b']') => return Err(invalid()),
            (At::HostEnd | At::PortEnd, b':') => At::PathStart,
            (At::HostEnd | At::PortEnd | At::Ipv6End, _) => return Err(invalid()),
            (At::Ipv6, b']') => At::Ipv6End,
            (At::PortStart, _) => {
                start = index;
                At::Port
            }
            (At::PathStart, _) => return Ok((user, None, host, port, rest())),
            (unchanged, _) => unchanged,
        };
    }
    Err(invalid())
}
/// libgit2's `is_ipv6`: '[', hex digits and at least two colons, then ']'.
fn ipv6(text: &str) -> bool {
    let inner = &text[1..];
    inner
        .find(|c: char| !c.is_ascii_hexdigit() && c != ':')
        .is_some_and(|end| inner.as_bytes()[end] == b']' && inner[..end].matches(':').count() > 1)
}
/// libgit2's `has_at`: an '@' before the first ':'.
fn has_at(text: &str) -> bool {
    text.find(['@', ':'])
        .is_some_and(|at| text.as_bytes()[at] == b'@')
}
fn decode(input: &str) -> io::Result<String> {
    String::from_utf8(decode_bytes(input)).map_err(|_| invalid())
}
/// libgit2's `git_str_decode_percent`: a '%' that does not start two hex
/// digits stays as written. Decoding never grows, so the output is never
/// reallocated, and no copy of a password is left unwiped.
fn decode_bytes(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let hex = |index: usize| bytes.get(index).and_then(|c| (*c as char).to_digit(16));
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                output.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, _, _) => {
                output.push(byte);
                index += 1;
            }
        }
    }
    output
}
