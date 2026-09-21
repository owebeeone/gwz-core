//! Root publication is a dependency barrier, not a cross-server transaction.
//! It freezes the exact root source object, so a branch move cannot swap its
//! lock, and proves every lock dependency available through the read URL of
//! its committed fetch remote before the root transfer.
//!
//! The proof comes from this operation (gwz-dev
//! `dev-docs/GwzUrlSchemePushPlan.md` §3.5 rule 1, D8 and D9): a destination's
//! advertisement, read at most once and kept for later questions, or an
//! accepted push of the locked commit or of a descendant. A forced or deleting
//! transfer anywhere in the operation voids both, and every dependency is then
//! read again after the member transfers.
//!
//! Push plans each round of reads before any read runs, so a destination is
//! read once whichever targets need it, and runs the round under the policy of
//! its transfers (step 3.4). Tag publication reads in turn, because its backend
//! need not be `Sync`.
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::{Path, PathBuf};

use crate::artifact::{self, ArtifactSourceKind, LockArtifact, ManifestArtifact};
use crate::git::{GitBackend, GitPreparedPush, GitRemoteRef, git_host};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::par_map_per_host;
use crate::workspace::WORKSPACE_MANIFEST;

use super::publication_url::{DependencyMember, ReadUrlRule, select_read_url};
use super::url_scheme_state::{requested_url_scheme, resolve_push_url_scheme};

/// The advertisements this operation has read, each kept for one destination:
/// its identity repository (none for a dependency that is not materialized),
/// remote name and read URL. The identity owner is part of the key: the same
/// URL reached with a different repository's SSH configuration still needs its
/// own authentication check.
#[derive(Default)]
pub(super) struct ReadPreflight {
    kept: BTreeMap<(Option<PathBuf>, String, String), Vec<GitRemoteRef>>,
    /// Set when the operation makes a forced or deleting transfer.
    voided: bool,
}

impl ReadPreflight {
    /// A destination's advertisement, read only when this operation has not
    /// kept one for it yet.
    pub(super) fn read<B: GitBackend>(
        &mut self,
        backend: &B,
        path: &Path,
        url: &str,
        remote: &str,
        identity_repo: Option<&Path>,
    ) -> ModelResult<&[GitRemoteRef]> {
        let key = (
            identity_repo.map(Path::to_path_buf),
            remote.to_owned(),
            url.to_owned(),
        );
        let advertised = match self.kept.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                entry.insert(backend.ls_remote_url(path, url, remote, identity_repo)?)
            }
        };
        Ok(advertised.as_slice())
    }

    fn kept(
        &self,
        identity_repo: Option<&Path>,
        remote: &str,
        url: &str,
    ) -> Option<&[GitRemoteRef]> {
        let key = (
            identity_repo.map(Path::to_path_buf),
            remote.to_owned(),
            url.to_owned(),
        );
        self.kept.get(&key).map(Vec::as_slice)
    }

    /// Whether the kept advertisement of a selected repository's destination
    /// already shows every destination ref of `plan` at its source object, so
    /// the transfer would change nothing. A deletion always transfers.
    pub(super) fn already_on_origin(&self, path: &Path, plan: &GitPreparedPush) -> bool {
        let Some(advertised) = self.kept(Some(path), &plan.remote, &plan.url) else {
            return false;
        };
        !plan.refspecs.is_empty()
            && plan.refspecs.iter().all(|refspec| {
                refspec_parts(refspec).is_some_and(|(source, destination)| {
                    !source.is_empty()
                        && advertised.iter().any(|reference| {
                            reference.name == destination && reference.target == source
                        })
                })
            })
    }

    /// Note the transfers this operation makes before its root proof. A forced
    /// or deleting transfer can rewind any destination, and URLs cannot tell
    /// which destinations share a repository, so after one no kept
    /// advertisement and no accepted push counts as evidence: the proof reads
    /// every dependency again.
    pub(super) fn expect_transfers<'a>(
        &mut self,
        plans: impl IntoIterator<Item = &'a GitPreparedPush>,
    ) {
        if plans.into_iter().any(may_rewind) {
            self.kept.clear();
            self.voided = true;
        }
    }

    /// Read, before any transfer, every destination `targets` need, and keep
    /// each advertisement. A target needs its own destination and, when it
    /// carries the frozen root request, every root-lock dependency's
    /// destination, in that order. The round is planned before any read runs,
    /// so a destination is read once whichever targets need it, and it runs
    /// `jobs` reads at once, at most `per_host` to one host. Returns each
    /// target's failure: the first in the order that target needs its reads,
    /// the one reading in turn would report.
    pub(super) fn read_before_transfers<B: GitBackend + Sync>(
        backend: &B,
        root: &Path,
        targets: &[PreflightTarget<'_>],
        jobs: usize,
        per_host: usize,
    ) -> (Self, Vec<Option<ModelError>>) {
        let mut round = ReadRound::default();
        let mut needs = Vec::with_capacity(targets.len());
        for target in targets {
            let path = target.path.as_path();
            let own = round.plan(path, &target.plan.url, &target.plan.remote, Some(path));
            let mut need = vec![Ok(own)];
            if let Some(request) = target.root_request {
                match root_dependencies(backend, root, request) {
                    Ok(dependencies) => need.extend(
                        dependencies
                            .iter()
                            .map(|dependency| Ok(round.plan_dependency(root, dependency))),
                    ),
                    Err(error) => need.push(Err(error)),
                }
            }
            needs.push(need);
        }
        let results = round.run(backend, jobs, per_host);
        // Every read is in; keep the advertisements in plan order.
        let mut reads = Self::default();
        let mut failures = Vec::with_capacity(results.len());
        for (read, result) in round.reads.into_iter().zip(results) {
            match result {
                Ok(advertised) => {
                    reads
                        .kept
                        .insert((read.identity_repo, read.remote, read.url), advertised);
                    failures.push(None);
                }
                Err(error) => failures.push(Some(error)),
            }
        }
        let target_failures = needs
            .into_iter()
            .map(|need| {
                need.into_iter().find_map(|step| match step {
                    Ok(read) => failures[read].clone(),
                    Err(error) => Some(error),
                })
            })
            .collect();
        (reads, target_failures)
    }
}

