//! System-level Git configuration the native read must cover.
//!
//! libgit2 reads one system file (`/etc/gitconfig`, or the Git for Windows
//! one it finds) while a Git on this machine may read another: Homebrew's
//! `/opt/homebrew/etc/gitconfig`, `/usr/local/etc/gitconfig`, Apple's
//! command-line-tools file, Git for Windows' `etc/gitconfig` and
//! `%PROGRAMDATA%/Git/config`. A key set in a file native does not read (a
//! hooks path, signing, a filter, an identity) would change the result with no
//! warning, so any such file that exists sends the operation to Git, and so
//! does a file that exists but cannot be read.

use std::path::{Path, PathBuf};

use super::environment::GitEnvironment;
use super::route::CliReason;

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// Every system file a Git for Windows or libgit2 could read.
        pub(super) fn candidates(env: &GitEnvironment) -> Vec<PathBuf> {
            let mut files = Vec::new();
            for variable in ["PROGRAMFILES", "PROGRAMW6432", "PROGRAMFILES(X86)"] {
                if let Some(root) = env.get(variable) {
                    let git = Path::new(root).join("Git");
                    files.push(git.join("etc").join("gitconfig"));
                    files.push(git.join("mingw64").join("etc").join("gitconfig"));
                    files.push(git.join("mingw32").join("etc").join("gitconfig"));
                }
            }
            if let Some(root) = env.get("PROGRAMDATA") {
                files.push(Path::new(root).join("Git").join("config"));
            }
            files
        }

        /// Files libgit2 reads: the one it finds, and `%PROGRAMDATA%/Git/config`.
        pub(super) fn covered(env: &GitEnvironment) -> Vec<PathBuf> {
            let mut files: Vec<PathBuf> = git2::Config::find_system().into_iter().collect();
            if let Some(root) = env.get("PROGRAMDATA") {
                files.push(Path::new(root).join("Git").join("config"));
            }
            files
        }
    } else {
        /// Every system file a Git on this machine could read.
        pub(super) fn candidates(_env: &GitEnvironment) -> Vec<PathBuf> {
            [
                "/etc/gitconfig",
                "/opt/homebrew/etc/gitconfig",
                "/usr/local/etc/gitconfig",
                "/Library/Developer/CommandLineTools/usr/share/git-core/gitconfig",
                "/Applications/Xcode.app/Contents/Developer/usr/share/git-core/gitconfig",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect()
        }

        /// Files libgit2 reads: the one it finds.
        pub(super) fn covered(_env: &GitEnvironment) -> Vec<PathBuf> {
            git2::Config::find_system().into_iter().collect()
        }
    }
}

fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn readable(path: &Path) -> bool {
    std::fs::File::open(path).is_ok()
}

fn exists(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

/// How `GIT_CONFIG_NOSYSTEM` reads; `None` when Git and libgit2 might not agree.
fn no_system(env: &GitEnvironment) -> Option<bool> {
    match env.get("GIT_CONFIG_NOSYSTEM") {
        None => Some(false),
        Some(value) => match value.to_str()?.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "" | "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        },
    }
}

/// Fail safe: native goes ahead only when every system configuration file
/// that exists is one libgit2 reads, and each can be read.
pub(super) fn check(
    env: &GitEnvironment,
    candidates: &[PathBuf],
    covered: &[PathBuf],
) -> Result<(), CliReason> {
    let Some(skip) = no_system(env) else {
        return Err(CliReason::SystemConfig("GIT_CONFIG_NOSYSTEM value"));
    };
    if skip {
        return Ok(());
    }
    if let Some(named) = env.get("GIT_CONFIG_SYSTEM") {
        // Git reads exactly this file, and so does libgit2 given the variable.
        let path = Path::new(named);
        return if !exists(path) || readable(path) {
            Ok(())
        } else {
            Err(CliReason::SystemConfig("GIT_CONFIG_SYSTEM file unreadable"))
        };
    }
    for path in candidates.iter().chain(covered) {
        if !exists(path) {
            continue;
        }
        if !readable(path) {
            return Err(CliReason::SystemConfig("system file unreadable"));
        }
        if !covered.iter().any(|known| same_file(known, path)) {
            return Err(CliReason::SystemConfig("system file native does not read"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, "[core]\n\thooksPath = /elsewhere\n").unwrap();
        path
    }

    #[test]
    fn an_existing_file_native_does_not_read_picks_the_cli() {
        let dir = tempfile::TempDir::new().unwrap();
        let read = file(&dir, "read");
        let other = file(&dir, "other");
        let env = GitEnvironment::none();
        assert_eq!(
            check(
                &env,
                std::slice::from_ref(&read),
                std::slice::from_ref(&read)
            ),
            Ok(())
        );
        assert!(matches!(
            check(
                &env,
                &[read.clone(), other.clone()],
                std::slice::from_ref(&read)
            ),
            Err(CliReason::SystemConfig(_))
        ));
        assert!(matches!(
            check(&env, &[other], &[]),
            Err(CliReason::SystemConfig(_))
        ));
        let missing = dir.path().join("missing");
        assert_eq!(check(&env, &[missing], &[read]), Ok(()));
    }

    #[test]
    fn no_system_and_the_named_system_file_are_honoured() {
        let dir = tempfile::TempDir::new().unwrap();
        let other = file(&dir, "other");
        for value in ["1", "true", "YES", "on"] {
            let env = GitEnvironment::from_vars([("GIT_CONFIG_NOSYSTEM", value)]);
            assert_eq!(
                check(&env, std::slice::from_ref(&other), &[]),
                Ok(()),
                "{value}"
            );
        }
        let named = GitEnvironment::from_vars([("GIT_CONFIG_SYSTEM", other.to_str().unwrap())]);
        assert_eq!(check(&named, std::slice::from_ref(&other), &[]), Ok(()));
        let odd = GitEnvironment::from_vars([("GIT_CONFIG_NOSYSTEM", "perhaps")]);
        assert!(matches!(
            check(&odd, &[], &[]),
            Err(CliReason::SystemConfig(_))
        ));
        let off = GitEnvironment::from_vars([("GIT_CONFIG_NOSYSTEM", "0")]);
        assert!(matches!(
            check(&off, &[other], &[]),
            Err(CliReason::SystemConfig(_))
        ));
    }

    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            #[test]
            fn an_unreadable_file_picks_the_cli() {
                use std::os::unix::fs::PermissionsExt;
                let dir = tempfile::TempDir::new().unwrap();
                let path = file(&dir, "locked");
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
                let env = GitEnvironment::none();
                let result = check(&env, std::slice::from_ref(&path), std::slice::from_ref(&path));
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                // Running as root can read it anyway; then the file is covered.
                if std::fs::File::open(&path).is_ok() && result.is_ok() {
                    return;
                }
                assert!(matches!(result, Err(CliReason::SystemConfig(_))));
            }
        }
    }
}
