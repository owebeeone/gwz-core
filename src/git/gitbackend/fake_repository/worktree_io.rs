//! Worktree and repository-path helpers backing the fake repository.
//!
//! All physical access goes through `FileSystem`; nothing here touches
//! `std::fs` or a Git process.

use super::*;

pub(super) fn failed(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::GitCommandFailed, message)
}
pub(super) fn read_worktree(
    filesystem: &dyn FileSystem,
    root: &Path,
    ignored: &[String],
    tracked: &FileTree,
) -> ModelResult<FileTree> {
    fn visit(
        filesystem: &dyn FileSystem,
        root: &Path,
        directory: &Path,
        files: &mut FileTree,
        ignored: &[String],
        tracked: &FileTree,
    ) -> ModelResult<()> {
        for entry in filesystem
            .read_directory(directory)
            .map_err(|e| failed(e.to_string()))?
        {
            if entry.name == ".git" {
                continue;
            }
            let path = directory.join(&entry.name);
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let prefix = format!("{name}/");
            let excluded = ignored
                .iter()
                .any(|rule| name == rule.trim_end_matches('/') || name.starts_with(rule));
            if excluded
                && !tracked.contains_key(&name)
                && !tracked.keys().any(|key| key.starts_with(&prefix))
            {
                continue;
            }
            if entry.kind == FsKind::Directory {
                visit(filesystem, root, &path, files, ignored, tracked)?;
            } else if entry.kind == FsKind::File {
                files.insert(
                    name,
                    filesystem.read(&path).map_err(|e| failed(e.to_string()))?,
                );
            } else {
                return unsupported("non-regular worktree entry");
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(filesystem, root, root, &mut files, ignored, tracked)?;
    Ok(files)
}

pub(super) fn repository_key(filesystem: &dyn FileSystem, path: &Path) -> PathBuf {
    let canonical = filesystem
        .canonical_path(path)
        .unwrap_or_else(|_| path.to_path_buf());
    if canonical.file_name().is_some_and(|name| name == ".git") {
        canonical.parent().unwrap().to_path_buf()
    } else {
        canonical
    }
}

pub(super) fn write_worktree_file(
    filesystem: &dyn FileSystem,
    path: &Path,
    bytes: &[u8],
) -> ModelResult<()> {
    filesystem
        .create_directories(
            path.parent()
                .ok_or_else(|| failed("worktree file has no parent"))?,
        )
        .map_err(|e| failed(e.to_string()))?;
    match filesystem.remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error.to_string())),
    }
    let file = filesystem
        .create_file(path)
        .map_err(|e| failed(e.to_string()))?;
    filesystem
        .write_all(&file, bytes)
        .map_err(|e| failed(e.to_string()))
}

pub(super) fn remove_worktree_file(filesystem: &dyn FileSystem, path: &Path) -> ModelResult<()> {
    filesystem
        .remove_file(path)
        .map_err(|e| failed(e.to_string()))
}
