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
//!
//! The same port also makes the family record private to the destination's
//! own root repository ([`ensure_managed_exclude`]): the managed block in
//! `.git/info/exclude` that every gwz mutation verb regenerates at a
//! workspace root (`/.gwz/`, `/gwz.conf/.tmp/`, every member path) is
//! written through the existing helper, so the pointer and marker install
//! writes next -- and the index and lock a root holds -- are ignored by
//! `git status` and never reach an index, whatever the source's exclude
//! held. Local and never committed, unlike `.gitignore`.

use std::path::{Path, PathBuf};

use gwz_repo_factory::origin_is_kept;
use gwz_workspace_install::{GitInstallReport, InstallPortError};

use crate::artifact::ManifestArtifact;
use crate::git::GitBackend;
use crate::workspace_ops::{ensure_workspace_exclude, read_lock_or_empty};

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

/// Regenerate gwz's managed block in `<workspace>/.git/info/exclude` from
/// `manifest` and the workspace's lock when it holds one
/// (`read_lock_or_empty`), through the one writer every other verb uses
/// (`workspace_ops::ensure_workspace_exclude`): idempotent, preserving
/// every non-gwz line, never committed.
///
/// Two callers, one rule. The destination install runs it in **every**
/// mode after the copy or the construction: a verbatim copy inherited the
/// source's file and the write is a no-op, while a constructed destination
/// (clean, bare; LCM3.1 / LCM2.3) inherits nothing and would otherwise show
/// its pointer and marker as untracked. Every create runs it at the family
/// root under the family lock, before the index is founded or rewritten,
/// so the record is ignored by enforcement rather than by the root's
/// history of other verbs.
///
/// The workspace root must already hold its repository (`.git` present):
/// the helper bootstraps `.git/info` but never a `.git`, so a root with no
/// repository is a configuration error rather than a stray directory. A
/// bare root (design §4.3) has a `.git` and no working tree; the block is
/// written inside it and hides nothing, harmlessly.
pub fn ensure_managed_exclude<B: GitBackend>(
    backend: &B,
    workspace: &Path,
    manifest: &ManifestArtifact,
) -> Result<(), InstallPortError> {
    if std::fs::symlink_metadata(workspace.join(".git")).is_err() {
        return Err(InstallPortError::Configuration {
            detail: format!(
                "{}: no root repository to hold the managed exclude block",
                workspace.display()
            ),
        });
    }
    let lock = read_lock_or_empty(workspace, &manifest.workspace.id).map_err(|error| {
        InstallPortError::Configuration {
            detail: format!("{}: lock: {}", workspace.display(), error.message),
        }
    })?;
    ensure_workspace_exclude(backend, workspace, manifest, &lock).map_err(|error| {
        InstallPortError::Configuration {
            detail: format!(
                "{}: managed exclude block: {}",
                workspace.display(),
                error.message
            ),
        }
    })
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

    /// The managed block goes into an existing root repository, once, with
    /// the operator's own lines kept; a directory with no repository is
    /// refused typed and gains no `.git`.
    #[test]
    fn the_managed_exclude_block_needs_a_root_repository_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::git::Git2Backend::without_credential_helpers();
        let manifest = ManifestArtifact {
            schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: crate::artifact::WorkspaceHeader {
                id: "ws_test".to_owned(),
            },
            members: vec![crate::artifact::ManifestMember {
                id: "mem_app".to_owned(),
                path: "app".to_owned(),
                source_kind: crate::artifact::ArtifactSourceKind::Git,
                source_id: "src_app".to_owned(),
                active: true,
                desired: None,
                remotes: Vec::new(),
            }],
        };
        let bare_directory = temp.path().join("no-repository");
        std::fs::create_dir(&bare_directory).unwrap();
        let error = ensure_managed_exclude(&backend, &bare_directory, &manifest).unwrap_err();
        assert!(
            matches!(error, InstallPortError::Configuration { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("no root repository"), "{error}");
        assert!(
            !bare_directory.join(".git").exists(),
            "a refusal creates no repository directory"
        );

        let workspace = temp.path().join("ws");
        git2::Repository::init(&workspace).unwrap();
        let exclude = workspace.join(".git/info/exclude");
        std::fs::write(&exclude, "# operator line\n/scratch/\n").unwrap();
        ensure_managed_exclude(&backend, &workspace, &manifest).unwrap();
        let once = std::fs::read_to_string(&exclude).unwrap();
        assert!(once.contains("# operator line\n/scratch/\n"), "{once}");
        assert!(once.contains("/.gwz/\n"), "{once}");
        assert!(once.contains("/app/\n"), "{once}");
        assert_eq!(
            once.matches("# BEGIN GWZ managed member repositories")
                .count(),
            1
        );
        ensure_managed_exclude(&backend, &workspace, &manifest).unwrap();
        assert_eq!(
            std::fs::read_to_string(&exclude).unwrap(),
            once,
            "a second run changes nothing"
        );
        let repository = git2::Repository::open(&workspace).unwrap();
        for private in [
            ".gwz/local-family.yml",
            ".gwz/local-family.lock",
            ".gwz/family-root",
            ".gwz/local-clone-allocation",
            "app/anything",
        ] {
            assert!(
                repository.is_path_ignored(Path::new(private)).unwrap(),
                "{private} is ignored"
            );
        }
        assert!(!repository.is_path_ignored(Path::new("README")).unwrap());
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
