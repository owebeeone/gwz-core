use std::fs;
use std::path::{Component, Path, PathBuf};

/// Resolve an existing path for the few callers that require physical identity,
/// then put its representation through the shared lexical normalizer.
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    lexical_normalize(&canonical)
}

/// Resolve the existing workspace and caller bases to physical paths for
/// containment and routing comparisons. This consumes explicit request context;
/// it never reads the executor's current directory. A caller outside the
/// workspace remains outside after normalization and is rejected by routing.
pub(crate) fn normalize_routing_bases(
    workspace_root: &Path,
    caller_cwd: &Path,
) -> (PathBuf, PathBuf) {
    (normalize_path(workspace_root), normalize_path(caller_cwd))
}

/// The physical spelling of an existing path, or `None` when it cannot be
/// resolved (it is missing, or a dangling or looping link).
pub(crate) fn canonical_existing_path(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path)
        .ok()
        .map(|canonical| lexical_normalize(&canonical))
}

/// The physical spelling of an absolute path that need not exist.
///
/// The longest prefix that resolves is replaced by its physical path, and the
/// remaining components, which do not resolve, are re-attached as written.
/// `None` when one of those is itself a link (dangling or looping), because
/// where it leads cannot be shown. A relative path stays lexical: this never
/// consults the process working directory.
pub(crate) fn physical_spelling(path: &Path) -> Option<PathBuf> {
    let path = lexical_normalize(path);
    if !path.is_absolute() {
        return Some(path);
    }
    let mut existing = path.as_path();
    let mut unresolved = Vec::new();
    loop {
        if let Some(mut physical) = canonical_existing_path(existing) {
            physical.extend(unresolved.iter().rev());
            return Some(physical);
        }
        if fs::symlink_metadata(existing).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return None;
        }
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return Some(path.clone());
        };
        unresolved.push(name);
        existing = parent;
    }
}

/// Normalize `.` and `..` without requiring the operand to exist.
///
/// Routing applies this to raw pathspecs before physical containment, so it
/// deliberately does not resolve symlinks itself. On Windows it also maps the
/// ordinary DOS/UNC and canonical verbatim spellings into one component
/// representation before comparisons.
pub(crate) fn lexical_normalize(path: &Path) -> PathBuf {
    let path = normalize_windows_path_representation(path);
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        fn normalize_windows_path_representation(path: &Path) -> PathBuf {
            use std::ffi::OsString;
            use std::path::Prefix;

            let mut components = path.components();
            let Some(Component::Prefix(prefix)) = components.next() else {
                return path.to_path_buf();
            };
            let mut normalized = match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                    PathBuf::from(format!("{}:", (drive as char).to_ascii_uppercase()))
                }
                Prefix::VerbatimUNC(server, share) => {
                    let mut unc = OsString::from(r"\\");
                    unc.push(server);
                    unc.push("\\");
                    unc.push(share);
                    PathBuf::from(unc)
                },
                _ => return path.to_path_buf(),
            };
            for component in components {
                normalized.push(component.as_os_str());
            }
            normalized
        }
    } else {
        fn normalize_windows_path_representation(path: &Path) -> PathBuf {
            path.to_path_buf()
        }
    }
}
