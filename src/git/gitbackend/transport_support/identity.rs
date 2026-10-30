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
    /// `home` is the SSH home of the transport that will use the identities, where there is one: `~/` expands under
    /// it, so that `known_hosts` and `~/` identity paths share one home (design section 3; plan step 3.1).
    pub(crate) fn from_options(
        start: &Path,
        home: Option<&Path>,
        options: &crate::TransportOptions,
    ) -> ModelResult<Self> {
        let default = options
            .default_identity
            .as_deref()
            .map(|path| resolve_path(start, home, path))
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
                    resolve_path(start, home, &entry.private_key_path)?,
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

    // The CLI placement keeps identity paths opaque to core.
    // The endpoint owns path parsing, expansion and file checks.
    cfg_if::cfg_if! {
        if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
            pub(crate) fn from_options_cli(options: &crate::TransportOptions) -> ModelResult<Self> {
        let default = options
            .default_identity
            .as_deref()
            .map(opaque_path)
            .transpose()?;
        let mut remotes = BTreeMap::new();
        for entry in &options.remote_identities {
            if entry.remote.trim().is_empty()
                || !git2::Reference::is_valid_name(&format!("refs/remotes/{}/HEAD", entry.remote))
            {
                return Err(invalid("invalid remote name in SSH identity override"));
            }
            if remotes
                .insert(entry.remote.clone(), opaque_path(&entry.private_key_path)?)
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
        }
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

    /// The selection for `options` where `backend`'s operation runs: a host context's `~/` is its endpoint's SSH
    /// home, the one `known_hosts` is read from, and each selected file is checked here. The
    /// Cli placement keeps the paths opaque and has its endpoint check them.
    pub(crate) fn for_backend(
        backend: &super::super::Git2Backend,
        start: &Path,
        options: &crate::TransportOptions,
    ) -> ModelResult<Self> {
        cfg_if::cfg_if! {
            if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
                if backend.ssh.is_cli_context() {
                    let identities = Self::from_options_cli(options)?;
                    identities.validate_endpoint_files(backend)?;
                    return Ok(identities);
                }
            }
        }
        let identities = Self::from_options(start, backend.ssh.ssh_home().as_deref(), options)?;
        identities.validate_files()?;
        Ok(identities)
    }

    // CLI placement validates every invocation supplied identity before any
    // driver starts. This includes an overridden default, so a malformed or
    // unavailable path cannot be hidden by remote selection.
    cfg_if::cfg_if! {
        if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
            pub(crate) fn validate_endpoint_files(
                &self,
                backend: &super::super::Git2Backend,
            ) -> ModelResult<()> {
                for path in self.default.iter().chain(self.remotes.values()) {
                    backend.ssh.check_identity(&path.to_string_lossy())?;
                }
                Ok(())
            }
        }
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

/// `path` as an absolute path: `~/` expands under `home`, the SSH home of the transport that will use it where there
/// is one, else under the process's own home (the native route, which is libgit2's); anything else is relative to
/// `start`.
pub(crate) fn resolve_path(start: &Path, home: Option<&Path>, path: &str) -> ModelResult<PathBuf> {
    if path.trim().is_empty() || path.contains('\0') {
        return Err(invalid("SSH identity requires a nonempty file path"));
    }
    let path = if path == "~" || path.starts_with("~/") {
        home.map(Path::to_path_buf)
            .or_else(std::env::home_dir)
            .ok_or_else(|| invalid("cannot resolve home directory for SSH identity"))?
            .join(path.strip_prefix("~/").unwrap_or(""))
    } else if path.starts_with('~') {
        return Err(invalid("SSH identity supports ~/ paths, not ~user paths"));
    } else {
        start.join(path)
    };
    std::path::absolute(path).map_err(|_| invalid("cannot resolve SSH identity path"))
}

cfg_if::cfg_if! {
    if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
        fn opaque_path(path: &str) -> ModelResult<PathBuf> {
            if path.trim().is_empty() || path.contains('\0') {
                return Err(invalid("SSH identity requires a nonempty file path"));
            }
            Ok(PathBuf::from(path))
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
        /// Checks the selected identity at `path` where the backend's operation runs it: the file itself, or, for
        /// the Cli placement, by asking its endpoint.
        fn check_selected(backend: &super::super::Git2Backend, path: &Path) -> ModelResult<()> {
            if backend.ssh.is_cli_context() {
                return backend.ssh.check_identity(&path.to_string_lossy());
            }
            validate_file(path)
        }
    } else {
        /// Checks the selected identity at `path`: the file itself.
        fn check_selected(_backend: &super::super::Git2Backend, path: &Path) -> ModelResult<()> {
            validate_file(path)
        }
    }
}

/// The repository-local identity setting `value` as a path: expanded under the operation's SSH home, or, for the
/// Cli placement, left opaque to core.
fn configured_path(
    backend: &super::super::Git2Backend,
    repo: &git2::Repository,
    value: &str,
) -> ModelResult<PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
            if backend.ssh.is_cli_context() {
                return opaque_path(value);
            }
        }
    }
    resolve_path(
        repo.workdir().unwrap_or(repo.path()),
        backend.ssh.ssh_home().as_deref(),
        value,
    )
}

