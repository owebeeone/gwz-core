//! `InstallPorts::install_destination_git`: the destination's own Git
//! configuration (design §4.1, last exclusion row).
//!
//! A verbatim copy carries every `.git/config` as it sat, remotes included.
//! Install removes the URLs that would tie the destination to the host it
//! was copied on -- a `file:` or filesystem-path URL, a URL whose userinfo
//! carries a secret -- and keeps ordinary non-credential https/ssh origins.
//! The rule is [`gwz_repo_factory::origin_is_kept`], the same one the
//! factory applies when it *constructs* a clean or bare destination, so
//! install (copy) and construction (clean/bare) drop exactly the same URLs:
//! §4.1's "remove in install" row is installation's job in every mode, and
//! the factory's `set_origin` is only ever handed a URL this rule keeps.
//!
//! Only the URL keys go (`remote.<name>.url`, `remote.<name>.pushurl`); the
//! remote's fetch refspecs and its `refs/remotes/<name>/*` tracking refs are
//! copied history and stay.

use std::path::{Path, PathBuf};

use gwz_repo_factory::origin_is_kept;
use gwz_workspace_install::{GitInstallReport, InstallPortError};

/// One repository to install: a label for the report (`@root`, a member
/// id, a nested path) and its destination-relative path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestinationRepository {
    pub label: String,
    pub relative: PathBuf,
}

/// Remove the filesystem and credential-bearing remote URLs of every listed
/// repository under `destination`.
pub fn install_destination_git(
    destination: &Path,
    repositories: &[DestinationRepository],
) -> Result<GitInstallReport, InstallPortError> {
    let mut report = GitInstallReport::default();
    for repository in repositories {
        let path = destination.join(&repository.relative);
        let removed =
            strip_remote_urls(&path).map_err(|detail| InstallPortError::Configuration {
                detail: format!("{}: {detail}", path.display()),
            })?;
        report.removed_remotes.extend(
            removed
                .into_iter()
                .map(|remote| format!("{}: {remote}", repository.label)),
        );
    }
    Ok(report)
}

/// The remotes whose URL or push URL was removed from the repository at
/// `path`, in name order.
fn strip_remote_urls(path: &Path) -> Result<Vec<String>, String> {
    let repository = git2::Repository::open_ext(
        path,
        git2::RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&std::ffi::OsStr>(),
    )
    .map_err(|error| error.message().to_owned())?;
    let mut config = repository
        .config()
        .and_then(|config| config.open_level(git2::ConfigLevel::Local))
        .map_err(|error| error.message().to_owned())?;
    let mut dropped: Vec<(String, String)> = Vec::new();
    {
        let mut entries = config
            .entries(None)
            .map_err(|error| error.message().to_owned())?;
        while let Some(entry) = entries.next() {
            let entry = entry.map_err(|error| error.message().to_owned())?;
            let name = entry.name().map_err(|error| error.message().to_owned())?;
            let Some(remote) = remote_of_url_key(name) else {
                continue;
            };
            // A URL that is not UTF-8 names nothing an ordinary https/ssh
            // origin could; it goes with the filesystem URLs.
            let kept = entry.value().is_ok_and(origin_is_kept);
            if !kept {
                dropped.push((name.to_owned(), remote.to_owned()));
            }
        }
    }
    let mut removed: Vec<String> = Vec::new();
    for (key, remote) in dropped {
        config
            .remove_multivar(&key, ".*")
            .or_else(|_| config.remove(&key))
            .map_err(|error| format!("{key}: {}", error.message()))?;
        if !removed.contains(&remote) {
            removed.push(remote);
        }
    }
    removed.sort();
    Ok(removed)
}

/// `remote.<name>.url` or `remote.<name>.pushurl` -> `<name>`.
fn remote_of_url_key(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("remote.")?;
    rest.strip_suffix(".url")
        .or_else(|| rest.strip_suffix(".pushurl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_url(path: &Path, name: &str) -> Option<String> {
        git2::Repository::open(path)
            .unwrap()
            .find_remote(name)
            .ok()
            .and_then(|remote| remote.url().ok().map(str::to_owned))
    }

    #[test]
    fn filesystem_and_credential_urls_go_and_ordinary_origins_stay() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("repo");
        let repository = git2::Repository::init(&path).unwrap();
        repository
            .remote("origin", "https://example.invalid/org/repo.git")
            .unwrap();
        repository
            .remote("upstream", "git@example.invalid:org/repo.git")
            .unwrap();
        repository
            .remote("sibling", "/home/u/limbo/gwz-dev")
            .unwrap();
        repository
            .remote("mirror", "file:///srv/mirrors/repo.git")
            .unwrap();
        repository
            .remote("token", "https://user:secret@example.invalid/org/repo.git")
            .unwrap();
        {
            let mut config = repository.config().unwrap();
            config
                .set_str("remote.origin.pushurl", "../peer/repo")
                .unwrap();
        }
        drop(repository);

        let report = install_destination_git(
            temp.path(),
            &[DestinationRepository {
                label: "@root".to_owned(),
                relative: PathBuf::from("repo"),
            }],
        )
        .unwrap();
        assert_eq!(
            report.removed_remotes,
            vec![
                "@root: mirror".to_owned(),
                "@root: origin".to_owned(),
                "@root: sibling".to_owned(),
                "@root: token".to_owned(),
            ]
        );
        assert_eq!(
            remote_url(&path, "origin").as_deref(),
            Some("https://example.invalid/org/repo.git"),
            "an ordinary https origin keeps its fetch URL"
        );
        assert_eq!(
            git2::Repository::open(&path)
                .unwrap()
                .find_remote("origin")
                .unwrap()
                .pushurl()
                .unwrap(),
            None,
            "its filesystem push URL is gone"
        );
        assert_eq!(
            remote_url(&path, "upstream").as_deref(),
            Some("git@example.invalid:org/repo.git")
        );
        for gone in ["sibling", "mirror", "token"] {
            assert_eq!(remote_url(&path, gone), None, "{gone} has no URL left");
        }
        let config = git2::Repository::open(&path).unwrap().config().unwrap();
        assert!(
            config.get_string("remote.sibling.fetch").is_ok(),
            "the remote's fetch refspec is copied configuration and stays"
        );
        // Repeating it is a no-op that reports nothing.
        let again = install_destination_git(
            temp.path(),
            &[DestinationRepository {
                label: "@root".to_owned(),
                relative: PathBuf::from("repo"),
            }],
        )
        .unwrap();
        assert!(again.removed_remotes.is_empty());
    }

    #[test]
    fn a_repository_that_cannot_be_opened_is_a_configuration_error() {
        let temp = tempfile::tempdir().unwrap();
        let error = install_destination_git(
            temp.path(),
            &[DestinationRepository {
                label: "mem_x".to_owned(),
                relative: PathBuf::from("absent"),
            }],
        )
        .unwrap_err();
        assert!(
            matches!(error, InstallPortError::Configuration { .. }),
            "{error}"
        );
    }
}
