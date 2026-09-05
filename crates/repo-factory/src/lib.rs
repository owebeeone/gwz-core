//! `gwz-repo-factory`: independent bare/clean repository construction
//! (lane B).
//!
//! [`construct`] builds a destination's repositories from one captured
//! freeze vector (destination, frozen commit, branch and origin per
//! repository) through the [`RepoBuildPort`] it owns: clean checkouts with
//! optional branch creation (design §4.2) and bare hubs (design §4.3),
//! following gwz-dev `dev-docs/GwzLocalCloneDesign.md` revision 9 and
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §8.
//!
//! What this crate promises:
//!
//! - **One freeze, every repository.** The captured vector carries the
//!   frozen commit of every member *including the root*; construction uses
//!   that commit and nothing else. It resolves no name, reads no
//!   configuration and consults no source ref beyond the two preflight
//!   questions below.
//! - **Everything is checked before anything is created.** The requested
//!   `-b <branch>` must not already exist in any source, and every frozen
//!   commit must already be present in its source object store. Both
//!   refusals aggregate and happen before the first `init_repository`, so
//!   a refused construction has allocated nothing (design §4.2, boundaries
//!   §3).
//! - **No implicit network, no hidden commit, no borrowed object store.**
//!   A missing object is [`FactoryError::ObjectMissing`], never a fetch.
//! - **No family metadata.** The destination's `gwz.conf/` and `.gwz/` are
//!   the installer's to write; [`FactoryReport::follow_ups`] says what it
//!   still owes before dest-complete may be true.
//! - **Nothing of the source worktree is inherited.** A clean destination
//!   is checked out from the transferred objects, so source dirt, a
//!   member `MERGE_*` and `target/` cannot arrive by construction. That
//!   the source *was* quiescent and clean at the freeze is the caller's
//!   check, not this crate's: it observes no worktree.
//! - **Best effort, no durability machinery.** A failed construction is
//!   [`FactoryError::Build`] naming the member, the typed cause and every
//!   repository already built. Nothing is rolled back or cleaned up — the
//!   port has no removal operation — and the destination is retained for
//!   inspection.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use gwz_repo_contract::{HeadState, ObjectFormat, ObjectId, RepoKey};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactoryMode {
    /// Clean worktree checkout at the captured commit; `branch` creates and
    /// attaches that branch in every repository.
    Clean { branch: Option<String> },
    /// Bare repository (`core.bare=true`), no worktree, no checkout.
    Bare,
}

impl FactoryMode {
    /// The branch `-b` asks for, when the mode has one. `--bare` never
    /// creates a branch (design §4.3).
    pub fn requested_branch(&self) -> Option<&str> {
        match self {
            Self::Clean { branch } => branch.as_deref(),
            Self::Bare => None,
        }
    }

    pub fn is_bare(&self) -> bool {
        matches!(self, Self::Bare)
    }
}

/// One repository of the freeze vector.
///
/// `source` and `destination` follow the repository-path convention of
/// `gwz_repo_contract::RepositoryInfo::path`: the worktree root of an
/// ordinary repository. `destination` is always the repository's place in
/// the destination *workspace layout* (the destination root, or the
/// destination root joined with `member.path`) — bare mode keeps that same
/// layout and builds the bare repository in its `.git` (design §4.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedRepo {
    pub key: RepoKey,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub object_format: ObjectFormat,
    /// The frozen commit every destination uses (the captured recorded
    /// HEAD by default — one choice for every member including root).
    pub head: ObjectId,
    /// The source's attached branch, when any. A plain branch name, not a
    /// full ref name.
    pub branch: Option<String>,
    /// The source's `origin` remote URL, as captured. A filesystem or
    /// credential-bearing URL is dropped; an ordinary https/ssh origin is
    /// kept in the destination (design §4.2, §4.1's exclusion table). See
    /// [`origin_is_kept`].
    pub origin: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactoryRequest {
    pub mode: FactoryMode,
    pub vector: Vec<CapturedRepo>,
}

