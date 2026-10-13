//! Whether the configuration view holds a credential helper for a URL, read as `git credential` reads it.
//!
//! `git credential fill` ends the same way when no helper is configured and when one is and gave nothing: git
//! prompts, prompts are off, and it exits with failure. The view is what tells them apart. It is the configuration
//! Git itself is given (the same entries, conditional includes unevaluated), so a helper list that is empty here is
//! empty for Git.
use super::framing::Entry;

/// The parts of a URL, split as `credential_from_potentially_partial_url` splits one: a URL missing its scheme or its
/// host is still a pattern, one that names only the parts it has.
struct Parts<'a> {
    protocol: Option<&'a [u8]>,
    host: Option<&'a [u8]>,
    path: Option<&'a [u8]>,
    username: Option<&'a [u8]>,
}

impl<'a> Parts<'a> {
    fn of(url: &'a [u8]) -> Self {
        let (protocol, rest) = match url.windows(3).position(|window| window == b"://") {
            Some(at) => (
                Some(&url[..at]).filter(|protocol| !protocol.is_empty()),
                &url[at + 3..],
            ),
            None => (None, url),
        };
        let slash = rest
            .iter()
            .position(|byte| *byte == b'/')
            .unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(slash);
        let at = rest
            .iter()
            .position(|byte| *byte == b'@')
            .filter(|at| *at < slash);
        let (username, host) = match at {
            Some(at) => {
                // `user:password@host` names the user before the colon; a password is never matched on.
                let name = rest[..at]
                    .split(|byte| *byte == b':')
                    .next()
                    .unwrap_or(&rest[..at]);
                (Some(name), &authority[at + 1..])
            }
            None => (None, authority),
        };
        Parts {
            protocol,
            host: Some(host).filter(|host| !host.is_empty()),
            path: tail.strip_prefix(b"/").filter(|path| !path.is_empty()),
            username,
        }
    }

    /// Git's `credential_match`: every part the pattern names is the same, byte for byte, in the request.
    fn partially_matches(&self, request: &Parts<'_>) -> bool {
        let same =
            |want: Option<&[u8]>, have: Option<&[u8]>| want.is_none_or(|want| have == Some(want));
        same(self.protocol, request.protocol)
            && same(self.host, request.host)
            && same(self.path, request.path)
            && same(self.username, request.username)
    }
}

/// A URL as Git's `url_normalize` writes it, which is what `credential.<url>.*` sections are matched on.
struct Normal {
    scheme: Vec<u8>,
    user: Option<Vec<u8>>,
    /// Lowercase, without a trailing dot and without the port that its scheme implies. A port stays attached to the
    /// last label, as it does in Git.
    host: Vec<u8>,
    /// Starts with `/`, has no dot segments and no trailing `/` (the root is empty).
    path: Vec<u8>,
}

fn unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

/// A byte that URL normalization writes as `%XX` when it is not already one.
fn needs_encoding(byte: u8) -> bool {
    byte <= b' ' || byte >= 0x7f || b"\"<>\\^`{|}%".contains(&byte)
}

fn push_escaped(path: &mut Vec<u8>, byte: u8) {
    path.extend_from_slice(format!("%{byte:02X}").as_bytes());
}

fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

fn hex_pair(bytes: &[u8]) -> Option<u8> {
    Some(hex_value(*bytes.first()?)? << 4 | hex_value(*bytes.get(1)?)?)
}

/// A pattern's path in normal form: `%XX` of an unreserved byte written as the byte, any other `%XX` in capitals.
/// `None` where Git refuses the URL: a `%` without two hex digits.
fn escaped_pattern_path(path: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(path.len());
    let mut at = 0;
    while at < path.len() {
        let byte = path[at];
        if byte == b'%' {
            let decoded = hex_pair(&path[at + 1..])?;
            if unreserved(decoded) {
                out.push(decoded);
            } else {
                push_escaped(&mut out, decoded);
            }
            at += 3;
        } else {
            if needs_encoding(byte) {
                push_escaped(&mut out, byte);
            } else {
                out.push(byte);
            }
            at += 1;
        }
    }
    Some(out)
}