/// A target of the pre-transfer reads: the repository at `path`, pushed by
/// `plan`, and for the root its frozen request, whose lock dependencies it
/// also needs read.
pub(super) struct PreflightTarget<'a> {
    pub(super) path: PathBuf,
    pub(super) plan: &'a GitPreparedPush,
    pub(super) root_request: Option<&'a crate::PushRequest>,
}

/// One round of reads, planned before any read runs: each destination once,
/// keyed as [`ReadPreflight`] keeps it, in the order it was first needed. That
/// first need also chooses the repository the read runs in.
#[derive(Default)]
struct ReadRound {
    reads: Vec<DestinationRead>,
}

struct DestinationRead {
    path: PathBuf,
    identity_repo: Option<PathBuf>,
    remote: String,
    url: String,
}

impl ReadRound {
    /// The read that answers a destination, planned when no earlier need
    /// planned it.
    fn plan(
        &mut self,
        path: &Path,
        url: &str,
        remote: &str,
        identity_repo: Option<&Path>,
    ) -> usize {
        let planned = self.reads.iter().position(|read| {
            read.identity_repo.as_deref() == identity_repo
                && read.remote == remote
                && read.url == url
        });
        planned.unwrap_or_else(|| {
            self.reads.push(DestinationRead {
                path: path.to_path_buf(),
                identity_repo: identity_repo.map(Path::to_path_buf),
                remote: remote.to_owned(),
                url: url.to_owned(),
            });
            self.reads.len() - 1
        })
    }

    /// A dependency's read, which runs in the workspace root.
    fn plan_dependency(&mut self, root: &Path, dependency: &PublicationDependency) -> usize {
        self.plan(
            root,
            &dependency.read_url,
            &dependency.remote,
            dependency.identity_repo(),
        )
    }

    /// Run every planned read, `jobs` at once and at most `per_host` to the
    /// host its URL reaches, keyed as push transfers are. Returns only when
    /// every read is done, with the results in plan order.
    fn run<B: GitBackend + Sync>(
        &self,
        backend: &B,
        jobs: usize,
        per_host: usize,
    ) -> Vec<ModelResult<Vec<GitRemoteRef>>> {
        let reads: Vec<&DestinationRead> = self.reads.iter().collect();
        par_map_per_host(
            reads,
            jobs,
            per_host,
            |read| git_host(&read.url),
            |read| {
                backend.ls_remote_url(
                    &read.path,
                    &read.url,
                    &read.remote,
                    read.identity_repo.as_deref(),
                )
            },
        )
    }
}

