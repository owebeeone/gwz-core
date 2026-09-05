//! Boundary containment: does a path stay inside the tree that will be
//! copied, once every symlink on the way has been followed?
//!
//! Design §4.0 refuses "Git metadata or an object-store path that resolves
//! through a symlink outside that repository's own copied boundary" and any
//! configured path that "would still name a path **outside dest** after
//! copy". Both questions reduce to one: normalise the candidate against the
//! base Git would use, follow the symlinks that exist, and compare the result
//! with the boundary. The comparison is normalised, never textual on the raw
//! value.
//!
//! Two rules keep this read-only and bounded:
//!
//! - a symlink that points out of the boundary is **reported, not followed**
//!   (architecture §4: "do not follow user symlinks outside the tree");
//! - link following is capped ([`MAX_LINK_HOPS`]), so a loop is an
//!   unresolvable configuration rather than a hang.

use std::io;
use std::path::{Component, Path, PathBuf};

/// Symlinks followed before resolution gives up. A cycle therefore reports
/// [`Resolution::Unresolvable`] instead of spinning.
const MAX_LINK_HOPS: u32 = 32;

/// What resolving one configured or metadata path against a boundary found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// The path stays inside the boundary. Carries the normalised location;
    /// the tail may not exist yet, which is not by itself a hazard.
    Inside(PathBuf),
    /// The normalised path lies outside the boundary without any symlink
    /// being involved (an absolute path, or one that climbs out with `..`).
    Outside(PathBuf),
    /// A symlink on the way pointed out of the boundary. The link was
    /// **not** followed further.
    Escapes { link: PathBuf, target: PathBuf },
    /// Resolution could not be completed: a loop, a permission error, a
    /// component that is not a directory. Never an admission.
    Unresolvable(String),
}

impl Resolution {
    pub(crate) fn is_inside(&self) -> bool {
        matches!(self, Self::Inside(_))
    }
}

/// Textual normalisation: drop `.`, apply `..` to the accumulated prefix.
/// The filesystem is not touched, so this never follows a link and never
/// fails. `..` at the top of a relative path is kept, which leaves the result
/// outside any boundary — the safe answer.
pub(crate) fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(Component::ParentDir.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(Component::CurDir.as_os_str());
    }
    out
}

/// Component-wise containment. `/a/bc` is not inside `/a/b`.
pub(crate) fn contains(boundary: &Path, candidate: &Path) -> bool {
    candidate == boundary || candidate.starts_with(boundary)
}

/// The real location of an existing path, with every symlink followed.
pub(crate) fn real(path: &Path) -> io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// A Git path (raw bytes, `/` separators) as a platform path. Git paths need
/// not be UTF-8; on a platform whose paths must be, an undecodable name has
/// no representation and the caller records the entry as unknown rather than
/// guessing at a lossy spelling.
pub(crate) fn byte_path_to_path(bytes: &[u8]) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes).ok().map(PathBuf::from)
    }
}

/// Resolve `candidate` the way Git would — relative to `base`, absolute taken
/// as written — and report where it lands relative to `boundary`.
///
/// `boundary` and `base` must already be real paths ([`real`]); `base` must be
/// inside `boundary`. Components that do not exist are taken normalised: a
/// hook directory that has not been created yet still has a knowable
/// location, and knowing it is inside the boundary is enough to admit it.
pub(crate) fn resolve_within(boundary: &Path, base: &Path, candidate: &Path) -> Resolution {
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(candidate)
    };
    resolve_normalised(boundary, &normalise(&joined), 0)
}

/// The path with its longest existing prefix resolved, and the rest appended
/// as written. Unlike [`real`] this never fails: a location that does not
/// exist yet still has a knowable place.
fn resolve_existing_prefix(path: &Path) -> PathBuf {
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    let mut prefix = path;
    loop {
        if let Ok(resolved) = std::fs::canonicalize(prefix) {
            let mut out = resolved;
            for component in tail.iter().rev() {
                out.push(component);
            }
            return out;
        }
        match (prefix.parent(), prefix.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name);
                prefix = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn resolve_normalised(boundary: &Path, normalised: &Path, hops: u32) -> Resolution {
    if hops > MAX_LINK_HOPS {
        return Resolution::Unresolvable(format!(
            "{} passes through more than {MAX_LINK_HOPS} symlinks",
            normalised.display()
        ));
    }
    if !contains(boundary, normalised) {
        return Resolution::Outside(normalised.to_path_buf());
    }

    // Walk from the boundary outwards, so only components the boundary
    // actually owns are ever stat-ed.
    let Ok(tail) = normalised.strip_prefix(boundary) else {
        return Resolution::Outside(normalised.to_path_buf());
    };
    let mut walked = boundary.to_path_buf();
    let mut remaining: Vec<_> = tail.components().collect();
    remaining.reverse();
    while let Some(component) = remaining.pop() {
        walked.push(component.as_os_str());
        let metadata = match std::fs::symlink_metadata(&walked) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Nothing left to follow; the rest is already normalised and
                // inside the boundary.
                for left in remaining.iter().rev() {
                    walked.push(left.as_os_str());
                }
                return Resolution::Inside(walked);
            }
            Err(error) => {
                return Resolution::Unresolvable(format!("{}: {error}", walked.display()));
            }
        };
        if !metadata.file_type().is_symlink() {
            continue;
        }
        let target = match std::fs::read_link(&walked) {
            Ok(target) => target,
            Err(error) => {
                return Resolution::Unresolvable(format!("{}: {error}", walked.display()));
            }
        };
        let parent = walked.parent().unwrap_or(boundary).to_path_buf();
        let pointed = if target.is_absolute() {
            normalise(&target)
        } else {
            normalise(&parent.join(&target))
        };
        // A link may name its own tree through another symlink (`/var` is
        // `/private/var` on macOS), so containment is decided on the resolved
        // location, not on the spelling the link happens to carry.
        let pointed = resolve_existing_prefix(&pointed);
        if !contains(boundary, &pointed) {
            // Report it; never follow a link out of the tree.
            return Resolution::Escapes {
                link: walked,
                target: pointed,
            };
        }
        let mut rest = pointed;
        for left in remaining.iter().rev() {
            rest.push(left.as_os_str());
        }
        return resolve_normalised(boundary, &normalise(&rest), hops + 1);
    }
    Resolution::Inside(walked)
}

