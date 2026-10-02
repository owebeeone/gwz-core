//! Executable discovery uses only the captured environment.
use super::*;

pub(super) fn resolve(config: &Config) -> Result<PathBuf, AuthError> {
    if config.executable.is_absolute() {
        return Ok(config.executable.clone());
    }
    if config.executable.as_os_str().is_empty() || config.executable.components().count() != 1 {
        return Err(AuthError::MissingExecutable);
    }
    let path = config.environment.iter().find(|(name, _)| name == OsStr::new("PATH"))
        .map(|(_, value)| value).ok_or(AuthError::MissingExecutable)?;
    for directory in std::env::split_paths(path) {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(&config.executable);
        if executable_file(&candidate) {
            return Ok(candidate);
        }
    }
    Err(AuthError::MissingExecutable)
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        fn executable_file(path: &std::path::Path) -> bool {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        }
    } else {
        fn executable_file(path: &std::path::Path) -> bool {
            path.is_file()
        }
    }
}
