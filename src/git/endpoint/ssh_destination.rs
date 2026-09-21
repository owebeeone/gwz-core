//! Side-effect-free SSH admission. Paths are operands, never shell commands.
use gwz_transport::pool::Key;
use std::{io, net::Ipv6Addr};

#[derive(Debug)]
pub(crate) struct Destination {
    pub(crate) key: Key,
    pub(crate) path: String,
}
impl Destination {
    /// None leaves a non-SSH URL on its existing native/local route. Once an SSH
    /// spelling is recognized, malformed input is an error, never a fallback.
    pub(crate) fn parse(input: &str) -> io::Result<Option<Self>> {
        let (authority, path, escaped) = if let Some((scheme, rest)) = input.split_once("://") {
            if !["ssh", "git+ssh", "ssh+git"]
                .iter()
                .any(|s| scheme.eq_ignore_ascii_case(s))
            {
                return Ok(None);
            }
            if rest.contains(['?', '#']) {
                return Err(invalid());
            }
            let slash = rest.find('/').ok_or_else(invalid)?;
            (&rest[..slash], &rest[slash..], true)
        } else {
            // A colon after a slash or a Windows drive designates a local path.
            let Some(colon) = scp_colon(input)? else {
                return Ok(None);
            };
            let authority = &input[..colon];
            if authority.contains(['/', '\\'])
                || (colon == 1 && input.as_bytes()[0].is_ascii_alphabetic())
            {
                return Ok(None);
            }
            (authority, &input[colon + 1..], false)
        };
        if input.len() > 18_000 || input.chars().any(char::is_control) {
            return Err(invalid());
        }
        // Native SCP accepts brackets around the entire authority as well as
        // around an IPv6 address. Strip only an outer, non-IPv6 grouping.
        let authority = if escaped {
            authority
        } else {
            scp_group(authority)
        };
        let (user, host_port) = match authority.split_once('@') {
            Some((user, host))
                if !user.is_empty() && !user.contains(':') && !host.contains('@') =>
            {
                (user, host)
            }
            Some(_) => return Err(invalid()),
            None => ("git", authority),
        };
        let user = if escaped {
            decode(user)?
        } else {
            user.to_owned()
        };
        if user.is_empty() || user.len() > 128 || user.chars().any(char::is_control) {
            return Err(invalid());
        }
        let (host, port) = host_port_parts(host_port, escaped)?;
        let mut path = if escaped {
            decode(path)?
        } else {
            path.to_owned()
        };
        // Match native libgit2: /~user is passed to Git as ~user, where the
        // repository-opening routine handles home-relative operands.
        if escaped && path.starts_with("/~") {
            path.remove(0);
        }
        if path.is_empty() || path.len() > 16_384 || path.chars().any(char::is_control) {
            return Err(invalid());
        }
        Ok(Some(Self {
            key: Key::ssh(user, host, port),
            path,
        }))
    }
}
fn invalid() -> io::Error {
    // Do not echo a possibly credential-bearing URL in an error or observation.
    io::Error::new(io::ErrorKind::InvalidInput, "invalid SSH destination")
}
fn scp_colon(input: &str) -> io::Result<Option<usize>> {
    if !input.contains(':') {
        return Ok(None);
    }
    let mut depth = 0_usize;
    for (index, byte) in input.bytes().enumerate() {
        match byte {
            b'[' => depth += 1,
            b']' => depth = depth.checked_sub(1).ok_or_else(invalid)?,
            b':' if depth == 0 => return Ok(Some(index)),
            b'/' | b'\\' if depth == 0 => return Ok(None),
            _ => {}
        }
    }
    if depth != 0 {
        return Err(invalid());
    }
    Ok(None)
}
fn scp_group(input: &str) -> &str {
    match input.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        Some(inner) if inner.parse::<Ipv6Addr>().is_err() => inner,
        _ => input,
    }
}
fn host_port_parts(input: &str, url: bool) -> io::Result<(String, u16)> {
    let input = if url { input } else { scp_group(input) };
    let (host, port) = if let Some(rest) = input.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']').ok_or_else(invalid)?;
        host.parse::<Ipv6Addr>().map_err(|_| invalid())?;
        let port = if suffix.is_empty() {
            None
        } else {
            Some(suffix.strip_prefix(':').ok_or_else(invalid)?)
        };
        (host, port)
    } else if let Some((host, port)) = input.split_once(':') {
        (host, Some(port))
    } else {
        (input, None)
    };
    if host.is_empty()
        || host.len() > 255
        || !host.is_ascii()
        || host
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "@/?#\\%[]".contains(c))
        || (!input.starts_with('[') && host.contains(':'))
    {
        return Err(invalid());
    }
    let port = match port {
        None => 22,
        Some(value) if !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()) => {
            value.parse::<u16>().map_err(|_| invalid())?
        }
        Some(_) => return Err(invalid()),
    };
    if port == 0 {
        return Err(invalid());
    }
    Ok((host.to_ascii_lowercase(), port))
}
fn decode(input: &str) -> io::Result<String> {
    let mut output = Vec::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes
                .next()
                .and_then(|c| (c as char).to_digit(16))
                .ok_or_else(invalid)?;
            let low = bytes
                .next()
                .and_then(|c| (c as char).to_digit(16))
                .ok_or_else(invalid)?;
            output.push((high * 16 + low) as u8);
        } else {
            output.push(byte);
        }
    }
    String::from_utf8(output).map_err(|_| invalid())
}