pub(crate) fn validate_file(path: &Path) -> ModelResult<()> {
    // Validate availability without reading or retaining any private-key bytes. The one regular-file open decides
    // in one step, so a path replaced by a FIFO or a device between a check and the open cannot block it.
    crate::git::regular_file::open(path)
        .map(drop)
        .map_err(|_| unavailable())
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
            if let Some(identity) = identity.as_ref() {
                check_selected(backend, &identity.path)?;
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
    let ssh = ssh_destination(is_cli(backend), url);
    // The route is refused here, before an identity is read, a connection is made or a file is written: the Cli
    // placement's endpoint serves SSH and HTTPS only, and the core never falls back to libgit2's native transport.
    cfg_if::cfg_if! {
        if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
            if backend.ssh.is_cli_context()
                && let Some(route) = local_only_route(url)
            {
                return Err(route.refusal());
            }
        }
    }
    if !ssh {
        if remote.is_some_and(|remote| backend.identities.remotes.contains_key(remote)) {
            return Err(invalid(
                "a per-remote SSH identity override names a non-SSH destination; use --identity PATH, which non-SSH destinations ignore, or no override for that remote",
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
                Ok(value) => Some(configured_path(backend, repo, &value)?),
                Err(error) if error.code() == git2::ErrorCode::NotFound => None,
                Err(error) => return Err(crate::git::git_error(error)),
            }
        }
        _ => None,
    };
    let identity = invocation.or_else(|| backend.identities.resolve(remote, configured.as_deref()));
    if let Some(identity) = &identity {
        check_selected(backend, &identity.path)?;
    }
    Ok(identity)
}

/// Whether identity selection treats `url` as an SSH destination, so that a per-remote override or an invocation
/// default applies to it. `cli` is whether the operation runs under the cli placement, where the route that carries
/// the request is `transport_binding::bound_route`: identity selection then reads the same classification, and no
/// spelling is an SSH route to one and a non-SSH destination to the other (State round-2 P2-1). Elsewhere the SCP test
/// stands, as before.
fn ssh_destination(cli: bool, url: &str) -> bool {
    let ssh_scheme = url.split_once("://").is_some_and(|(scheme, _)| {
        ["ssh", "git+ssh", "ssh+git"]
            .iter()
            .any(|v| scheme.eq_ignore_ascii_case(v))
    });
    cfg_if::cfg_if! {
        if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
            if cli {
                return ssh_scheme
                    || super::super::transport_binding::bound_route(url)
                        == super::super::transport_binding::Bound::Ssh;
            }
        } else {
            let _ = cli;
        }
    }
    ssh_scheme || scp_like(url)
}

cfg_if::cfg_if! {
    if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
        fn is_cli(backend: &super::super::Git2Backend) -> bool {
            backend.ssh.is_cli_context()
        }
    } else {
        fn is_cli(_backend: &super::super::Git2Backend) -> bool {
            false
        }
    }
}