/// A captured refspec's source and destination, without its `+`.
fn refspec_parts(refspec: &str) -> Option<(&str, &str)> {
    refspec.strip_prefix('+').unwrap_or(refspec).split_once(':')
}

/// Whether a captured transfer can take history away from its destination: a
/// forced (`+`) or deleting refspec. An ordinary transfer is refused unless it
/// fast-forwards.
fn may_rewind(plan: &GitPreparedPush) -> bool {
    plan.refspecs.iter().any(|refspec| {
        refspec.starts_with('+')
            || refspec_parts(refspec).is_none_or(|(source, _)| source.is_empty())
    })
}

pub(super) fn freeze_root_request<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
) -> ModelResult<crate::PushRequest> {
    let head = backend.head(root)?;
    let refspec = super::resolve_push_refspec(&head, request)?;
    let force = refspec.starts_with('+');
    let plain = refspec.strip_prefix('+').unwrap_or(&refspec);
    let (source, destination) = plain.split_once(':').ok_or_else(|| {
        refused("root publication requires one explicit source:destination refspec")
    })?;
    if source.is_empty() {
        return Ok(request.clone());
    } // deletion publishes no lock
    if source.contains('*') || destination.contains('*') || destination.contains(':') {
        return Err(refused(
            "root publication requires a single concrete refspec",
        ));
    }
    let source_object = backend
        .read_ref(root, source)?
        .ok_or_else(|| refused("root push source does not resolve"))?;
    let mut pinned = request.clone();
    pinned.refspec = Some(format!(
        "{}{source_object}:{destination}",
        if force { "+" } else { "" }
    ));
    Ok(pinned)
}

/// Prove every dependency of the frozen root source after member transfers. A
/// destination this operation pushed to is proven by that push (D8), and any
/// other by its advertisement kept from before the transfers when that shows
/// the commit available (D9). Anything else is read, and so is everything once
/// the operation makes a forced or deleting transfer. The reads run in turn,
/// as tag publication proves its root; push runs them concurrently
/// ([`checked_root_request_concurrently`]).
pub(super) fn checked_root_request<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
    published: &BTreeMap<String, GitPreparedPush>,
    reads: &ReadPreflight,
) -> ModelResult<crate::PushRequest> {
    let proof = RootProof::plan(backend, root, request, published, reads)?;
    proof.check(backend, PUBLISH_OR_FETCH, |_, dependency| {
        backend.ls_remote_url(
            root,
            &dependency.read_url,
            &dependency.remote,
            dependency.identity_repo(),
        )
    })
}

/// [`checked_root_request`] for push. The unproven dependencies' reads are
/// planned first, a destination once however many dependencies it answers, and
/// run `jobs` at once, at most `per_host` to one host, all after the member
/// transfers. The proof then decides in lock order, so it refuses with the
/// failure reading in turn would meet first. A push under the default refuses
/// with the remedies of §3.6.
pub(super) fn checked_root_request_concurrently<B: GitBackend + Sync>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
    published: &BTreeMap<String, GitPreparedPush>,
    reads: &ReadPreflight,
    jobs: usize,
    per_host: usize,
) -> ModelResult<crate::PushRequest> {
    let proof = RootProof::plan(backend, root, request, published, reads)?;
    let mut round = ReadRound::default();
    let answers: Vec<usize> = proof
        .unproven
        .iter()
        .map(|dependency| round.plan_dependency(root, dependency))
        .collect();
    let results = round.run(backend, jobs, per_host);
    let remedy = if matches!(request.remote_check, Some(crate::RemoteCheck::Always)) {
        PUBLISH_OR_FETCH
    } else {
        PUBLISH_FETCH_OR_CHECK_REMOTES
    };
    proof.check(backend, remedy, |index, _| results[answers[index]].clone())
}

/// What a refused root proof asks of the operator, as tag publication and a
/// push that checks every remote ask it.
const PUBLISH_OR_FETCH: &str = "publish the member, or fetch its advertised history and retry";

/// What a refused root proof asks of the operator after a push under the
/// default (§3.6). That push may not have checked the member's remote for its
/// own push, so `--check-remotes` can publish the member.
const PUBLISH_FETCH_OR_CHECK_REMOTES: &str = "publish the member by pushing a branch that contains the commit, fetch its advertised history and retry, or run gwz push --check-remotes, which re-checks the members and pushes those whose remote lacks their branch's commit";

