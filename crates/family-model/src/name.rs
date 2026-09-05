//! Clone name policy (design §2, §3, §8.4).
//!
//! A name is the family's whole namespace: it is the index key, the
//! `--remote` token every exchange verb resolves, and the `from` selector
//! that also accepts a path. So a name that could be read as a path, a Git
//! ref or an empty argument refuses at the boundary rather than becoming an
//! index key nothing can address.
//!
//! **Names are compared exactly.** `Root`, `Origin` and `head` are ordinary
//! legal names, distinct from the reserved `root`, `origin` and `HEAD`, and
//! `A` and `a` are two members. The model never case-folds: it would have to
//! guess a locale, and the family's own namespace is not a filesystem. Two
//! names whose *paths* differ only in case collide on a case-insensitive
//! filesystem — that is the store's canonicalisation, not this rule.

use std::fmt;

/// Names a clone may never take (design §2).
pub const RESERVED_NAMES: [&str; 4] = ["root", "origin", "HEAD", "FETCH_HEAD"];
/// Directory shorthands: `from` accepts "family name or path" (design §7),
/// so a name that is a path shorthand is refused with the reserved names.
pub const RESERVED_DIRECTORY_NAMES: [&str; 2] = [".", ".."];

/// A validated clone name: non-empty, unpadded, free of control characters
/// and `/`/`:`, and not reserved.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemberName(String);

impl MemberName {
    pub fn parse(name: &str) -> Result<Self, NameError> {
        if name.is_empty() {
            return Err(NameError::Empty);
        }
        if name.chars().all(char::is_whitespace) {
            return Err(NameError::Blank);
        }
        if name.starts_with(char::is_whitespace) || name.ends_with(char::is_whitespace) {
            return Err(NameError::SurroundingWhitespace);
        }
        if let Some(control) = name.chars().find(|c| c.is_control()) {
            return Err(NameError::Control(control));
        }
        if let Some(separator) = name.chars().find(|c| matches!(c, '/' | ':')) {
            return Err(NameError::Separator(separator));
        }
        if RESERVED_NAMES.contains(&name) || RESERVED_DIRECTORY_NAMES.contains(&name) {
            return Err(NameError::Reserved(name.to_owned()));
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MemberName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// [`MemberName::parse`] under the name the policy uses, beside
/// [`validate_member_path`](crate::validate_member_path).
pub fn validate(name: &str) -> Result<MemberName, NameError> {
    MemberName::parse(name)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    /// No name at all: create without a name refuses (design §2).
    Empty,
    /// Only whitespace.
    Blank,
    /// Leading or trailing whitespace, which no listing could show.
    SurroundingWhitespace,
    /// A control character; the request is malformed.
    Control(char),
    /// `/` or `:`: a path or Git-refspec separator (design §2).
    Separator(char),
    /// A reserved name, or a directory shorthand (design §2, §7 `from`).
    Reserved(String),
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a clone name must not be empty"),
            Self::Blank => f.write_str("a clone name must not be only whitespace"),
            Self::SurroundingWhitespace => {
                f.write_str("a clone name must not start or end with whitespace")
            }
            Self::Control(c) => write!(f, "a clone name must not contain {}", c.escape_debug()),
            Self::Separator(c) => write!(f, "a clone name must not contain `{c}`"),
            Self::Reserved(name) => write!(f, "`{name}` is a reserved name"),
        }
    }
}

impl std::error::Error for NameError {}

/// What `gwz local dispose <token>` is addressing.
///
/// The root is a legal, meaningful token here and an illegal clone name, so
/// a dispose refusal says why rather than reporting a reserved word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposeTarget {
    /// `root`: never disposed. The original tree is never deleted; `gwz
    /// local disband` drops the family and retains every tree (design §3,
    /// §8.4).
    Root,
    /// A clone name, valid in shape. Whether the family holds a row for it
    /// is a separate lookup.
    Member(MemberName),
    /// Not a name at all.
    Invalid(NameError),
}

/// Classify a dispose token. Pure shape only; no family is consulted.
pub fn classify_dispose_target(token: &str) -> DisposeTarget {
    if token == crate::ROOT_NAME {
        return DisposeTarget::Root;
    }
    match MemberName::parse(token) {
        Ok(name) => DisposeTarget::Member(name),
        Err(error) => DisposeTarget::Invalid(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reserved_name_and_shape_refuses_with_a_typed_reason() {
        assert_eq!(MemberName::parse("").unwrap_err(), NameError::Empty);
        assert_eq!(MemberName::parse("   ").unwrap_err(), NameError::Blank);
        for padded in [" A", "A ", "A\t", "\u{a0}A"] {
            assert_eq!(
                MemberName::parse(padded).unwrap_err(),
                NameError::SurroundingWhitespace,
                "{padded:?}"
            );
        }
        assert_eq!(
            MemberName::parse("a\nb").unwrap_err(),
            NameError::Control('\n')
        );
        assert_eq!(
            MemberName::parse("a\u{7f}b").unwrap_err(),
            NameError::Control('\u{7f}')
        );
        assert_eq!(
            MemberName::parse("a/b").unwrap_err(),
            NameError::Separator('/')
        );
        assert_eq!(
            MemberName::parse("a:b").unwrap_err(),
            NameError::Separator(':')
        );
        for reserved in RESERVED_NAMES.iter().chain(&RESERVED_DIRECTORY_NAMES) {
            assert_eq!(
                MemberName::parse(reserved).unwrap_err(),
                NameError::Reserved((*reserved).to_owned()),
                "{reserved}"
            );
        }
        assert_eq!(
            NameError::Reserved("root".to_owned()).to_string(),
            "`root` is a reserved name"
        );
        assert_eq!(
            NameError::Control('\n').to_string(),
            "a clone name must not contain \\n"
        );
    }

    #[test]
    fn ordinary_names_pass_and_comparison_is_exact() {
        for ordinary in [
            "A", "lane-17", "hub", "Root", "Origin", "head", "a.b", "ws A",
        ] {
            assert_eq!(
                validate(ordinary).unwrap().as_str(),
                ordinary,
                "{ordinary} is an ordinary clone name"
            );
        }
        assert_ne!(
            MemberName::parse("A").unwrap(),
            MemberName::parse("a").unwrap(),
            "names are compared exactly; the model never case-folds"
        );
    }

    #[test]
    fn dispose_addresses_the_root_by_name_but_never_disposes_it() {
        assert_eq!(classify_dispose_target("root"), DisposeTarget::Root);
        assert_eq!(
            classify_dispose_target("C"),
            DisposeTarget::Member(MemberName::parse("C").unwrap())
        );
        assert_eq!(
            classify_dispose_target("Root"),
            DisposeTarget::Member(MemberName::parse("Root").unwrap()),
            "only the exact reserved spelling is the root"
        );
        assert_eq!(
            classify_dispose_target(""),
            DisposeTarget::Invalid(NameError::Empty)
        );
        assert_eq!(
            classify_dispose_target("origin"),
            DisposeTarget::Invalid(NameError::Reserved("origin".to_owned()))
        );
    }
}
