use std::fs;
use std::path::{Component, Path, PathBuf};

/// Resolve an existing path for the few callers that require physical identity,
/// then put its representation through the shared lexical normalizer.
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    lexical_normalize(&canonical)
}

/// Normalize `.` and `..` without requiring the operand to exist.
///
/// Routing uses this for raw pathspecs, so it deliberately does not resolve
/// symlinks. On Windows it also maps the ordinary DOS/UNC and canonical
/// verbatim spellings into one component representation before comparisons.
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
