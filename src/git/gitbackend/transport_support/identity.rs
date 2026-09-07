//! One resolver for invocation and repository-local SSH identity selection.

use crate::model::{ErrorCode, ModelError, ModelResult};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type ResolutionKey = (Option<PathBuf>, Option<String>, String);
type Resolutions = BTreeMap<ResolutionKey, Option<SelectedIdentity>>;

#[derive(Clone, Debug, Default)]
pub(crate) struct Selection {
    default: Option<PathBuf>,
    remotes: BTreeMap<String, PathBuf>,
    resolved: Option<Arc<Mutex<Resolutions>>>,
}

// Backend equality describes configured authority, not observations made while
// executing an operation. Clones within one operation share its frozen choices.
impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        self.default == other.default && self.remotes == other.remotes
    }
}
impl Eq for Selection {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Source {
    InvocationRemote,
    InvocationDefault,
    LocalConfiguration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectedIdentity {
    pub path: PathBuf,
    pub source: Source,
}

pub(crate) fn has_options(options: Option<&crate::TransportOptions>) -> bool {
    options.is_some_and(|options| {
        options.default_identity.is_some() || !options.remote_identities.is_empty()
    })
}

impl Selection {
    pub(crate) fn from_options(
        start: &Path,
        options: &crate::TransportOptions,
    ) -> ModelResult<Self> {
        let default = options
            .default_identity
            .as_deref()
            .map(|path| resolve_path(start, path))
            .transpose()?;
        let mut remotes = BTreeMap::new();
        for entry in &options.remote_identities {
            if entry.remote.trim().is_empty()
                || !git2::Reference::is_valid_name(&format!("refs/remotes/{}/HEAD", entry.remote))
            {
                return Err(invalid("invalid remote name in SSH identity override"));
            }
            if remotes
                .insert(
                    entry.remote.clone(),
                    resolve_path(start, &entry.private_key_path)?,
                )
                .is_some()
            {
                return Err(invalid("duplicate remote SSH identity override"));
            }
        }
        Ok(Self {
            default,
            remotes,
            resolved: Some(Arc::new(Mutex::new(BTreeMap::new()))),
        })
    }

    pub(crate) fn validate_remote_names(&self, names: &[String]) -> ModelResult<()> {
        if let Some(name) = self.remotes.keys().find(|name| !names.contains(name)) {
            return Err(invalid(format!(
                "SSH identity override names unused remote {name}"
            )));
        }
        Ok(())
    }

    pub(crate) fn validate_files(&self) -> ModelResult<()> {
        for path in self.default.iter().chain(self.remotes.values()) {
            validate_file(path)?;
        }
        Ok(())
    }

