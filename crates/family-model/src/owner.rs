//! The caller's owner token for a row (GwzLaneCleanFixes R20).
//!
//! A lane made by a tool records *who asked for it* so the same tool can
//! decide, later and without gwz's help, whether the lane it is looking at
//! is one of its own. The token is opaque: gwz validates its shape, stores
//! it on the row in the same index write that reserves the row, reports it
//! through `gwz local list`, and never interprets, compares, parses or acts
//! on the value. A row written without one has none, for ever; nothing
//! changes a row's token after creation.
//!
//! The shape is deliberately narrow — at most
//! [`MAX_OWNER_TOKEN_BYTES`] bytes drawn from `[A-Za-z0-9._:-]` — so the
//! value survives YAML, a shell, a log line and a JSON field unquoted and
//! unescaped, and so a stray newline can never forge a row in a listing
//! consumers read by line.

use std::fmt;

/// Largest owner token the model admits, in bytes. The alphabet is ASCII,
/// so bytes and characters agree.
pub const MAX_OWNER_TOKEN_BYTES: usize = 128;

/// An opaque caller identity recorded on a member row. Never interpreted.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnerToken(String);

impl OwnerToken {
    /// Validate and take an owner token. The only decisions here are
    /// emptiness, length and alphabet; the meaning is the caller's.
    pub fn parse(value: impl Into<String>) -> Result<Self, OwnerError> {
        let value = value.into();
        if value.is_empty() {
            return Err(OwnerError::Empty);
        }
        if value.len() > MAX_OWNER_TOKEN_BYTES {
            return Err(OwnerError::TooLong { bytes: value.len() });
        }
        if let Some(character) = value.chars().find(|character| !is_admitted(*character)) {
            return Err(OwnerError::Character { character });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const fn is_admitted(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
}

impl fmt::Display for OwnerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why an owner token was refused. Shape only: an admitted token still
/// means nothing to gwz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerError {
    Empty,
    TooLong { bytes: usize },
    Character { character: char },
}

impl fmt::Display for OwnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("an owner token must not be empty"),
            Self::TooLong { bytes } => write!(
                f,
                "an owner token is at most {MAX_OWNER_TOKEN_BYTES} bytes; this one is {bytes}"
            ),
            Self::Character { character } => write!(
                f,
                "an owner token accepts only `[A-Za-z0-9._:-]`; `{character}` is not one of them"
            ),
        }
    }
}

impl std::error::Error for OwnerError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_admitted_alphabet_round_trips() {
        let token = OwnerToken::parse("claude-code:session_01.A-Z").unwrap();
        assert_eq!(token.as_str(), "claude-code:session_01.A-Z");
        assert_eq!(token.to_string(), "claude-code:session_01.A-Z");
    }

    #[test]
    fn empty_too_long_and_foreign_characters_refuse() {
        assert_eq!(OwnerToken::parse(""), Err(OwnerError::Empty));
        let long = "a".repeat(MAX_OWNER_TOKEN_BYTES + 1);
        assert_eq!(
            OwnerToken::parse(long),
            Err(OwnerError::TooLong {
                bytes: MAX_OWNER_TOKEN_BYTES + 1
            })
        );
        assert!(OwnerToken::parse("a".repeat(MAX_OWNER_TOKEN_BYTES)).is_ok());
        for value in ["a b", "a/b", "a\nb", "café", "a\"b", "a#b"] {
            assert!(
                matches!(OwnerToken::parse(value), Err(OwnerError::Character { .. })),
                "`{value}` must refuse"
            );
        }
    }

    #[test]
    fn the_refusals_say_what_the_shape_is() {
        let long = OwnerToken::parse("a".repeat(200)).unwrap_err().to_string();
        assert!(long.contains("128"), "{long}");
        let character = OwnerToken::parse("a b").unwrap_err().to_string();
        assert!(character.contains("[A-Za-z0-9._:-]"), "{character}");
    }
}
