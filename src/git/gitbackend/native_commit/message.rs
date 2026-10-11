//! Commit and tag message cleanup, as `git commit -m` and `git tag -m` do it.

/// The `--cleanup` modes native reproduces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cleanup {
    /// Strip comment lines, then whitespace.
    Strip,
    /// Whitespace only (the default for `git commit -m`; `scissors` acts the
    /// same way when no editor is used, as Git 2.52 does).
    Whitespace,
    /// Keep the message as given.
    Verbatim,
}

impl Cleanup {
    /// Parse a `commit.cleanup` value; `default` is whitespace for `-m`.
    pub(super) fn from_commit_config(value: Option<&str>) -> Option<Self> {
        match value.map(str::to_ascii_lowercase).as_deref() {
            None | Some("default") | Some("whitespace") | Some("scissors") => {
                Some(Self::Whitespace)
            }
            Some("strip") => Some(Self::Strip),
            Some("verbatim") => Some(Self::Verbatim),
            Some(_) => None,
        }
    }

    /// Whether the comment character decides the result.
    pub(super) fn uses_comment_character(self) -> bool {
        matches!(self, Self::Strip)
    }
}

/// Git's comment character when `core.commentChar` and `core.commentString`
/// are unset.
const COMMENT: char = '#';

/// The characters Git's `isspace` accepts.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// `strbuf_stripspace`: trim each line's end, drop (with `strip`) comment
/// lines, collapse runs of blank lines, drop leading and trailing blank lines
/// and end with a newline.
fn strip_space(text: &str, drop_comments: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_blank = false;
    for line in text.split_inclusive('\n') {
        if drop_comments && line.starts_with(COMMENT) {
            continue;
        }
        let trimmed = line.trim_end_matches(is_space);
        if trimmed.is_empty() {
            if !out.is_empty() {
                pending_blank = true;
            }
            continue;
        }
        if pending_blank {
            out.push('\n');
            pending_blank = false;
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

/// The message `git commit -m <message>` stores, before any hook.
pub(super) fn clean_commit_message(message: &str, mode: Cleanup) -> String {
    let mut text = message.to_owned();
    if !text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    match mode {
        Cleanup::Verbatim => text,
        Cleanup::Whitespace => strip_space(&text, false),
        Cleanup::Strip => strip_space(&text, true),
    }
}

/// The message `git tag -m <message>` stores: comment lines and whitespace
/// stripped.
pub(super) fn clean_tag_message(message: &str) -> String {
    let mut text = message.to_owned();
    if !text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    strip_space(&text, true)
}

/// The reflog line Git writes for a commit: `kind`, a colon, the message's
/// first line, with every run of whitespace squashed to one space and the
/// ends trimmed (`copy_reflog_msg`).
pub(super) fn reflog_message(kind: &str, message: &str) -> String {
    let first_line = message.split('\n').next().unwrap_or("");
    let mut out = String::new();
    let mut was_space = true;
    for c in format!("{kind}: {first_line}").chars() {
        if is_space(c) {
            if !was_space {
                out.push(' ');
            }
            was_space = true;
        } else {
            out.push(c);
            was_space = false;
        }
    }
    out.truncate(out.trim_end().len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_cleanup_matches_the_characterized_form() {
        let message = "\n  body  \n# comment\nnext  \n\n";
        assert_eq!(
            clean_commit_message(message, Cleanup::Whitespace),
            "  body\n# comment\nnext\n"
        );
        assert_eq!(
            clean_commit_message(message, Cleanup::Strip),
            "  body\nnext\n"
        );
        assert_eq!(clean_commit_message(message, Cleanup::Verbatim), message);
    }

    #[test]
    fn blank_runs_collapse_and_edges_go() {
        assert_eq!(strip_space("\n\na\n\n\n\nb  \n\n\n", false), "a\n\nb\n");
        assert_eq!(strip_space("a\n\n# c\n\nb", true), "a\n\nb\n");
        assert_eq!(strip_space("   \n  ", false), "");
    }

    #[test]
    fn cleanup_modes_parse() {
        assert_eq!(Cleanup::from_commit_config(None), Some(Cleanup::Whitespace));
        assert_eq!(
            Cleanup::from_commit_config(Some("Strip")),
            Some(Cleanup::Strip)
        );
        assert_eq!(
            Cleanup::from_commit_config(Some("scissors")),
            Some(Cleanup::Whitespace)
        );
        assert_eq!(Cleanup::from_commit_config(Some("nonsense")), None);
    }

    #[test]
    fn reflog_line_is_the_squashed_first_line() {
        assert_eq!(reflog_message("commit", "  body\nnext\n"), "commit: body");
        assert_eq!(
            reflog_message("commit", "multi\nline\n\nbody\n"),
            "commit: multi"
        );
        assert_eq!(
            reflog_message("commit", "tab\there   spaces\r\nx"),
            "commit: tab here spaces"
        );
        assert_eq!(
            reflog_message("commit (initial)", "\n\nlater"),
            "commit (initial):"
        );
    }
}
