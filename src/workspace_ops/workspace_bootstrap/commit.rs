use super::*;
use crate::git::GitRepositoryState;

const OPTIONAL_OUTPUTS: [&str; 2] = [AGENTS_PATH, CLAUDE_SETTINGS_PATH];

pub(super) fn optional_outputs(
    filesystem: &dyn FileSystem,
    root: &Path,
) -> ModelResult<Vec<Option<Vec<u8>>>> {
    OPTIONAL_OUTPUTS
        .iter()
        .map(|path| read_optional_bytes(filesystem, &root.join(path)))
        .collect()
}

pub(super) fn preflight<B: GitBackend>(backend: &B, root: &Path) -> ModelResult<Option<String>> {
    let head = backend.head(root)?;
    if head.is_detached
        || backend.repository_state(root)? != GitRepositoryState::Clean
        || backend.status(root)?.unresolved != 0
    {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "init --update --commit requires an attached root with no native Git operation or unresolved conflicts",
        ));
    }
    Ok(head.commit)
}

pub(super) fn publish<B: GitBackend>(
    filesystem: &dyn FileSystem,
    backend: &B,
    root: &Path,
    head: Option<&str>,
    before: &[Option<Vec<u8>>],
) -> ModelResult<Option<String>> {
    let mut paths = Vec::new();
    for path in [
        crate::workspace::WORKSPACE_MANIFEST,
        artifact::LOCK_PATH,
        artifact::CONF_INTEGRITY_MARKER_PATH,
        AGENTS_GWZ_PATH,
    ] {
        if path_exists_in(filesystem, &root.join(path))? {
            paths.push(path);
        }
    }
    for (path, old) in OPTIONAL_OUTPUTS.into_iter().zip(before) {
        if let Some(contents) = read_optional_bytes(filesystem, &root.join(path))?
            && old.as_ref() != Some(&contents)
        {
            paths.push(path);
        }
    }
    // Staging only these paths leaves every unrelated index entry intact. The
    // commit itself is built from HEAD plus only these staged paths, including
    // Git's normal clean filters, never the whole index.
    backend.stage_paths(root, &paths)?;
    let result = backend.commit_bootstrap_paths_checked(
        root,
        head,
        &paths,
        "Accept workspace configuration and refresh managed bootstrap files",
    )?;
    Ok(result.map(|commit| format!("committed {} ({})", commit.commit, paths.join(", "))))
}

fn read_optional_bytes(filesystem: &dyn FileSystem, path: &Path) -> ModelResult<Option<Vec<u8>>> {
    match filesystem.read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}