/// Whether `url` is git's SCP-like SSH form (`host:path`): no scheme, not an absolute path, and a host of more than one
/// character before the first `:` (so a Windows drive letter is a path) that holds no `/`.
fn scp_like(url: &str) -> bool {
    !url.contains("://")
        && !Path::new(url).is_absolute()
        && url
            .split_once(':')
            .is_some_and(|(host, _)| host.len() > 1 && !host.contains('/'))
}

cfg_if::cfg_if! {
    if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
        use super::super::transport_binding::{Bound, bound_route};
        /// A route the Cli placement does not serve (plan step 1.7; `GwzSplitCommandChannel.md` section 4.6): the
        /// client's endpoint carries SSH and HTTPS only, so a remote that libgit2 would serve with its own transport
        /// stays on the machine that holds it and is refused, by name, before anything runs.
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub(crate) enum LocalOnlyRoute {
            /// A `file://` URL.
            File,
            /// A bare local path: no scheme, and not an SCP-like spelling the binding reads as SSH.
            LocalPath,
            /// A plain `http://` URL.
            Http,
            /// A `git://` URL.
            Git,
            /// The clone family's anonymous local peer (`fetch_anonymous`, `push_anonymous`).
            CloneFamily,
            /// Any other scheme, as the URL wrote it (kept to the characters a scheme has).
            Other(String),
        }
        impl LocalOnlyRoute {
            /// The words that name the route in a refusal.
            pub(crate) fn name(&self) -> String {
                match self {
                    Self::File => "file://".to_owned(),
                    Self::LocalPath => "local path".to_owned(),
                    Self::Http => "http://".to_owned(),
                    Self::Git => "git://".to_owned(),
                    Self::CloneFamily => "clone-family local peer".to_owned(),
                    Self::Other(scheme) => format!("{scheme}://"),
                }
            }
            /// The refusal for the route. It carries no part of the URL, which may hold a user name or a password.
            pub(crate) fn refusal(&self) -> ModelError {
                ModelError::new(
                    ErrorCode::UnsupportedOperation,
                    format!(
                        "the cli placement does not serve the {} route: its endpoint carries ssh://, git+ssh://, \
                         ssh+git://, SCP-like and https:// remotes only, and the core never falls back to libgit2's \
                         native transport; nothing was run",
                        self.name()
                    ),
                )
            }
            /// The refusal for a route that the configuration of the repository the operation created, and not the
            /// configuration the route was judged on, rewrote the remote to.
            pub(crate) fn refusal_after_creation(&self) -> ModelError {
                let mut refusal = self.refusal();
                refusal.message.push_str(
                    " (the new repository's configuration rewrote the remote to this route after the route was judged)",
                );
                refusal
            }
        }
        /// The route `url` takes that the Cli placement does not serve, or `None` for an SSH or HTTPS remote. It is
        /// decided by `transport_binding::bound_route`, the classifier the binding binds on, so that nothing the check
        /// admits is left to libgit2's own transport (State P1-1).
        pub(crate) fn local_only_route(url: &str) -> Option<LocalOnlyRoute> {
            (bound_route(url) == Bound::Unbound).then(|| unbound_route(url))
        }
        /// The name of the route of a `url` the endpoint does not bind (`bound_route` is `Bound::Unbound`).
        pub(crate) fn unbound_route(url: &str) -> LocalOnlyRoute {
            let Some((scheme, _)) = url.split_once("://") else {
                return LocalOnlyRoute::LocalPath;
            };
            let is = |name: &str| scheme.eq_ignore_ascii_case(name);
            if is("file") {
                LocalOnlyRoute::File
            } else if is("http") {
                LocalOnlyRoute::Http
            } else if is("git") {
                LocalOnlyRoute::Git
            } else {
                let named = !scheme.is_empty()
                    && scheme.len() <= 16
                    && scheme
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
                LocalOnlyRoute::Other(if named {
                    scheme.to_ascii_lowercase()
                } else {
                    "unrecognised".to_owned()
                })
            }
        }
        /// The URL the route check and the identity judge for an operation that connects to the URL `connecting` makes
        /// (what libgit2 will bind, after the configuration's `url.<base>.insteadOf` and `pushInsteadOf` rules): under
        /// the Cli placement that URL, so no rewrite lands a refused route after the check; elsewhere `requested`, as
        /// before. `connecting` runs only under the Cli placement.
        pub(crate) fn judged_url<'a>(
            backend: &super::super::Git2Backend,
            requested: &'a str,
            connecting: impl FnOnce() -> ModelResult<String>,
        ) -> ModelResult<std::borrow::Cow<'a, str>> {
            if backend.ssh.is_cli_context() {
                return connecting().map(std::borrow::Cow::Owned);
            }
            Ok(std::borrow::Cow::Borrowed(requested))
        }
        /// Observes the remote a clone created in its new repository against the URL the route was judged on. The judge
        /// reads the default configuration, which cannot see the conditional includes (`includeIf "gitdir:"`) of the new
        /// repository, so a rule only that repository applies can still land a refused route: under the Cli placement the
        /// created remote is refused, by the name of the route it was rewritten to, when it is not served or is not the
        /// kind of route judged (libgit2 then removes the target it made). Elsewhere nothing is observed.
        pub(crate) fn refuse_unjudged_remote(
            backend: &super::super::Git2Backend,
            judged: &str,
            created: &git2::Remote<'_>,
            attempt: &super::super::transport_observations::TransportAttempt,
        ) -> Result<(), git2::Error> {
            if !backend.ssh.is_cli_context() {
                return Ok(());
            }
            let created = created.url().unwrap_or_default();
            if created == judged {
                return Ok(());
            }
            let refusal = match local_only_route(created) {
                Some(route) => route.refusal_after_creation(),
                None if bound_route(created) == bound_route(judged) => return Ok(()),
                None => ModelError::new(
                    ErrorCode::UnsupportedOperation,
                    "the cli placement does not serve this remote: the new repository's configuration rewrote it to a \
                     different route after the route was judged; nothing was run",
                ),
            };
            attempt.failed(refusal.clone());
            Err(git2::Error::new(
                git2::ErrorCode::GenericError,
                git2::ErrorClass::Net,
                &refusal.message,
            ))
        }
        /// Refuses the clone family's anonymous local peer when the backend's operation is the Cli placement, before the
        /// peer is looked at (plan step 1.7). Any other backend runs it.
        pub(crate) fn refuse_clone_family_under_cli(
            backend: &super::super::Git2Backend,
        ) -> ModelResult<()> {
            if backend.ssh.is_cli_context() {
                return Err(LocalOnlyRoute::CloneFamily.refusal());
            }
            Ok(())
        }
    } else {
        /// A build without the transport has no Cli placement: `with_transport` refuses an explicit `cli` request.
        pub(crate) fn judged_url<'a>(
            _backend: &super::super::Git2Backend,
            requested: &'a str,
            _connecting: impl FnOnce() -> ModelResult<String>,
        ) -> ModelResult<std::borrow::Cow<'a, str>> {
            Ok(std::borrow::Cow::Borrowed(requested))
        }
        pub(crate) fn refuse_clone_family_under_cli(
            _backend: &super::super::Git2Backend,
        ) -> ModelResult<()> {
            Ok(())
        }
        pub(crate) fn refuse_unjudged_remote(
            _backend: &super::super::Git2Backend,
            _judged: &str,
            _created: &git2::Remote<'_>,
            _attempt: &super::super::transport_observations::TransportAttempt,
        ) -> Result<(), git2::Error> {
            Ok(())
        }
    }
}