/// What one destination repository ended up holding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltRepo {
    pub key: RepoKey,
    /// The repository's place in the destination workspace layout, exactly
    /// as the request gave it.
    pub destination: PathBuf,
    /// The Git directory that was created: `destination/.git` in both
    /// modes. In bare mode this *is* the repository.
    pub git_dir: PathBuf,
    pub bare: bool,
    /// Where the destination's HEAD points, at the frozen commit.
    pub head: HeadState,
    /// Branches created at the frozen commit, in creation order.
    pub branches: Vec<String>,
    /// The source `origin` URL kept in the destination, when one was kept.
    pub origin: Option<String>,
    /// The source `origin` URL that was dropped as a filesystem or
    /// credential URL. Reported, never silently discarded.
    pub dropped_origin: Option<String>,
}

/// What the installer still owes the destination after construction, in
/// order. Construction writes no family metadata itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallerFollowUp {
    /// Recapture the destination's desired branches after `-b <branch>`
    /// (design §4.2).
    RecaptureDesiredBranches { branch: String },
    /// Recapture `gwz.conf/gwz.lock.yml` to dest HEAD. Dest-complete
    /// includes lock/HEAD agreement; if the recapture cannot be done the
    /// destination is incomplete and the row must not flip `ready`.
    RecaptureLock,
    /// Write `.gwz/family-root` (registering root plus family id) so
    /// workspace discovery works. For a bare hub this is half of what
    /// makes the hub itself a gwz workspace (design §4.3).
    WriteFamilyRoot,
    /// Write `gwz.conf/gwz.yml` last; the destination is not discoverable
    /// until then, and the manifest is the last thing before `ready`.
    WriteManifestLast,
}

/// Everything that was built, plus what the installer must still do.
///
/// A report only exists when construction completed: every repository of
/// the request was built at its frozen commit ([`FactoryReport::is_complete`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FactoryReport {
    /// Every repository built, in construction order (root first).
    pub built: Vec<RepoKey>,
    /// The detail of each, in the same order as `built`.
    pub repositories: Vec<BuiltRepo>,
    /// Ordered installer obligations; the manifest is always last.
    pub follow_ups: Vec<InstallerFollowUp>,
}

impl FactoryReport {
    /// Whether every repository of `request` was built. The installer must
    /// not treat a destination as complete otherwise.
    pub fn is_complete(&self, request: &FactoryRequest) -> bool {
        let built: BTreeSet<&RepoKey> = self.built.iter().collect();
        self.built.len() == built.len()
            && request.vector.len() == built.len()
            && request.vector.iter().all(|repo| built.contains(&repo.key))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    Repository { path: PathBuf, detail: String },
    Failed { detail: String },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repository { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Failed { detail } => f.write_str(detail),
        }
    }
}

impl std::error::Error for BuildError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactoryError {
    InvalidRequest {
        detail: String,
    },
    /// `-b` names a branch that already exists in a source; refused before
    /// any allocation (aggregating). Reported ahead of
    /// [`ObjectMissing`](Self::ObjectMissing) when both are found.
    BranchExists {
        conflicts: Vec<(RepoKey, String)>,
    },
    /// A frozen commit is not present in its source object store; refused
    /// before any allocation (aggregating). Objects are the caller's
    /// problem: construction never fetches.
    ObjectMissing {
        missing: Vec<(RepoKey, ObjectId)>,
    },
    RefMissing {
        key: RepoKey,
        name: String,
    },
    Build {
        key: RepoKey,
        error: BuildError,
        /// Repositories fully built before the failure; retained.
        built: Vec<RepoKey>,
    },
    /// Retained from the LCM1.0c checkpoint for a composition root whose
    /// builder port is not implemented yet. [`construct`] no longer
    /// returns it.
    Unimplemented,
}

