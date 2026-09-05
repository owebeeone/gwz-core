//! Root-relative member path policy (design §2, §3, §3.1).
//!
//! A member path is the **root-relative** location of one clone, as it is
//! recorded in the index. It is a value, not a host path: this module never
//! touches a filesystem, never resolves a symlink, and never learns the
//! root's own directory names.
//!
//! Three rules, all decided from the string alone:
//!
//! 1. It is **normalised** — no `.`, no empty and no interior `..`
//!    components, no trailing separator. The recorded form is therefore the
//!    only form, so the store, the orphan guard and this model all key on
//!    the same bytes (`root.join(row.path)` is one path, not a family of
//!    spellings).
//! 2. It **escapes the root** — at least one leading `..` and at least one
//!    ordinary component after the leading run. A path inside the root, the
//!    root itself, or an ancestor of the root refuses (design §2, "nested
//!    dest refuses"; §3, "the index lives only at `root`").
//! 3. Two member paths **do not overlap** — neither is the other, and
//!    neither is inside the other ([`relate`]).
//!
//! **What a pure comparison cannot decide.** Equivalence that depends on
//! the root's own directory names is invisible here: with the root at
//! `/a/b/root`, both `../root` and `../../b/root` name the root itself, and
//! `../ws-A` and `../../b/ws-A` name one directory. Case-insensitive and
//! symlinked filesystems widen the same gap. Those are the store's to close
//! by comparing canonical absolute paths (design §3.1, "use canonical
//! paths, no-follow checks where available, pointer equality"); this module
//! removes only the spelling differences a string can see.

use std::fmt;

/// A normalised, root-relative, root-escaping member path.
///
/// Construct with [`normalize`] (accepts any spelling) or [`validate`]
/// (additionally requires the input to be the normalised spelling).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemberPath(String);

impl MemberPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The path's components, in order. Every component is either `..` (in
    /// the leading run) or an ordinary directory name.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for MemberPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a recorded path is not a usable member path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    /// Empty, or only separators.
    Empty,
    /// Absolute, or drive/UNC qualified: a host path, which never enters
    /// the model.
    Absolute { path: String },
    /// Contains a control character; the metadata is malformed.
    ControlCharacter { path: String },
    /// Normalises to the root itself (`.`, `ws/..`).
    RootItself { path: String },
    /// Normalises to a path inside the root (`sub/dir`). The index lives
    /// only at the root, and a nested destination refuses.
    InsideRoot { path: String, normalised: String },
    /// Normalises to an ancestor of the root (`..`, `../..`), which
    /// contains the whole root workspace.
    ContainsRoot { path: String, normalised: String },
    /// A member path with a valid meaning, spelled unnormalised. The
    /// normalised spelling is the one the index records.
    NotNormalised { path: String, normalised: String },
}

impl PathError {
    /// The normalised spelling, when the path had a valid one.
    pub fn normalised(&self) -> Option<&str> {
        match self {
            Self::InsideRoot { normalised, .. }
            | Self::ContainsRoot { normalised, .. }
            | Self::NotNormalised { normalised, .. } => Some(normalised),
            Self::Empty
            | Self::Absolute { .. }
            | Self::ControlCharacter { .. }
            | Self::RootItself { .. } => None,
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a member path must not be empty"),
            Self::Absolute { path } => {
                write!(
                    f,
                    "member path `{path}` must be root-relative, not absolute"
                )
            }
            Self::ControlCharacter { path } => {
                write!(f, "member path `{path}` contains a control character")
            }
            Self::RootItself { path } => {
                write!(f, "member path `{path}` is the root itself")
            }
            Self::InsideRoot { path, normalised } => write!(
                f,
                "member path `{path}` (`{normalised}`) is inside the root"
            ),
            Self::ContainsRoot { path, normalised } => {
                write!(f, "member path `{path}` (`{normalised}`) contains the root")
            }
            Self::NotNormalised { path, normalised } => {
                write!(f, "member path `{path}` is not normalised (`{normalised}`)")
            }
        }
    }
}

impl std::error::Error for PathError {}

/// How two member paths sit relative to each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRelation {
    /// The same directory: a path collision.
    Same,
    /// The first path is inside the second.
    Inside,
    /// The first path contains the second.
    Contains,
    /// Neither contains the other: the only admissible relation.
    Disjoint,
}

impl PathRelation {
    /// Every relation but [`Disjoint`](Self::Disjoint) refuses.
    pub const fn overlaps(self) -> bool {
        !matches!(self, Self::Disjoint)
    }
}

/// Relate two member paths by their components.
pub fn relate(a: &MemberPath, b: &MemberPath) -> PathRelation {
    let mut left = a.components();
    let mut right = b.components();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return PathRelation::Same,
            (Some(_), None) => return PathRelation::Inside,
            (None, Some(_)) => return PathRelation::Contains,
            (Some(one), Some(other)) if one == other => continue,
            _ => return PathRelation::Disjoint,
        }
    }
}