/// The path of a request, which `credential_from_url` has decoded before Git describes it again: every `%XX` is its
/// byte (so `%2F` is a separator), and the bytes that need it are written as `%XX` once more.
fn escaped_request_path(path: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(path.len());
    let mut at = 0;
    while at < path.len() {
        let byte = match (path[at], hex_pair(path.get(at + 1..).unwrap_or_default())) {
            (b'%', Some(decoded)) => {
                at += 2;
                decoded
            }
            (byte, _) => byte,
        };
        if needs_encoding(byte) {
            push_escaped(&mut out, byte);
        } else {
            out.push(byte);
        }
        at += 1;
    }
    out
}

/// `escaped` (a path without its leading `/`) with its dot segments removed and no trailing `/`; `None` where a `..`
/// climbs above the root, as Git refuses.
fn path_without_dots(escaped: &[u8]) -> Option<Vec<u8>> {
    let mut segments: Vec<&[u8]> = Vec::new();
    for segment in escaped.split(|byte| *byte == b'/') {
        match segment {
            b"." => {}
            b".." => {
                segments.pop()?;
            }
            _ => segments.push(segment),
        }
    }
    let mut path = Vec::new();
    for segment in segments {
        path.push(b'/');
        path.extend_from_slice(segment);
    }
    // A trailing `/` is not part of what a prefix names.
    if path.last() == Some(&b'/') {
        path.pop();
    }
    if path == b"/" {
        path.clear();
    }
    Some(path)
}

/// `host` with its port, written as Git writes them: lowercase, a trailing dot dropped, and no port that the scheme
/// implies (`:80` for http, `:443` for https). `None` where Git refuses the host: an empty label or a byte a host
/// cannot hold (a `*` only where `globs`).
fn normal_host(scheme: &[u8], host: &[u8], globs: bool) -> Option<Vec<u8>> {
    let (name, port) = if host.first() == Some(&b'[') {
        let close = host.iter().position(|byte| *byte == b']')? + 1;
        let literal = &host[1..close - 1];
        if literal.is_empty()
            || !literal
                .iter()
                .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b':' | b'.'))
        {
            return None;
        }
        (&host[..close], host[close..].strip_prefix(b":"))
    } else {
        match host.iter().rposition(|byte| *byte == b':') {
            Some(colon) => (&host[..colon], Some(&host[colon + 1..])),
            None => (host, None),
        }
    };
    let name = if name.first() == Some(&b'[') {
        name.to_ascii_lowercase()
    } else {
        let name = name.strip_suffix(b".").unwrap_or(name);
        let label_byte =
            |byte: &u8| byte.is_ascii_alphanumeric() || *byte == b'-' || (globs && *byte == b'*');
        if name.is_empty()
            || name
                .split(|byte| *byte == b'.')
                .any(|label| label.is_empty() || !label.iter().all(label_byte))
        {
            return None;
        }
        name.to_ascii_lowercase()
    };
    let mut normal = name;
    let port = port.filter(|port| !port.is_empty());
    if let Some(port) = port {
        let number = std::str::from_utf8(port)
            .ok()
            .filter(|port| port.bytes().all(|byte| byte.is_ascii_digit()));
        let number = number.and_then(|number| number.parse::<u32>().ok());
        let implied = match (scheme, number) {
            (b"http", Some(80)) | (b"https", Some(443)) => true,
            _ => false,
        };
        if !implied {
            normal.push(b':');
            match number {
                Some(number) => normal.extend_from_slice(number.to_string().as_bytes()),
                None => normal.extend_from_slice(&port.to_ascii_lowercase()),
            }
        }
    }
    Some(normal)
}