/// The root proof after the member transfers: the frozen root request, and, in
/// lock order, the dependencies that no accepted push and no kept advertisement
/// proves.
struct RootProof {
    pinned: crate::PushRequest,
    unproven: Vec<PublicationDependency>,
}

impl RootProof {
    fn plan<B: GitBackend>(
        backend: &B,
        root: &Path,
        request: &crate::PushRequest,
        published: &BTreeMap<String, GitPreparedPush>,
        reads: &ReadPreflight,
    ) -> ModelResult<Self> {
        let pinned = freeze_root_request(backend, root, request)?;
        let mut unproven = Vec::new();
        for dependency in root_dependencies(backend, root, &pinned)? {
            let proven = if reads.voided {
                false
            } else if pushed_to(&dependency, published).is_some() {
                dependency_was_published(backend, &dependency, published)?
            } else {
                match reads.kept(
                    dependency.identity_repo(),
                    &dependency.remote,
                    &dependency.read_url,
                ) {
                    Some(advertised) => available(backend, &dependency, advertised)?,
                    None => false,
                }
            };
            if !proven {
                unproven.push(dependency);
            }
        }
        Ok(Self { pinned, unproven })
    }

    /// Prove each unproven dependency, in lock order, from the read that
    /// `advertisement` answers for it. The first read that fails, or that does
    /// not show the commit available, refuses the root, asking for `remedy`.
    fn check<B: GitBackend>(
        self,
        backend: &B,
        remedy: &str,
        mut advertisement: impl FnMut(usize, &PublicationDependency) -> ModelResult<Vec<GitRemoteRef>>,
    ) -> ModelResult<crate::PushRequest> {
        for (index, dependency) in self.unproven.iter().enumerate() {
            let advertised = advertisement(index, dependency)?;
            if !available(backend, dependency, &advertised)? {
                // Name the URL that was read when it is not the committed one.
                let read_through = if dependency.read_url == dependency.url {
                    String::new()
                } else {
                    format!(" (read through {})", dependency.read_url)
                };
                return Err(refused(format!(
                    "root publication blocked: cannot prove member {} commit {} is available at its committed fetch remote {}{read_through}; {remedy}",
                    dependency.member_id, dependency.commit, dependency.remote
                )));
            }
        }
        Ok(self.pinned)
    }
}

/// Whether an advertisement proves the dependency's commit available: the
/// commit is advertised, or, for a materialized member, is an ancestor of an
/// advertised object. An advertised commit proves it without asking ancestry.
/// An ancestry query that fails is a local Git error, not a disproof, so it is
/// reported when nothing else proves the commit: none of the remedies a
/// publication refusal names would repair a missing or shallow object.
fn available<B: GitBackend>(
    backend: &B,
    dependency: &PublicationDependency,
    advertised: &[GitRemoteRef],
) -> ModelResult<bool> {
    if advertised
        .iter()
        .any(|reference| reference.target == dependency.commit)
    {
        return Ok(true);
    }
    if dependency.identity_repo().is_none() {
        return Ok(false);
    }
    let mut failure = None;
    for reference in advertised {
        match backend.is_ancestor(&dependency.path, &dependency.commit, &reference.target) {
            Ok(true) => {
                return Ok(true);
            }
            Ok(false) => {}
            Err(error) => {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(false),
    }
}

/// This operation's accepted push to the dependency's destination: its
/// member's push through the same remote to its read URL, which every other
/// read of it uses.
fn pushed_to<'a>(
    dependency: &PublicationDependency,
    published: &'a BTreeMap<String, GitPreparedPush>,
) -> Option<&'a GitPreparedPush> {
    published
        .get(&dependency.member_id)
        .filter(|plan| plan.remote == dependency.remote && plan.url == dependency.read_url)
}

