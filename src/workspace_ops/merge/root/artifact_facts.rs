use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::artifact;
use crate::model::{ErrorCode, ModelError, ModelResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum RegularFileFact {
    Missing,
    Bytes(Vec<u8>),
    Invalid,
}

pub(in crate::workspace_ops::merge) fn observe(
    root: &Path,
    relative: &str,
) -> ModelResult<RegularFileFact> {
    let path = checked_path(root, relative)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RegularFileFact::Missing);
        }
        Err(error) => return Err(io_error(relative, error)),
    };
    if !metadata.file_type().is_file() || executable(&metadata) {
        return Ok(RegularFileFact::Invalid);
    }
    fs::read(&path)
        .map(RegularFileFact::Bytes)
        .map_err(|error| io_error(relative, error))
}

pub(in crate::workspace_ops::merge) fn write_checked(
    root: &Path,
    relative: &str,
    expected: &[u8],
    bytes: &[u8],
) -> ModelResult<()> {
    let path = checked_path(root, relative)?;
    if observe(root, relative)? != RegularFileFact::Bytes(expected.to_vec()) {
        return Err(invalid(relative));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("workspace artifact '{relative}' is not UTF-8"),
        )
    })?;
    artifact::write_atomic(&path, text)
}

pub(in crate::workspace_ops::merge) fn remove_exact(
    root: &Path,
    relative: &str,
    expected: &[u8],
) -> ModelResult<()> {
    match observe(root, relative)? {
        RegularFileFact::Bytes(bytes) if bytes == expected => {
            fs::remove_file(checked_path(root, relative)?)
                .map_err(|error| io_error(relative, error))
        }
        RegularFileFact::Missing => Ok(()),
        RegularFileFact::Bytes(_) | RegularFileFact::Invalid => Err(invalid(relative)),
    }
}

fn checked_path(root: &Path, relative: &str) -> ModelResult<PathBuf> {
    require_real_directory(root, "workspace root")?;
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid(&relative.to_string_lossy()));
    }
    let mut path = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        path.push(component.as_os_str());
        require_real_directory(&path, &path.display().to_string())?;
    }
    path.push(components.last().expect("non-empty path").as_os_str());
    Ok(path)
}

fn require_real_directory(path: &Path, label: &str) -> ModelResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(label, error))?;
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(invalid(label))
    }
}

#[cfg(unix)]
fn executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &fs::Metadata) -> bool {
    false
}

fn invalid(path: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("workspace artifact '{path}' is not a canonical regular file"),
    )
}

fn io_error(path: &str, error: std::io::Error) -> ModelError {
    ModelError::new(
        ErrorCode::IoError,
        format!("failed to inspect workspace artifact '{path}': {error}"),
    )
}
