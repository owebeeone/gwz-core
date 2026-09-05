//! Core-minted values for one create: the family and allocation ids, the
//! destination's host path and its root-relative recorded path.
//!
//! The recorded path is what the store keys everything on (`root.join(path)`
//! is the row's destination for `install_pointer`, `remove_pointer` and the
//! `RemoveRow`/`Disband` guard), so it is computed once here, from the
//! canonical root and the canonical parent of the destination, and never
//! from a spelling the host happened to receive.

use std::path::{Component, Path, PathBuf};

use gwz_family_model::{AllocationId, FamilyId, MemberName, MemberPath, normalize_member_path};

use crate::model::{ErrorCode, ModelError, ModelResult};

/// `fam_<32 hex>`: the core-derived family identity (design §3, §11 item 6).
pub fn mint_family_id() -> ModelResult<FamilyId> {
    FamilyId::new(format!("fam_{}", random_hex()?)).map_err(internal)
}

/// `alloc_<32 hex>`: an ordinary allocation marker value (design §3).
pub fn mint_allocation_id() -> ModelResult<AllocationId> {
    AllocationId::new(format!("alloc_{}", random_hex()?)).map_err(internal)
}

fn random_hex() -> ModelResult<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ModelError::new(
            ErrorCode::InternalError,
            format!("could not mint a local family id: {error}"),
        )
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn internal(error: impl std::fmt::Display) -> ModelError {
    ModelError::new(ErrorCode::InternalError, error.to_string())
}

/// The destination a create addresses: `dest` as given (a relative spelling
/// is taken against `start`, the invocation's own directory), or the
/// default sibling `../<root-dirname>-<Name>` of the family root (design §4,
/// §11 item 3).
pub fn destination_path(
    start: &Path,
    dest: Option<&str>,
    root: &Path,
    name: &MemberName,
) -> ModelResult<PathBuf> {
    match dest {
        Some(dest) => {
            let dest = Path::new(dest);
            if dest.is_absolute() {
                Ok(dest.to_path_buf())
            } else {
                let base = if start.is_file() {
                    start.parent().unwrap_or(start)
                } else {
                    start
                };
                Ok(base.join(dest))
            }
        }
        None => {
            let dirname = root
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    ModelError::new(
                        ErrorCode::InvalidRequest,
                        format!(
                            "the family root {} has no directory name to derive a default \
                             destination from; pass dest explicitly",
                            root.display()
                        ),
                    )
                })?;
            let parent = root.parent().ok_or_else(|| {
                ModelError::new(
                    ErrorCode::InvalidRequest,
                    format!(
                        "the family root {} has no parent directory; pass dest explicitly",
                        root.display()
                    ),
                )
            })?;
            Ok(parent.join(format!("{dirname}-{}", name.as_str())))
        }
    }
}

/// The destination's parent resolved through the filesystem plus its final
/// component: the spelling a not-yet-existing destination canonicalises to
/// once it is allocated, and the one the store's resolution will agree with.
pub fn intended_destination(destination: &Path) -> ModelResult<PathBuf> {
    let name = destination.file_name().ok_or_else(|| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            format!(
                "destination {} has no final path component",
                destination.display()
            ),
        )
    })?;
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let parent = std::fs::canonicalize(&parent).map_err(|error| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            format!(
                "destination parent {} does not resolve: {error}",
                parent.display()
            ),
        )
    })?;
    Ok(parent.join(name))
}

/// The root-relative recorded path of `destination` (design §3: "Paths are
/// root-relative"): both inputs are absolute, lexically clean paths (the
/// canonical root and [`intended_destination`]); the result must escape the
/// root, which the model's own validation enforces.
pub fn recorded_path(root: &Path, destination: &Path) -> ModelResult<MemberPath> {
    let relative = relative_between(root, destination).ok_or_else(|| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            format!(
                "no root-relative path leads from {} to {}",
                root.display(),
                destination.display()
            ),
        )
    })?;
    normalize_member_path(&relative).map_err(|error| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            format!(
                "destination {} is not a usable family member path: {error}",
                destination.display()
            ),
        )
    })
}