impl fmt::Display for FactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest { detail } => write!(f, "invalid factory request: {detail}"),
            Self::BranchExists { conflicts } => write!(f, "branch already exists: {conflicts:?}"),
            Self::ObjectMissing { missing } => write!(f, "frozen commit is missing: {missing:?}"),
            Self::RefMissing { key, name } => write!(f, "{key}: ref {name} is missing"),
            Self::Build { key, error, .. } => write!(f, "{key}: {error}"),
            Self::Unimplemented => f.write_str("gwz-repo-factory: construct is not implemented"),
        }
    }
}

impl std::error::Error for FactoryError {}

/// The repository-builder port. Core implements it over the Git backend.
///
/// Every method takes a repository path following the convention of
/// `gwz_repo_contract::RepositoryInfo::path`: the worktree root of an
/// ordinary repository, the Git directory of a bare one. No method ever
/// removes anything.
///
/// Call order per invocation of [`construct`]: for every repository of the
/// vector, `object_exists` and (only with `-b`) `ref_exists` against the
/// **source**, all of them before any other call. Then, for each
/// repository in turn, `init_repository`, `transfer_objects`, exactly one
/// of `set_head` / `set_detached_head`, and `set_origin` when an origin
/// survives. Nothing is called after the first error.
pub trait RepoBuildPort {
    /// Does `name` (a full ref name) resolve in `repository`?
    fn ref_exists(&mut self, repository: &Path, name: &str) -> Result<bool, BuildError>;

    /// Is `oid` present in `repository`'s own object store? Answers from
    /// what is local; never fetches.
    fn object_exists(&mut self, repository: &Path, oid: &ObjectId) -> Result<bool, BuildError>;

    /// Create an empty repository with its own object store. `bare` sets
    /// `core.bare=true` and gives it no worktree.
    fn init_repository(
        &mut self,
        destination: &Path,
        bare: bool,
        object_format: ObjectFormat,
    ) -> Result<(), BuildError>;

    /// Copy objects reachable from `refspecs` from `source` into
    /// `destination` without borrowing the source store. Each refspec is a
    /// source-side tip; [`construct`] passes the frozen commit id.
    fn transfer_objects(
        &mut self,
        source: &Path,
        destination: &Path,
        refspecs: &[String],
    ) -> Result<(), BuildError>;

    /// Point `branch` at `target`, creating it, and attach HEAD to it;
    /// `checkout` materializes the worktree for clean mode.
    fn set_head(
        &mut self,
        repository: &Path,
        branch: &str,
        target: &ObjectId,
        checkout: bool,
    ) -> Result<(), BuildError>;

    /// Detach HEAD at `target` (the source was detached at the freeze);
    /// `checkout` materializes the worktree for clean mode.
    fn set_detached_head(
        &mut self,
        repository: &Path,
        target: &ObjectId,
        checkout: bool,
    ) -> Result<(), BuildError>;

    /// Record `url` as the destination's `origin` remote. Only ever called
    /// with a URL that [`origin_is_kept`] admits.
    fn set_origin(&mut self, repository: &Path, url: &str) -> Result<(), BuildError>;
}

/// Whether a captured `origin` URL survives into a constructed
/// destination.
///
/// Design §4.2 keeps the source's `origin` URLs when they are not `file:`;
/// §4.1's install removes filesystem and credential URLs from copied
/// remotes and keeps "ordinary non-credential https/ssh origin URLs". This
/// is that rule: drop a `file:` URL, a local path (absolute, `./`, `../`,
/// `~`, a Windows drive, or a bare relative name), and any URL whose
/// userinfo carries a secret. An `scp`-style `git@host:path` remote and a
/// `user@host` URL without a password are ordinary remotes and are kept.
pub fn origin_is_kept(url: &str) -> bool {
    !(is_filesystem_url(url) || carries_credentials(url))
}