/// A member push to the dependency's destination that the remote accepted
/// during this operation is stronger evidence than a read (D8). A remote
/// accepts a ref update only with the full history of the new object, so the
/// push proves the locked commit when that is a pushed source or an ancestor
/// of one. A pushed source equal to the commit proves it without asking
/// ancestry; an ancestry query that fails is a local Git error, reported when
/// nothing else proves the commit rather than read as a disproof, because no
/// remedy a publication refusal names would repair it. Ordinary and forced
/// pushes both count: the `+` prefix decides only whether the remote may
/// rewind the destination, not which object it now holds. A forced or deleting
/// transfer voids this evidence for the whole operation instead
/// ([`ReadPreflight::expect_transfers`]).
pub(super) fn dependency_was_published<B: GitBackend>(
    backend: &B,
    dependency: &PublicationDependency,
    published: &BTreeMap<String, GitPreparedPush>,
) -> ModelResult<bool> {
    let Some(plan) = pushed_to(dependency, published) else {
        return Ok(false);
    };
    let mut ancestry = Vec::new();
    for (source, _) in plan
        .refspecs
        .iter()
        .filter_map(|refspec| refspec_parts(refspec))
    {
        if source == dependency.commit {
            return Ok(true);
        }
        if !source.is_empty() {
            ancestry.push(source);
        }
    }
    let mut failure = None;
    for source in ancestry {
        match backend.is_ancestor(&dependency.path, &dependency.commit, source) {
            Ok(true) => {
                return Ok(true);
            }
            Ok(false) => {}
            Err(error) => {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(false),
    }
}

pub(super) struct PublicationDependency {
    pub member_id: String,
    pub path: std::path::PathBuf,
    pub commit: String,
    pub remote: String,
    /// The committed fetch URL, which names the repository to prove.
    pub url: String,
    /// The one URL every read of this dependency uses, and the rule of
    /// [`select_read_url`] that chose it.
    pub read_url: String,
    pub read_rule: ReadUrlRule,
}

impl PublicationDependency {
    /// The repository whose SSH configuration reads of this dependency use:
    /// none for a member that is not materialized, the only case rule 3 chooses.
    fn identity_repo(&self) -> Option<&Path> {
        (self.read_rule != ReadUrlRule::EffectiveScheme).then_some(self.path.as_path())
    }
}

pub(super) fn validate_dependency_identity<B: GitBackend>(
    backend: &B,
    dependency: &PublicationDependency,
) -> ModelResult<()> {
    backend.validate_url_identity(
        dependency.identity_repo(),
        &dependency.remote,
        &dependency.read_url,
    )
}

/// Advertise refs from the effective destination with the same identity owner.
/// Read access is deliberately not presented as proof of push permission.
pub(super) fn preflight_remote<B: GitBackend>(
    backend: &B,
    path: &Path,
    name: &str,
    push: bool,
) -> ModelResult<()> {
    let remote = backend
        .remotes(path)?
        .into_iter()
        .find(|remote| remote.name == name)
        .ok_or_else(|| ModelError::new(ErrorCode::MissingRemote, "remote is not configured"))?;
    let url = if push {
        remote.push_url.or(remote.url)
    } else {
        remote.url
    }
    .ok_or_else(|| ModelError::new(ErrorCode::MissingRemote, "remote has no destination URL"))?;
    backend
        .ls_remote_url(path, &url, name, Some(path))
        .map(|_| ())
}

/// Read every root-lock dependency's destination before any transfer, in turn,
/// which confirms read access; a destination already read is not read again.
/// Tag publication preflights its root so. Push reads its dependencies in one
/// round with its other destinations ([`ReadPreflight::read_before_transfers`]).
pub(super) fn preflight_dependencies<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
) -> ModelResult<()> {
    let mut reads = ReadPreflight::default();
    for dependency in root_dependencies(backend, root, request)? {
        reads.read(
            backend,
            root,
            &dependency.read_url,
            &dependency.remote,
            dependency.identity_repo(),
        )?;
    }
    Ok(())
}

/// Read-only dependencies from an already frozen root source. Preflight and
/// publication share this interpretation; neither consults the worktree lock.
pub(super) fn root_dependencies<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
) -> ModelResult<Vec<PublicationDependency>> {
    let refspec = request
        .refspec
        .as_deref()
        .ok_or_else(|| refused("missing root refspec"))?;
    let (source_object, _) = refspec
        .strip_prefix('+')
        .unwrap_or(refspec)
        .split_once(':')
        .ok_or_else(|| refused("invalid frozen root refspec"))?;
    if source_object.is_empty() {
        return Ok(Vec::new());
    }
    // Inspect the commit behind an annotated tag but publish the tag object,
    // preserving its annotation/signature and its exact source identity.
    let commit = backend
        .read_ref(root, &format!("{source_object}^{{commit}}"))?
        .ok_or_else(|| refused("root publication source does not resolve to a commit"))?;
    let mut dependencies = Vec::new();
    if let Some(lock_bytes) = backend.read_file_at_commit(root, &commit, artifact::LOCK_PATH)? {
        let lock = LockArtifact::from_yaml(
            std::str::from_utf8(&lock_bytes).map_err(|_| refused("committed lock is not UTF-8"))?,
        )?;
        let manifest_bytes = backend
            .read_file_at_commit(root, &commit, WORKSPACE_MANIFEST)?
            .ok_or_else(|| refused("committed lock has no accompanying manifest"))?;
        let manifest = ManifestArtifact::from_yaml(
            std::str::from_utf8(&manifest_bytes)
                .map_err(|_| refused("committed manifest is not UTF-8"))?,
        )?;
        if lock.workspace_id != manifest.workspace.id {
            return Err(refused(
                "committed manifest and lock identify different workspaces",
            ));
        }
        // Only a member that is not materialized reads through the effective
        // scheme, so it is resolved on first need: an unreadable preference
        // refuses only a publication that would use it.
        let mut effective_scheme = None;
        for (id, state) in &lock.members {
            if state.source_kind != ArtifactSourceKind::Git {
                return Err(ModelError::new(
                    ErrorCode::UnsupportedSourceKind,
                    format!(
                        "root publication blocked: member {id} uses {:?}, whose remote availability cannot yet be verified",
                        state.source_kind
                    ),
                ));
            }
            let Some(oid) = state.commit.as_deref() else {
                return Err(refused(format!(
                    "root publication blocked: Git member {id} has no pinned commit; resolve and commit its lock state before publishing root"
                )));
            };
            let member = manifest
                .members
                .iter()
                .find(|member| &member.id == id)
                .ok_or_else(|| {
                    refused(format!("committed lock member {id} has no manifest entry"))
                })?;
            if state.source_id.as_deref() != Some(member.source_id.as_str())
                || state.path != member.path
                || state.source_kind != member.source_kind
            {
                return Err(refused(format!(
                    "committed lock member {id} has inconsistent source identity or path"
                )));
            }
            let remote = member.remotes.iter().find(|remote| remote.fetch)
                .ok_or_else(|| refused(format!("committed lock member {id} has no fetch URL; publish it and record its remote before publishing root")))?;
            let path = root.join(&member.path);
            let read = if backend.is_repository(&path)? {
                // Local configuration only; no network.
                let remotes = backend.remotes(&path)?;
                let configured = remotes
                    .iter()
                    .find(|candidate| candidate.name == remote.name);
                select_read_url(&remote.url, DependencyMember::Materialized(configured))
            } else {
                let scheme = match effective_scheme {
                    Some(scheme) => scheme,
                    None => {
                        let scheme =
                            resolve_push_url_scheme(root, requested_url_scheme(&request.meta))?;
                        effective_scheme = Some(scheme);
                        scheme
                    }
                };
                select_read_url(&remote.url, DependencyMember::Unmaterialized(scheme))
            };
            dependencies.push(PublicationDependency {
                member_id: id.clone(),
                path,
                commit: oid.to_owned(),
                remote: remote.name.clone(),
                url: remote.url.clone(),
                read_url: read.url,
                read_rule: read.rule,
            });
        }
    }
    Ok(dependencies)
}