/// A `/`-separated relative path from `from` to `to`, both absolute. `None`
/// when they share no prefix (a different Windows drive) or a component is
/// not UTF-8, which no recorded path can carry.
fn relative_between(from: &Path, to: &Path) -> Option<String> {
    let from: Vec<Component<'_>> = from.components().collect();
    let to: Vec<Component<'_>> = to.components().collect();
    let common = from
        .iter()
        .zip(to.iter())
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for component in &from[common..] {
        match component {
            Component::Normal(_) => parts.push("..".to_owned()),
            _ => return None,
        }
    }
    for component in &to[common..] {
        match component {
            Component::Normal(name) => parts.push(name.to_str()?.to_owned()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        return Some(".".to_owned());
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_ids_carry_their_prefix_and_differ() {
        let family = mint_family_id().unwrap();
        assert!(family.as_str().starts_with("fam_"), "{family}");
        assert_eq!(family.as_str().len(), 4 + 32);
        assert_ne!(mint_family_id().unwrap(), family);
        let allocation = mint_allocation_id().unwrap();
        assert!(allocation.as_str().starts_with("alloc_"), "{allocation}");
        assert_ne!(mint_allocation_id().unwrap(), allocation);
    }

    #[test]
    fn the_default_destination_is_the_roots_sibling_named_after_the_clone() {
        let root = Path::new("/home/u/limbo/gwz-dev");
        let name = MemberName::parse("A").unwrap();
        assert_eq!(
            destination_path(Path::new("/home/u/limbo/gwz-dev/sub"), None, root, &name).unwrap(),
            PathBuf::from("/home/u/limbo/gwz-dev-A")
        );
        assert_eq!(
            destination_path(Path::new("/cwd"), Some("/elsewhere/A"), root, &name).unwrap(),
            PathBuf::from("/elsewhere/A")
        );
        assert_eq!(
            destination_path(Path::new("/cwd"), Some("../lanes/A"), root, &name).unwrap(),
            PathBuf::from("/cwd/../lanes/A"),
            "a relative dest is taken against the invocation directory"
        );
    }

    #[test]
    fn the_recorded_path_escapes_the_root_and_is_normalised() {
        let root = Path::new("/home/u/limbo/gwz-dev");
        assert_eq!(
            recorded_path(root, Path::new("/home/u/limbo/gwz-dev-A"))
                .unwrap()
                .as_str(),
            "../gwz-dev-A"
        );
        assert_eq!(
            recorded_path(root, Path::new("/tmp/lanes/A"))
                .unwrap()
                .as_str(),
            "../../../../tmp/lanes/A"
        );
        let inside = recorded_path(root, Path::new("/home/u/limbo/gwz-dev/nested")).unwrap_err();
        assert_eq!(inside.code, ErrorCode::InvalidRequest);
        assert!(
            inside.message.contains("inside the root"),
            "{}",
            inside.message
        );
        let itself = recorded_path(root, root).unwrap_err();
        assert!(itself.message.contains("root itself"), "{}", itself.message);
        let above = recorded_path(root, Path::new("/home/u/limbo")).unwrap_err();
        assert!(
            above.message.contains("contains the root"),
            "{}",
            above.message
        );
    }

    #[test]
    fn the_intended_destination_resolves_the_parent_and_keeps_the_name() {
        let temp = tempfile::tempdir().unwrap();
        let parent = std::fs::canonicalize(temp.path()).unwrap();
        std::fs::create_dir(temp.path().join("sub")).unwrap();
        let intended = intended_destination(&temp.path().join("sub/../ws-A")).unwrap();
        assert_eq!(intended, parent.join("ws-A"));
        assert!(!intended.exists(), "nothing is created");
        let missing = intended_destination(&temp.path().join("absent/ws-A")).unwrap_err();
        assert_eq!(missing.code, ErrorCode::InvalidRequest);
        assert!(
            missing.message.contains("does not resolve"),
            "{}",
            missing.message
        );
        // `dest=.` names the invocation directory itself (`Path::file_name`
        // drops a trailing `.`), as `git clone <url> .` does; the model's
        // path rules then decide whether that directory is a usable member.
        assert_eq!(
            intended_destination(&temp.path().join(".")).unwrap(),
            parent
        );
        let dotdot = intended_destination(&temp.path().join("..")).unwrap_err();
        assert!(
            dotdot.message.contains("no final path component"),
            "{}",
            dotdot.message
        );
    }
}
