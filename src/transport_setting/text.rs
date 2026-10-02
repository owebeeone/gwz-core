//! The setting's words (design §10): the refusals in each driver's words,
//! and the pieces that the drivers' notes share, `<where>`, `<remove>`,
//! `<entry>` and `<scope>`, with E1's rules for values, paths and
//! identifiers.

use std::path::Path;

use super::{Driver, IgnoredValue, KEY, Location, Refusal, Scope};

impl Refusal {
    /// The refusal in `driver`'s words, which the driver prints after its own
    /// prefix: gwz-cli after `gwz: `, exiting 2, and gwz-py after
    /// `native bridge call failed for <method>: `, as `InvalidRequest`. gwz-py
    /// has no flag form in 1.1.0, so its words leave out `--transport`
    /// (design §10).
    pub fn message(&self, driver: Driver) -> String {
        let (flag, decides) = match driver {
            Driver::Cli => (
                "; --transport decides without it",
                "--transport or GWZ_TRANSPORT decides without it",
            ),
            Driver::Python => ("", "GWZ_TRANSPORT decides without it"),
        };
        match self {
            Self::Environment { value } => {
                format!("GWZ_TRANSPORT must be gwz or native, not {value:?}{flag}")
            }
            Self::EnvironmentNotUtf8 => format!("GWZ_TRANSPORT is not valid UTF-8{flag}"),
            Self::Configuration { location, value } => {
                let problem = match value {
                    Some(value) => format!("not {value:?}"),
                    None => "and has no value".to_owned(),
                };
                format!(
                    "{KEY} in {} must be gwz or native, {problem}; {}, or fix it; {decides}",
                    location.where_text(),
                    location.remove_text()
                )
            }
            Self::Unparsable { file, cause } => format!(
                "could not parse {}, or a file it includes, for {KEY}: {}; fix the file and line it \
                 names; {decides}",
                path_text(file),
                escaped(cause)
            ),
        }
    }
}

impl Location {
    /// `<where>`: the file, or `a file included by` the file for a value it
    /// reaches through `include.path`, the path's control characters escaped
    /// (design §10, E1).
    pub fn where_text(&self) -> String {
        let file = path_text(&self.file);
        if self.included {
            format!("a file included by {file}")
        } else {
            file
        }
    }

    /// `<remove>`: how to remove the value (design §10, E2). For a value the
    /// file holds, a `git config --file … --unset-all` command, which removes
    /// every line of the key; for an included value, which git never writes
    /// into, the command that locates it, run in `/` so that git, like gwz,
    /// applies no `includeIf`. A file no command can name gets words instead.
    pub fn remove_text(&self) -> String {
        match (shell_quoted(&self.file), self.included) {
            (Some(file), false) => {
                format!("remove it with git config --file {file} --unset-all {KEY}")
            }
            (Some(file), true) => format!(
                "git -C / config --file {file} --includes --show-origin --get-all {KEY} shows \
                 which file holds it; remove it there"
            ),
            (None, false) => "remove it from that file".to_owned(),
            (None, true) => "remove it from the file that holds it".to_owned(),
        }
    }
}

impl Scope {
    /// `<scope>`: `root`, or `member` and the member's ID, its control
    /// characters escaped (design §10, E1).
    pub fn text(&self) -> String {
        match self {
            Self::Root => "root".to_owned(),
            Self::Member(id) => format!("member {}", escaped(id)),
        }
    }
}

impl IgnoredValue {
    /// `<entry>`: `gwz.transport = "<value>"`, the value as written, quoted
    /// and escaped as a value is, or `gwz.transport` for a key with no value
    /// (design §10).
    pub fn entry_text(&self) -> String {
        match &self.value {
            Some(value) => format!("{KEY} = {value:?}"),
            None => KEY.to_owned(),
        }
    }
}

/// A path as the notes, the `--verbose` line and the messages show it: its
/// control characters escaped, so one entry is one line, and bytes that are
/// not UTF-8 shown as U+FFFD (design §10, E1). Commands quote a path for the
/// shell instead (`Location::remove_text`).
pub fn path_text(path: &Path) -> String {
    escaped(&path.to_string_lossy())
}

/// `text` with each control character escaped as a value's quoting escapes
/// it, `\n` or `\u{1b}`, and nothing else changed (E1).
fn escaped(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() {
            shown.extend(character.escape_debug());
        } else {
            shown.push(character);
        }
    }
    shown
}

/// `path` quoted for a POSIX shell, between `'`s with an embedded `'`
/// written `'\''`, or `None` for a path that no printed command can name: one
/// that holds a control character (E1), one that is not UTF-8, which the
/// command's text cannot hold, and one that is not absolute, which the
/// command, run in `/`, would misname.
fn shell_quoted(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    if !path.is_absolute() || text.contains(char::is_control) {
        return None;
    }
    Some(format!("'{}'", text.replace('\'', r"'\''")))
}