#[cfg(test)]
mod tests {
    // Fixture setup writes; the merge-path writer boundary governs production
    // merge code, not test scaffolding.
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use std::fs;

    #[test]
    fn normalisation_applies_dot_and_dotdot_without_touching_the_filesystem() {
        assert_eq!(normalise(Path::new("a/./b/../c")), PathBuf::from("a/c"));
        assert_eq!(normalise(Path::new("/a/b/../../c")), PathBuf::from("/c"));
        assert_eq!(normalise(Path::new("../out")), PathBuf::from("../out"));
        assert_eq!(normalise(Path::new("")), PathBuf::from("."));
    }

    #[test]
    fn containment_is_component_wise_not_textual() {
        assert!(contains(Path::new("/a/b"), Path::new("/a/b")));
        assert!(contains(Path::new("/a/b"), Path::new("/a/b/c")));
        assert!(!contains(Path::new("/a/b"), Path::new("/a/bc")));
        assert!(!contains(Path::new("/a/b"), Path::new("/a")));
    }

    #[test]
    fn a_relative_path_resolves_against_the_base_and_a_missing_tail_is_inside() {
        let temp = tempfile::tempdir().expect("tempdir");
        let boundary = real(temp.path()).expect("real boundary");
        fs::create_dir_all(boundary.join("git")).expect("git dir");
        assert_eq!(
            resolve_within(&boundary, &boundary, Path::new("hooks/never-created")),
            Resolution::Inside(boundary.join("hooks/never-created"))
        );
        assert_eq!(
            resolve_within(&boundary, &boundary.join("git"), Path::new("../hooks")),
            Resolution::Inside(boundary.join("hooks"))
        );
    }

    #[test]
    fn climbing_out_or_naming_an_absolute_path_is_outside() {
        let temp = tempfile::tempdir().expect("tempdir");
        let boundary = real(temp.path()).expect("real boundary");
        assert_eq!(
            resolve_within(&boundary, &boundary, Path::new("../elsewhere")),
            Resolution::Outside(normalise(&boundary.join("../elsewhere")))
        );
        assert_eq!(
            resolve_within(&boundary, &boundary, Path::new("/etc/hooks")),
            Resolution::Outside(PathBuf::from("/etc/hooks"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_escaping_symlink_is_reported_and_not_followed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = real(temp.path()).expect("real root");
        let boundary = root.join("inside");
        fs::create_dir_all(boundary.join("deep")).expect("deep");
        fs::create_dir_all(root.join("outside")).expect("outside");
        std::os::unix::fs::symlink(root.join("outside"), boundary.join("deep/link"))
            .expect("symlink");
        assert_eq!(
            resolve_within(&boundary, &boundary, Path::new("deep/link/hooks")),
            Resolution::Escapes {
                link: boundary.join("deep/link"),
                target: root.join("outside"),
            }
        );
        // An internal link is followed and stays inside.
        std::os::unix::fs::symlink(boundary.join("deep"), boundary.join("inner")).expect("symlink");
        assert_eq!(
            resolve_within(&boundary, &boundary, Path::new("inner/hooks")),
            Resolution::Inside(boundary.join("deep/hooks"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_is_unresolvable_rather_than_a_hang() {
        let temp = tempfile::tempdir().expect("tempdir");
        let boundary = real(temp.path()).expect("real boundary");
        std::os::unix::fs::symlink(boundary.join("b"), boundary.join("a")).expect("a");
        std::os::unix::fs::symlink(boundary.join("a"), boundary.join("b")).expect("b");
        assert!(matches!(
            resolve_within(&boundary, &boundary, Path::new("a")),
            Resolution::Unresolvable(_)
        ));
    }
}