fn refused(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::RemoteRejected, message)
}

pub(super) fn attach_transport<B: GitBackend>(backend: &B, response: &mut crate::ResponseEnvelope) {
    let Some(observations) = backend.transport_observations() else {
        return;
    };
    let mut rows = observations.snapshot();
    rows.extend(response.meta.transport.take().unwrap_or_default());
    if !rows.is_empty() {
        response.meta.transport = Some(rows);
    }
}

pub(super) fn attach_transport_error<B: GitBackend>(
    backend: &B,
    mut error: ModelError,
    context: &crate::operation::OperationContext,
) -> ModelError {
    let mut rows = backend
        .transport_observations()
        .map(|value| value.snapshot())
        .unwrap_or_default();
    if let Some(meta) = error.response_meta.as_mut() {
        rows.extend(meta.transport.take().unwrap_or_default());
    }
    if !rows.is_empty() {
        error.response_meta = Some(Box::new(crate::ResponseMeta {
            request_id: context.request_id.clone(),
            schema_version: context.schema_version.clone(),
            action: context.action.into(),
            aggregate_status: crate::AggregateStatus::Failed,
            operation_id: Some(context.operation_id.clone()),
            message: None,
            attribution: context.attribution.as_ref().map(Into::into),
            transport: Some(rows),
            ..Default::default()
        }));
    }
    error
}
