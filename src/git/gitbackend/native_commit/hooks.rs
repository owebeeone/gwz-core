//! Which Git hooks would fire.
//!
//! Git runs a hook when a file with the hook's name sits in the hooks
//! directory and is executable. The hooks directory is `core.hooksPath` when
//! set (relative to the worktree root), else `hooks` in the repository
//! directory; a linked worktree shares the common directory's `hooks`.

use std::path::{Path, PathBuf};

/// Hooks `git commit` can run for a commit made with `-m`.
pub(super) const COMMIT_HOOKS: &[&str] = &[
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "reference-transaction",
    "post-index-change",
];

/// Hooks `git tag` and `git tag -d` can run: ordinary commit hooks do not
/// run for tags, only ref updates are announced.
pub(super) const TAG_HOOKS: &[&str] = &["reference-transaction"];

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;

        /// Git ignores a hook that is not executable. An entry that cannot be
        /// read for any other reason than being absent counts as a hook.
        fn hook_present(path: &Path) -> bool {
            match std::fs::metadata(path) {
                Ok(meta) => meta.permissions().mode() & 0o111 != 0,
                Err(error) => error.kind() != std::io::ErrorKind::NotFound,
            }
        }
    } else {
        /// On Windows any file with the hook's exact name counts, and so does
        /// the `.exe` Git for Windows also tries.
        fn hook_present(path: &Path) -> bool {
            let exe = {
                let mut name = path.as_os_str().to_owned();
                name.push(".exe");
                PathBuf::from(name)
            };
            [path, exe.as_path()]
                .iter()
                .any(|candidate| match std::fs::symlink_metadata(candidate) {
                    Ok(_) => true,
                    Err(error) => error.kind() != std::io::ErrorKind::NotFound,
                })
        }
    }
}

/// The first of `names` that would fire from any of `directories`.
pub(super) fn firing_hook(directories: &[PathBuf], names: &[&str]) -> Option<String> {
    for directory in directories {
        for name in names {
            if hook_present(&directory.join(name)) {
                return Some((*name).to_owned());
            }
        }
    }
    None
}

/// Why the hooks directory cannot be told.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum HooksPathError {
    /// The value is empty, or a form native does not expand (`~user/`).
    Unsupported,
}

/// The directories Git looks in for hooks.
///
/// `configured` is the raw `core.hooksPath` value, `expanded` the same with a
/// leading `~/` expanded by the configuration reader.
pub(super) fn hook_directories(
    configured: Option<&str>,
    expanded: Option<PathBuf>,
    worktree: &Path,
    git_dir: &Path,
    common_dir: &Path,
) -> Result<Vec<PathBuf>, HooksPathError> {
    if let Some(raw) = configured {
        if raw.is_empty() {
            return Err(HooksPathError::Unsupported);
        }
        if raw.starts_with('~') && expanded.is_none() {
            return Err(HooksPathError::Unsupported);
        }
        if raw.contains("%(prefix)") {
            return Err(HooksPathError::Unsupported);
        }
        let path = expanded.unwrap_or_else(|| PathBuf::from(raw));
        return Ok(vec![if path.is_absolute() {
            path
        } else {
            worktree.join(path)
        }]);
    }
    let mut directories = vec![git_dir.join("hooks")];
    let common = common_dir.join("hooks");
    if !directories.contains(&common) {
        directories.push(common);
    }
    Ok(directories)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_directories_include_the_common_directory_once() {
        let same = hook_directories(
            None,
            None,
            Path::new("/w"),
            Path::new("/g"),
            Path::new("/g"),
        )
        .unwrap();
        assert_eq!(same, vec![PathBuf::from("/g/hooks")]);
        let linked = hook_directories(
            None,
            None,
            Path::new("/w"),
            Path::new("/g/wt"),
            Path::new("/g"),
        )
        .unwrap();
        assert_eq!(
            linked,
            vec![PathBuf::from("/g/wt/hooks"), PathBuf::from("/g/hooks")]
        );
    }

    #[test]
    fn configured_path_is_relative_to_the_worktree_and_odd_forms_are_refused() {
        let relative = hook_directories(
            Some(".githooks"),
            None,
            Path::new("/w"),
            Path::new("/g"),
            Path::new("/g"),
        )
        .unwrap();
        assert_eq!(relative, vec![PathBuf::from("/w/.githooks")]);
        for odd in ["", "~someone/hooks", "%(prefix)/hooks"] {
            assert_eq!(
                hook_directories(
                    Some(odd),
                    None,
                    Path::new("/w"),
                    Path::new("/g"),
                    Path::new("/g")
                ),
                Err(HooksPathError::Unsupported),
                "{odd}"
            );
        }
    }
}
