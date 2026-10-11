//! The command an SSH client sends to run a Git service, read back: exactly what [`GitService::command`] writes, and
//! nothing else, so the server never hands a client's string to a shell.
//!
//! [`GitService::command`]: crate::git::endpoint::ssh_channel::GitService::command

/// A Git service and the repository path the client named for it.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct Request {
    /// The `git` subcommand: `upload-pack` or `receive-pack`.
    pub(super) service: &'static str,
    pub(super) path: String,
}

/// `git-upload-pack 'path'` or `git-receive-pack 'path'`, the path quoted for a POSIX shell as the client quotes it:
/// single quotes around the text, an embedded `'` as `'\''` and `!` as `'\!'`. A command of any other shape, an
/// empty path or a path with a NUL is no request.
pub(super) fn parse(command: &[u8]) -> Option<Request> {
    let command = std::str::from_utf8(command).ok()?;
    let (service, rest) = if let Some(rest) = command.strip_prefix("git-upload-pack ") {
        ("upload-pack", rest)
    } else if let Some(rest) = command.strip_prefix("git-receive-pack ") {
        ("receive-pack", rest)
    } else {
        return None;
    };
    let path = unquote(rest)?;
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    Some(Request { service, path })
}

/// The one shell word `word` spells with single-quoted runs and backslash escapes, or `None` if it spells anything
/// else (unquoted text, an unclosed quote, a trailing backslash, a second word).
fn unquote(word: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => loop {
                match chars.next()? {
                    '\'' => break,
                    inside => out.push(inside),
                }
            },
            '\\' => out.push(chars.next()?),
            _ => return None,
        }
    }
    Some(out)
}