fn unavailable() -> ModelError {
    ModelError::new(
        ErrorCode::PermissionDenied,
        "selected SSH identity is unavailable or not a regular file; no agent fallback was attempted",
    )
}

pub(crate) fn configured_identity(path: &Path, remote: &str) -> ModelResult<Option<String>> {
    let repo = super::super::open_repo(path)?;
    crate::git::find_remote(&repo, remote)?;
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
    crate::git::find_remote(&repo, remote)?;
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

cfg_if::cfg_if! { if #[cfg(test)] {
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
                    url_scheme: None,
                    default_identity: Some("key".into()),
                    remote_identities: vec![],
                    ..Default::default()
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
            url_scheme: None,
            default_identity: Some("default=key".into()),
            remote_identities: vec![crate::RemoteSshIdentity {
                remote: "origin".into(),
                private_key_path: "specific=key".into(),
            }],
            ..Default::default()
        };
        let selection = Selection::from_options(temp.path(), None, &options).unwrap();
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
    fn tilde_expands_under_the_transports_home_and_otherwise_under_the_processs() {
        let root = std::env::temp_dir();
        let (start, home) = (root.join("start"), root.join("the").join("ssh").join("home"));
        let under_home = |path: &str| resolve_path(&start, Some(&home), path).unwrap();
        assert_eq!(under_home("~/.ssh/id"), home.join(".ssh/id"));
        assert_eq!(under_home("~"), home);
        // A path that is not a `~` one is relative to the start, whatever the home.
        assert_eq!(under_home("key"), start.join("key"));
        assert!(resolve_path(&start, Some(&home), "~user/key").is_err());
        // Without a transport home it is the process's, as the native route has it.
        if let Some(process) = std::env::home_dir() {
            assert_eq!(resolve_path(&start, None, "~/k").unwrap(), process.join("k"));
        }
        // A `Selection` built for a transport carries the transport's home.
        let options = crate::TransportOptions {
            default_identity: Some("~/id".into()),
            remote_identities: vec![crate::RemoteSshIdentity {
                remote: "origin".into(),
                private_key_path: "~/special".into(),
            }],
            ..Default::default()
        };
        let selection = Selection::from_options(&start, Some(&home), &options).unwrap();
        assert_eq!(
            selection.resolve(Some("origin"), None).unwrap().path,
            home.join("special")
        );
        assert_eq!(
            selection.resolve(Some("other"), None).unwrap().path,
            home.join("id")
        );
    }

    #[test]
    fn duplicate_empty_and_unknown_overrides_refuse() {
        let temp = tempfile::tempdir().unwrap();
        let entry = crate::RemoteSshIdentity {
            remote: "origin".into(),
            private_key_path: "key".into(),
        };
        let mut options = crate::TransportOptions {
            url_scheme: None,
            default_identity: None,
            remote_identities: vec![entry.clone(), entry.clone()],
            ..Default::default()
        };
        assert!(Selection::from_options(temp.path(), None, &options).is_err());
        options.remote_identities = vec![entry];
        options.default_identity = Some("".into());
        assert!(Selection::from_options(temp.path(), None, &options).is_err());
        options.default_identity = None;
        let selection = Selection::from_options(temp.path(), None, &options).unwrap();
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

// Plan step 1.7: which remotes the Cli placement serves, and the words that refuse the rest.
cfg_if::cfg_if! {
    if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
        mod route_tests {
            use super::{LocalOnlyRoute, local_only_route};

            /// The spellings the route table pins: every classifier of a URL must agree on each.
            const ROUTE_TABLE: &[&str] = &[
                "a@b:c", "b:c", r"a\b:c", r"CORP\user@host:r.git", r"[u\x@127.0.0.1:1]:r", r".\x:y",
                r"\x:y", r"C:\x", "C:/x", "c:x", "/a:b", "./x:y", "x", "", ":x", "@x:y", "[::1]:repo",
                "[2001:db8::1]:repo", "[host]:repo", "[u@127.0.0.1:2222]:r", "file:x", "ext::cmd", "foo::bar",
                "file:///x", "http://h/r", "git://h/r", "ssh://h/r", "ssh://", "https://h/r", "https://",
                "HTTP://h/r", "ftp://h/r", " ssh://h/r", "%66ile://x", r"\\server\share\r.git",
                "git@[::1]:repo", "ssh://[::1]/repo",
            ];

            #[test]
            fn the_routes_the_endpoint_carries_are_not_refused() {
                for url in [
                    "ssh://git@example.com/o/r.git",
                    "SSH://git@example.com/o/r.git",
                    "git+ssh://example.com/o/r.git",
                    "ssh+git://example.com/o/r.git",
                    "https://example.com/o/r.git",
                    "HTTPS://example.com/o/r.git",
                    "git@example.com:o/r.git",
                    "example.com:o/r.git",
                ] {
                    assert_eq!(local_only_route(url), None, "{url}");
                }
            }

            #[test]
            fn a_backslash_before_the_first_colon_is_a_local_path_not_scp() {
                // State P1-1: the binding reads these as local paths and leaves them to libgit2's native SSH, so the
                // check must refuse them: `Destination::parse` returns `Ok(None)` for each.
                for url in [
                    r"a\b:c",
                    r"CORP\user@host:r.git",
                    r"[u\x@127.0.0.1:1]:r",
                    r".\x:y",
                    r"\x:y",
                ] {
                    assert_eq!(local_only_route(url), Some(LocalOnlyRoute::LocalPath), "{url}");
                }
            }

            #[test]
            fn the_check_refuses_exactly_what_the_binding_does_not_bind() {
                use crate::git::endpoint::ssh_destination::Destination;
                for url in ROUTE_TABLE.iter().copied() {
                    let bound = super::super::super::transport_binding::is_https_remote(url)
                        || !matches!(Destination::parse(url), Ok(None));
                    assert_eq!(local_only_route(url).is_none(), bound, "{url:?}");
                }
            }

            #[test]
            fn an_ipv6_scp_spelling_is_bound_to_the_ssh_route_and_gwz_cannot_open_it() {
                // What is measured: the classifier sends these to the SSH route, as git, libgit2 and `Destination::parse`
                // read them, so the check does not name them "local path" (the old SCP test, a one-character host `[`,
                // did). What is not served: the core's session driver then refuses the destination at its parse
                // ("invalid SSH destination", `session/driver/opening.rs`), so no request reaches the endpoint and no
                // connection is made; `driver_only_tests::identity_split` records that outcome end to end.
                for url in ["[::1]:repo", "[2001:db8::1]:repo", "git@[::1]:repo"] {
                    assert_eq!(local_only_route(url), None, "{url}");
                }
            }

            #[test]
            fn identity_selection_reads_every_url_the_binding_binds_to_ssh_as_ssh_under_cli() {
                // State round-2 P2-1: the route and the identity rules must classify a URL once. Every spelling of the
                // route table that the binding sends to the SSH route is an SSH destination for identity selection, and
                // an HTTPS one is not.
                use super::{Bound, bound_route};
                for url in ROUTE_TABLE.iter().copied() {
                    match bound_route(url) {
                        Bound::Ssh => assert!(super::ssh_destination(true, url), "{url:?}"),
                        Bound::Https => assert!(!super::ssh_destination(true, url), "{url:?}"),
                        Bound::Unbound => {}
                    }
                }
            }

            #[test]
            fn each_local_only_route_has_its_own_name() {
                for (url, route, name) in [
                    ("file:///srv/r.git", LocalOnlyRoute::File, "file://"),
                    ("FILE:///srv/r.git", LocalOnlyRoute::File, "file://"),
                    ("/srv/r.git", LocalOnlyRoute::LocalPath, "local path"),
                    ("../r.git", LocalOnlyRoute::LocalPath, "local path"),
                    ("r.git", LocalOnlyRoute::LocalPath, "local path"),
                    (r"C:\srv\r.git", LocalOnlyRoute::LocalPath, "local path"),
                    ("http://example.com/r.git", LocalOnlyRoute::Http, "http://"),
                    ("git://example.com/r.git", LocalOnlyRoute::Git, "git://"),
                    ("ftp://example.com/r.git", LocalOnlyRoute::Other("ftp".into()), "ftp://"),
                    ("a b://x", LocalOnlyRoute::Other("unrecognised".into()), "unrecognised://"),
                ] {
                    let found = local_only_route(url);
                    assert_eq!(found.as_ref(), Some(&route), "{url}");
                    assert_eq!(route.name(), name, "{url}");
                }
            }

            #[test]
            fn a_refusal_names_the_route_and_repeats_no_part_of_the_url() {
                let route = local_only_route("http://user:hunter2@example.com/secret/r.git").unwrap();
                let message = route.refusal().message;
                assert!(message.contains("the http:// route"), "{message}");
                assert!(!message.contains("hunter2") && !message.contains("secret"), "{message}");
            }
        }
    }
}

} }