impl Normal {
    /// The URL of `parts` in normal form, `None` where Git's `url_normalize` refuses it. `pattern` is a
    /// `credential.<url>` section name (host labels may be `*`, and its `?` and `#` are refused) and not a request.
    fn of(parts: &Parts<'_>, pattern: bool) -> Option<Self> {
        let scheme = parts.protocol?;
        if !scheme.first()?.is_ascii_alphabetic()
            || !scheme
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        {
            return None;
        }
        let scheme = scheme.to_ascii_lowercase();
        let host = normal_host(&scheme, parts.host?, pattern)?;
        let raw = parts.path.unwrap_or_default();
        if pattern && raw.iter().any(|byte| matches!(byte, b'?' | b'#')) {
            return None;
        }
        let escaped = if pattern {
            escaped_pattern_path(raw)?
        } else {
            escaped_request_path(raw)
        };
        Some(Self {
            scheme,
            user: parts.username.map(<[u8]>::to_vec),
            host,
            path: path_without_dots(&escaped)?,
        })
    }

    /// Git's `match_urls`, for a pattern against a request: the same scheme; the same user if the pattern has one;
    /// the same host, label by label, where a pattern label `*` matches any one label; and a path that the request's
    /// path continues at a `/` or ends with.
    fn is_matched_by(&self, pattern: &Self) -> bool {
        let labels = |host: &[u8]| {
            host.split(|byte| *byte == b'.')
                .map(<[u8]>::to_vec)
                .collect::<Vec<_>>()
        };
        let (wanted, have) = (labels(&pattern.host), labels(&self.host));
        self.scheme == pattern.scheme
            && pattern
                .user
                .as_ref()
                .is_none_or(|user| self.user.as_ref() == Some(user))
            && wanted.len() == have.len()
            && wanted
                .iter()
                .zip(&have)
                .all(|(want, have)| want == b"*" || want == have)
            && self.path.starts_with(&pattern.path)
            && matches!(self.path.get(pattern.path.len()), None | Some(b'/'))
    }
}

/// The request a helper is looked up for: the URL as `git credential` parses it, and that URL normalized.
struct Request<'a> {
    parts: Parts<'a>,
    normal: Option<Normal>,
}

impl<'a> Request<'a> {
    /// `None` where Git ignores the URL (it has no scheme or no host).
    fn of(url: &'a [u8]) -> Option<Self> {
        let parts = Parts::of(url);
        parts.protocol?;
        parts.host?;
        let normal = Normal::of(&parts, false);
        Some(Self { parts, normal })
    }

    /// Whether the section `credential.<pattern>` applies to this request: Git's `urlmatch_config_entry`. A pattern
    /// that normalizes as a URL is matched as one; any other is a partial URL (`example.test`, `me@example.test`,
    /// `/path`, or the empty pattern), matched part by part, byte for byte (`match_partial_url`). A request Git
    /// cannot normalize is matched by no section, and the unscoped `credential.helper` still applies to it.
    fn is_matched_by(&self, pattern: &[u8]) -> bool {
        let Some(request) = &self.normal else {
            return false;
        };
        let parts = Parts::of(pattern);
        match Normal::of(&parts, true) {
            Some(normal) => request.is_matched_by(&normal),
            None => parts.partially_matches(&self.parts),
        }
    }
}

/// Whether Git, given `entries` and asked for `url`, holds at least one credential helper. Entries apply in their
/// order: `credential.helper` and the `credential.<pattern>.helper` whose pattern matches the URL add a helper,
/// and an empty value clears the list. A key with no value adds nothing.
pub(super) fn in_effect(entries: &[Entry], url: &[u8]) -> bool {
    let Some(request) = Request::of(url) else {
        return false;
    };
    let mut held = false;
    for entry in entries {
        let Some(rest) = entry.name.0.strip_prefix(b"credential.") else {
            continue;
        };
        let applies = rest == b"helper"
            || rest
                .strip_suffix(b".helper")
                .is_some_and(|pattern| request.is_matched_by(pattern));
        if !applies {
            continue;
        }
        if let Some(value) = &entry.value {
            held = !value.0.is_empty();
        }
    }
    held
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