/// Normalise any spelling of a root-relative member path.
///
/// Accepts `\` as a separator, collapses `.`, empty and interior `..`
/// components, and drops a trailing separator. Refuses an absolute path and
/// any path that is, is inside, or contains the root.
pub fn normalize(path: &str) -> Result<MemberPath, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.chars().any(char::is_control) {
        return Err(PathError::ControlCharacter {
            path: path.to_owned(),
        });
    }
    let unified = path.replace('\\', "/");
    if unified.starts_with('/') || has_drive_prefix(&unified) {
        return Err(PathError::Absolute {
            path: path.to_owned(),
        });
    }
    let mut components: Vec<&str> = Vec::new();
    let mut leading_up = 0usize;
    for component in unified.split('/') {
        match component {
            "" | "." => continue,
            ".." => {
                if components.len() > leading_up {
                    components.pop();
                } else {
                    components.push("..");
                    leading_up += 1;
                }
            }
            ordinary => components.push(ordinary),
        }
    }
    if components.is_empty() {
        return Err(PathError::RootItself {
            path: path.to_owned(),
        });
    }
    let normalised = components.join("/");
    if leading_up == 0 {
        return Err(PathError::InsideRoot {
            path: path.to_owned(),
            normalised,
        });
    }
    if components.len() == leading_up {
        return Err(PathError::ContainsRoot {
            path: path.to_owned(),
            normalised,
        });
    }
    Ok(MemberPath(normalised))
}

/// [`normalize`], and additionally require `path` to be spelled the way the
/// index records it. This is the check a recorded row must pass.
pub fn validate(path: &str) -> Result<MemberPath, PathError> {
    let member = normalize(path)?;
    if member.as_str() != path {
        return Err(PathError::NotNormalised {
            path: path.to_owned(),
            normalised: member.0,
        });
    }
    Ok(member)
}

/// `C:`, `C:/x` or `C:x`: a Windows drive-qualified path.
fn has_drive_prefix(path: &str) -> bool {
    let mut chars = path.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic()) && chars.next() == Some(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalised(path: &str) -> String {
        normalize(path).unwrap().as_str().to_owned()
    }

    #[test]
    fn normalisation_collapses_every_spelling_of_one_path() {
        for spelling in [
            "../ws-A",
            "../ws-A/",
            "../ws-A/.",
            ".././ws-A",
            "..//ws-A",
            "../other/../ws-A",
            "..\\ws-A",
            "../ws-A///",
        ] {
            assert_eq!(normalised(spelling), "../ws-A", "{spelling}");
        }
        assert_eq!(normalised("../../elsewhere/ws-A"), "../../elsewhere/ws-A");
        assert_eq!(normalised("../a/b/c"), "../a/b/c");
        assert_eq!(normalised("../a/b/../c"), "../a/c");
    }

    #[test]
    fn only_the_recorded_spelling_validates() {
        assert_eq!(validate("../ws-A").unwrap().as_str(), "../ws-A");
        let refusal = validate("../ws-A/").unwrap_err();
        assert_eq!(
            refusal,
            PathError::NotNormalised {
                path: "../ws-A/".to_owned(),
                normalised: "../ws-A".to_owned(),
            }
        );
        assert_eq!(refusal.normalised(), Some("../ws-A"));
        assert!(refusal.to_string().contains("is not normalised"));
    }

    #[test]
    fn a_member_path_escapes_the_root_and_is_never_a_host_path() {
        assert_eq!(normalize("").unwrap_err(), PathError::Empty);
        for absolute in ["/tmp/x", "//server/share", "C:/ws", "c:ws", "\\\\srv\\s"] {
            assert!(
                matches!(normalize(absolute), Err(PathError::Absolute { .. })),
                "{absolute}"
            );
        }
        assert!(matches!(
            normalize("../ws\nA"),
            Err(PathError::ControlCharacter { .. })
        ));
        for root in [".", "./", "ws/..", "./."] {
            assert!(
                matches!(normalize(root), Err(PathError::RootItself { .. })),
                "{root}"
            );
        }
        for inside in ["sub", "sub/dir", "./sub/dir", "sub/../other"] {
            assert!(
                matches!(normalize(inside), Err(PathError::InsideRoot { .. })),
                "{inside}"
            );
        }
        for ancestor in ["..", "../", "../.", "../..", "../x/../.."] {
            assert!(
                matches!(normalize(ancestor), Err(PathError::ContainsRoot { .. })),
                "{ancestor}"
            );
        }
    }

    #[test]
    fn relation_is_decided_component_wise() {
        let a = normalize("../ws-A").unwrap();
        let a_again = normalize("../ws-A/").unwrap();
        let inner = normalize("../ws-A/inner").unwrap();
        let sibling = normalize("../ws-AB").unwrap();
        let uncle = normalize("../../elsewhere/ws-A").unwrap();
        assert_eq!(relate(&a, &a_again), PathRelation::Same);
        assert_eq!(relate(&inner, &a), PathRelation::Inside);
        assert_eq!(relate(&a, &inner), PathRelation::Contains);
        assert_eq!(
            relate(&a, &sibling),
            PathRelation::Disjoint,
            "a name prefix is not a path prefix"
        );
        assert_eq!(relate(&a, &uncle), PathRelation::Disjoint);
        assert!(PathRelation::Same.overlaps());
        assert!(PathRelation::Inside.overlaps());
        assert!(PathRelation::Contains.overlaps());
        assert!(!PathRelation::Disjoint.overlaps());
    }
}