fn is_filesystem_url(url: &str) -> bool {
    if url
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
    {
        return true;
    }
    if url.starts_with('/')
        || url.starts_with('\\')
        || url.starts_with('~')
        || url.starts_with("./")
        || url.starts_with("../")
    {
        return true;
    }
    let mut chars = url.chars();
    if let (Some(drive), Some(':'), Some(separator)) = (chars.next(), chars.next(), chars.next())
        && drive.is_ascii_alphabetic()
        && (separator == '\\' || separator == '/')
    {
        return true;
    }
    // Neither a scheme nor an `scp`-style `host:path`: an ordinary
    // relative path, or nothing at all.
    !url.contains("://") && !url.contains(':')
}

fn carries_credentials(url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    authority
        .rsplit_once('@')
        .is_some_and(|(userinfo, _)| userinfo.contains(':'))
}

/// Build every repository of the captured freeze vector.
///
/// Returns a complete [`FactoryReport`], or a typed refusal. A refusal
/// before the build loop ([`FactoryError::InvalidRequest`],
/// [`FactoryError::BranchExists`], [`FactoryError::ObjectMissing`]) has
/// created nothing; [`FactoryError::Build`] names the member that failed
/// and every repository already built, all of which are retained.
pub fn construct(
    request: &FactoryRequest,
    port: &mut dyn RepoBuildPort,
) -> Result<FactoryReport, FactoryError> {
    let order = validate(request)?;
    preflight(request, &order, port)?;
    build(request, &order, port)
}

/// Reject a request that cannot describe a destination, and order the
/// vector root-first so member directories are created inside the
/// destination root.
fn validate(request: &FactoryRequest) -> Result<Vec<&CapturedRepo>, FactoryError> {
    if request.vector.is_empty() {
        return Err(invalid("a factory request needs at least one repository"));
    }
    if let Some(branch) = request.mode.requested_branch() {
        validate_branch_name(branch, "the requested branch")?;
    }
    let mut keys = BTreeSet::new();
    let mut destinations = BTreeSet::new();
    for repo in &request.vector {
        if !keys.insert(&repo.key) {
            return Err(invalid(format!(
                "{} appears twice in the captured vector",
                repo.key
            )));
        }
        if !destinations.insert(&repo.destination) {
            return Err(invalid(format!(
                "{} and another repository share the destination {}",
                repo.key,
                repo.destination.display()
            )));
        }
        if repo.head.format() != repo.object_format {
            return Err(invalid(format!(
                "{}: the frozen commit is {:?} but the repository is {:?}",
                repo.key,
                repo.head.format(),
                repo.object_format
            )));
        }
        if let Some(branch) = &repo.branch {
            validate_branch_name(branch, &format!("{}'s captured branch", repo.key))?;
        }
    }
    if !keys.contains(&RepoKey::Root) {
        return Err(invalid(
            "the captured vector must include the workspace root",
        ));
    }
    let (root, members): (Vec<&CapturedRepo>, Vec<&CapturedRepo>) = request
        .vector
        .iter()
        .partition(|repo| repo.key == RepoKey::Root);
    Ok(root.into_iter().chain(members).collect())
}

fn validate_branch_name(name: &str, what: &str) -> Result<(), FactoryError> {
    if name.trim().is_empty() {
        return Err(invalid(format!("{what} is empty")));
    }
    if name.starts_with("refs/") {
        return Err(invalid(format!(
            "{what} is a full ref name ({name}); a plain branch name is required"
        )));
    }
    Ok(())
}