    pub(crate) fn resolve(
        &self,
        remote: Option<&str>,
        configured: Option<&Path>,
    ) -> Option<SelectedIdentity> {
        if let Some(path) = remote.and_then(|name| self.remotes.get(name)) {
            return Some(SelectedIdentity {
                path: path.clone(),
                source: Source::InvocationRemote,
            });
        }
        if let Some(path) = &self.default {
            return Some(SelectedIdentity {
                path: path.clone(),
                source: Source::InvocationDefault,
            });
        }
        configured.map(|path| SelectedIdentity {
            path: path.to_owned(),
            source: Source::LocalConfiguration,
        })
    }
}

pub(crate) fn resolve_path(start: &Path, path: &str) -> ModelResult<PathBuf> {
    if path.trim().is_empty() || path.contains('\0') {
        return Err(invalid("SSH identity requires a nonempty file path"));
    }
    let path = if path == "~" || path.starts_with("~/") {
        std::env::home_dir()
            .ok_or_else(|| invalid("cannot resolve home directory for SSH identity"))?
            .join(path.strip_prefix("~/").unwrap_or(""))
    } else if path.starts_with('~') {
        return Err(invalid("SSH identity supports ~/ paths, not ~user paths"));
    } else {
        start.join(path)
    };
    std::path::absolute(path).map_err(|_| invalid("cannot resolve SSH identity path"))
}

pub(crate) fn validate_file(path: &Path) -> ModelResult<()> {
    // Validate availability without reading or retaining any private-key bytes.
    if !std::fs::metadata(path)
        .map_err(|_| unavailable())?
        .is_file()
    {
        return Err(unavailable());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| unavailable())?;
    if !file.metadata().map_err(|_| unavailable())?.is_file() {
        return Err(unavailable());
    }
    Ok(())
}

pub(crate) fn for_remote(
    backend: &super::super::Git2Backend,
    repo: Option<&git2::Repository>,
    remote: Option<&str>,
    url: &str,
) -> ModelResult<Option<SelectedIdentity>> {
    let key = (
        repo.map(|repo| repo.path().to_path_buf()),
        remote.map(str::to_owned),
        url.to_owned(),
    );
    if let Some(cache) = &backend.identities.resolved {
        let mut cache = cache.lock().map_err(|_| {
            ModelError::new(
                ErrorCode::InternalError,
                "identity resolution lock poisoned",
            )
        })?;
        if let Some(identity) = cache.get(&key) {
            if let Some(identity) = identity {
                validate_file(&identity.path)?;
            }
            return Ok(identity.clone());
        }
        let identity = resolve_remote(backend, repo, remote, url)?;
        cache.insert(key, identity.clone());
        return Ok(identity);
    }
    resolve_remote(backend, repo, remote, url)
}

fn resolve_remote(
    backend: &super::super::Git2Backend,
    repo: Option<&git2::Repository>,
    remote: Option<&str>,
    url: &str,
) -> ModelResult<Option<SelectedIdentity>> {
    let scp = !url.contains("://")
        && !Path::new(url).is_absolute()
        && url
            .split_once(':')
            .is_some_and(|(host, _)| host.len() > 1 && !host.contains('/'));
    if !url.starts_with("ssh://") && !scp {
        if remote.is_some_and(|remote| backend.identities.remotes.contains_key(remote)) {
            return Err(invalid(
                "a per-remote SSH identity override names a non-SSH destination",
            ));
        }
        // An invocation-wide SSH default does not change HTTPS authentication.
        return Ok(None);
    }
    let invocation = backend.identities.resolve(remote, None);
    let configured = match (invocation.is_none(), repo, remote) {
        (true, Some(repo), Some(remote)) => {
            let config = repo.config().map_err(crate::git::git_error)?;
            let local = config
                .open_level(git2::ConfigLevel::Local)
                .map_err(crate::git::git_error)?;
            match local.get_string(&format!("remote.{remote}.gwzSshIdentity")) {
                Ok(value) => Some(resolve_path(repo.workdir().unwrap_or(repo.path()), &value)?),
                Err(error) if error.code() == git2::ErrorCode::NotFound => None,
                Err(error) => return Err(crate::git::git_error(error)),
            }
        }
        _ => None,
    };
    let identity = invocation.or_else(|| backend.identities.resolve(remote, configured.as_deref()));
    if let Some(identity) = &identity {
        validate_file(&identity.path)?;
    }
    Ok(identity)
}

fn unavailable() -> ModelError {
    ModelError::new(
        ErrorCode::PermissionDenied,
        "selected SSH identity is unavailable or not a regular file; no agent fallback was attempted",
    )
}

pub(crate) fn configured_identity(path: &Path, remote: &str) -> ModelResult<Option<String>> {
    let repo = super::super::open_repo(path)?;
    repo.find_remote(remote).map_err(crate::git::git_error)?;
    let local = repo
        .config()
        .map_err(crate::git::git_error)?
        .open_level(git2::ConfigLevel::Local)
        .map_err(crate::git::git_error)?;
    match local.get_string(&format!("remote.{remote}.gwzSshIdentity")) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(crate::git::git_error(error)),
    }
}

pub(crate) fn set_configured_identity(
    path: &Path,
    remote: &str,
    value: Option<&str>,
) -> ModelResult<()> {
    let repo = super::super::open_repo(path)?;
    repo.find_remote(remote).map_err(crate::git::git_error)?;
    let mut local = repo
        .config()
        .map_err(crate::git::git_error)?
        .open_level(git2::ConfigLevel::Local)
        .map_err(crate::git::git_error)?;
    let key = format!("remote.{remote}.gwzSshIdentity");
    match value {
        Some(value) => local.set_str(&key, value).map_err(crate::git::git_error),
        None => match local.remove(&key) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(()),
            Err(error) => Err(crate::git::git_error(error)),
        },
    }
}
fn invalid(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_identity_is_frozen_and_scopes_do_not_share_configured_keys() {
        use crate::git::{Git2Backend, GitBackend};
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let a = temp.path().join("key-a");
        let b = temp.path().join("key-b");
        std::fs::write(&a, "fixture a").unwrap();
        std::fs::write(&b, "fixture b").unwrap();
        repo.config()
            .unwrap()
            .set_str("remote.origin.gwzSshIdentity", a.to_str().unwrap())
            .unwrap();
        let base = Git2Backend::without_credential_helpers();
        let scoped_a = base.with_transport(temp.path(), None).unwrap();
        let backend_a = scoped_a.as_ref().unwrap_or(&base);
        let url = "ssh://git@example.invalid/repo";
        assert_eq!(
            for_remote(backend_a, Some(&repo), Some("origin"), url)
                .unwrap()
                .unwrap()
                .path,
            a
        );
        repo.config()
            .unwrap()
            .set_str("remote.origin.gwzSshIdentity", b.to_str().unwrap())
            .unwrap();
        assert_eq!(
            for_remote(backend_a, Some(&repo), Some("origin"), url)
                .unwrap()
                .unwrap()
                .path,
            a
        );
        let scoped_b = base.with_transport(temp.path(), None).unwrap();
        assert_eq!(
            for_remote(
                scoped_b.as_ref().unwrap_or(&base),
                Some(&repo),
                Some("origin"),
                url
            )
            .unwrap()
            .unwrap()
            .path,
            b
        );
    }

    #[test]
    fn invocation_identity_beats_invalid_local_config_and_leaves_https_helpers_unchanged() {
        use crate::git::{Git2Backend, GitBackend};
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        repo.config()
            .unwrap()
            .set_str("remote.origin.gwzSshIdentity", "")
            .unwrap();
        std::fs::write(
            temp.path().join("key"),
            "fixture: only local selection is tested",
        )
        .unwrap();
        let backend = Git2Backend::without_credential_helpers()
            .with_transport(
                temp.path(),
                Some(&crate::TransportOptions {
                    default_identity: Some("key".into()),
                    remote_identities: vec![],
                }),
            )
            .unwrap()
            .unwrap();
        let identity = for_remote(
            &backend,
            Some(&repo),
            Some("origin"),
            "ssh://git@example.invalid/repo",
        )
        .unwrap()
        .unwrap();
        assert_eq!(identity.source, Source::InvocationDefault);
        assert!(
            for_remote(
                &backend,
                Some(&repo),
                Some("origin"),
                "https://example.invalid/repo"
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn explicit_credentials_never_fall_back_after_rejection_or_other_challenge() {
        let identity = SelectedIdentity {
            path: PathBuf::from("unused-key"),
            source: Source::InvocationDefault,
        };
        let mut attempts = 0;
        super::super::explicit_credential(
            &identity,
            Some("operator"),
            git2::CredentialType::USERNAME,
            &mut attempts,
        )
        .unwrap();
        assert_eq!(attempts, 0);
        super::super::explicit_credential(
            &identity,
            Some("operator"),
            git2::CredentialType::SSH_KEY,
            &mut attempts,
        )
        .unwrap();
        assert_eq!(attempts, 1);
        assert!(
            super::super::explicit_credential(
                &identity,
                Some("operator"),
                git2::CredentialType::SSH_KEY,
                &mut attempts
            )
            .is_err()
        );
        for challenge in [
            git2::CredentialType::DEFAULT,
            git2::CredentialType::USER_PASS_PLAINTEXT,
        ] {
            assert!(
                super::super::explicit_credential(&identity, Some("operator"), challenge, &mut 0)
                    .is_err()
            );
        }
    }

    #[test]
    fn explicit_precedence_and_equals_in_paths_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let options = crate::TransportOptions {
            default_identity: Some("default=key".into()),
            remote_identities: vec![crate::RemoteSshIdentity {
                remote: "origin".into(),
                private_key_path: "specific=key".into(),
            }],
        };
        let selection = Selection::from_options(temp.path(), &options).unwrap();
        let local = temp.path().join("configured");
        assert_eq!(
            selection
                .resolve(Some("origin"), Some(&local))
                .unwrap()
                .path,
            temp.path().join("specific=key")
        );
        assert_eq!(
            selection
                .resolve(Some("upstream"), Some(&local))
                .unwrap()
                .path,
            temp.path().join("default=key")
        );
        assert_eq!(
            Selection::default()
                .resolve(Some("origin"), Some(&local))
                .unwrap()
                .path,
            local
        );
        assert!(Selection::default().resolve(Some("origin"), None).is_none());
    }

    #[test]
    fn duplicate_empty_and_unknown_overrides_refuse() {
        let temp = tempfile::tempdir().unwrap();
        let entry = crate::RemoteSshIdentity {
            remote: "origin".into(),
            private_key_path: "key".into(),
        };
        let mut options = crate::TransportOptions {
            default_identity: None,
            remote_identities: vec![entry.clone(), entry.clone()],
        };
        assert!(Selection::from_options(temp.path(), &options).is_err());
        options.remote_identities = vec![entry];
        options.default_identity = Some("".into());
        assert!(Selection::from_options(temp.path(), &options).is_err());
        options.default_identity = None;
        let selection = Selection::from_options(temp.path(), &options).unwrap();
        assert!(
            selection
                .validate_remote_names(&["upstream".into()])
                .is_err()
        );
        selection
            .validate_remote_names(&["origin".into(), "origin".into()])
            .unwrap();
    }
}

#[cfg(test)]
mod timeout_tests {
    #[test]
    fn native_backends_install_bounded_default_timeouts() {
        let _backend = crate::git::Git2Backend::without_credential_helpers();
        // Backend construction must finish the one-time startup configuration
        // before a worker can observe libgit2's process-wide timeout values.
        unsafe {
            assert!(git2::opts::get_server_connect_timeout_in_milliseconds().unwrap() > 0);
            assert!(git2::opts::get_server_timeout_in_milliseconds().unwrap() > 0);
        }
    }
}