/// Every source-side question, asked before anything is allocated. Both
/// refusals aggregate across the whole vector.
fn preflight(
    request: &FactoryRequest,
    order: &[&CapturedRepo],
    port: &mut dyn RepoBuildPort,
) -> Result<(), FactoryError> {
    let requested = request.mode.requested_branch();
    let mut missing = Vec::new();
    let mut conflicts = Vec::new();
    for repo in order {
        let present = port
            .object_exists(&repo.source, &repo.head)
            .map_err(|error| preflight_failure(repo, error))?;
        if !present {
            missing.push((repo.key.clone(), repo.head.clone()));
        }
        if let Some(branch) = requested {
            let name = format!("refs/heads/{branch}");
            let exists = port
                .ref_exists(&repo.source, &name)
                .map_err(|error| preflight_failure(repo, error))?;
            if exists {
                conflicts.push((repo.key.clone(), branch.to_owned()));
            }
        }
    }
    if !conflicts.is_empty() {
        return Err(FactoryError::BranchExists { conflicts });
    }
    if !missing.is_empty() {
        return Err(FactoryError::ObjectMissing { missing });
    }
    Ok(())
}

fn build(
    request: &FactoryRequest,
    order: &[&CapturedRepo],
    port: &mut dyn RepoBuildPort,
) -> Result<FactoryReport, FactoryError> {
    let bare = request.mode.is_bare();
    let checkout = !bare;
    let requested = request.mode.requested_branch();
    let mut report = FactoryReport::default();
    for repo in order {
        let git_dir = repo.destination.join(".git");
        let path = if bare { &git_dir } else { &repo.destination };
        let failure = |error: BuildError| FactoryError::Build {
            key: repo.key.clone(),
            error,
            built: report.built.clone(),
        };

        port.init_repository(path, bare, repo.object_format)
            .map_err(failure)?;
        port.transfer_objects(&repo.source, path, &[repo.head.to_hex()])
            .map_err(failure)?;

        // `-b` wins: it is the branch the destination is created on. With
        // no `-b`, the destination takes the frozen branch; a source that
        // was detached at the freeze stays detached.
        let attach = requested.or(repo.branch.as_deref());
        let (head, branches) = match attach {
            Some(branch) => {
                port.set_head(path, branch, &repo.head, checkout)
                    .map_err(failure)?;
                (
                    HeadState::Attached {
                        branch: branch.to_owned(),
                        target: repo.head.clone(),
                    },
                    vec![branch.to_owned()],
                )
            }
            None => {
                port.set_detached_head(path, &repo.head, checkout)
                    .map_err(failure)?;
                (
                    HeadState::Detached {
                        target: repo.head.clone(),
                    },
                    Vec::new(),
                )
            }
        };

        let (origin, dropped_origin) = match repo.origin.as_deref() {
            Some(url) if origin_is_kept(url) => {
                port.set_origin(path, url).map_err(failure)?;
                (Some(url.to_owned()), None)
            }
            Some(url) => (None, Some(url.to_owned())),
            None => (None, None),
        };

        report.built.push(repo.key.clone());
        report.repositories.push(BuiltRepo {
            key: repo.key.clone(),
            destination: repo.destination.clone(),
            git_dir,
            bare,
            head,
            branches,
            origin,
            dropped_origin,
        });
    }
    report.follow_ups = follow_ups(&request.mode);
    debug_assert!(report.is_complete(request));
    Ok(report)
}

fn follow_ups(mode: &FactoryMode) -> Vec<InstallerFollowUp> {
    let mut list = Vec::new();
    if let Some(branch) = mode.requested_branch() {
        list.push(InstallerFollowUp::RecaptureDesiredBranches {
            branch: branch.to_owned(),
        });
    }
    list.push(InstallerFollowUp::RecaptureLock);
    list.push(InstallerFollowUp::WriteFamilyRoot);
    list.push(InstallerFollowUp::WriteManifestLast);
    list
}

fn preflight_failure(repo: &CapturedRepo, error: BuildError) -> FactoryError {
    FactoryError::Build {
        key: repo.key.clone(),
        error,
        built: Vec::new(),
    }
}

fn invalid(detail: impl Into<String>) -> FactoryError {
    FactoryError::InvalidRequest {
        detail: detail.into(),
    }
}
