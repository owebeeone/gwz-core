//! Root publication is a dependency barrier, not a cross-server transaction.
//! Inspect the exact source commit, prove its lock dependencies at committed
//! fetch URLs, then freeze the push source so a branch move cannot swap its lock.
use std::path::Path;

use crate::artifact::{self, ArtifactSourceKind, LockArtifact, ManifestArtifact};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;

/// Remote reads completed before any publication starts.  The identity owner is
/// part of the key: the same URL reached with a different repository's SSH
/// configuration still needs its own authentication check.
#[derive(Default)]
pub(super) struct ReadPreflight {
    checked: std::collections::BTreeSet<(std::path::PathBuf, String, String)>,
}

impl ReadPreflight {
    pub(super) fn record(&mut self, identity_repo: &Path, remote: &str, url: &str) {
        self.checked.insert((
            identity_repo.to_path_buf(),
            remote.to_owned(),
            url.to_owned(),
        ));
    }

    fn contains(&self, identity_repo: &Path, remote: &str, url: &str) -> bool {
        self.checked.contains(&(
            identity_repo.to_path_buf(),
            remote.to_owned(),
            url.to_owned(),
        ))
    }
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

pub(super) fn checked_root_request<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
    published: &std::collections::BTreeMap<String, crate::git::GitPreparedPush>,
) -> ModelResult<crate::PushRequest> {
    let pinned = freeze_root_request(backend, root, request)?;
    for dependency in root_dependencies(backend, root, &pinned)? {
        if dependency_was_published(&dependency, published) {
            continue;
        }
        let materialized = backend.is_repository(&dependency.path)?;
        let advertised = backend.ls_remote_url(
            root,
            &dependency.url,
            &dependency.remote,
            materialized.then_some(dependency.path.as_path()),
        )?;
        let mut available = advertised
            .iter()
            .any(|reference| reference.target == dependency.commit);
        if !available && materialized {
            for reference in &advertised {
                if backend
                    .is_ancestor(&dependency.path, &dependency.commit, &reference.target)
                    .unwrap_or(false)
                {
                    available = true;
                    break;
                }
            }
        }
        if !available {
            return Err(refused(format!(
                "root publication blocked: cannot prove member {} commit {} is available at its committed fetch remote {}; publish the member, or fetch its advertised history and retry",
                dependency.member_id, dependency.commit, dependency.remote
            )));
        }
    }
    Ok(pinned)
}

/// A completed member push of the exact commit in the frozen root lock is
/// stronger evidence than a second read advertisement: that remote accepted
/// the object during this operation.  Keep the comparison intentionally
/// exact; an ahead member still receives the ordinary remote proof below.
fn dependency_was_published(
    dependency: &PublicationDependency,
    published: &std::collections::BTreeMap<String, crate::git::GitPreparedPush>,
) -> bool {
    let Some(plan) = published.get(&dependency.member_id) else {
        return false;
    };
    plan.remote == dependency.remote
        && plan.url == dependency.url
        && plan.refspecs.iter().any(|refspec| {
            refspec
                .strip_prefix('+')
                .and_then(|value| value.split_once(':'))
                .map(|(source, _)| source == dependency.commit)
                .unwrap_or(false)
        })
}

pub(super) struct PublicationDependency {
    pub member_id: String,
    pub path: std::path::PathBuf,
    pub commit: String,
    pub remote: String,
    pub url: String,
}

pub(super) fn validate_dependency_identity<B: GitBackend>(
    backend: &B,
    dependency: &PublicationDependency,
) -> ModelResult<()> {
    let materialized = backend.is_repository(&dependency.path)?;
    backend.validate_url_identity(
        materialized.then_some(dependency.path.as_path()),
        &dependency.remote,
        &dependency.url,
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

pub(super) fn preflight_dependencies<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
) -> ModelResult<()> {
    preflight_dependencies_with_reads(backend, root, request, &mut ReadPreflight::default())
}

/// Confirm that every root-lock dependency has read access before any push.
/// Previously checked destinations can be reused only in this pre-transfer
/// phase. `checked_root_request` deliberately reads again after member pushes
/// to prove the pinned objects are now advertised.
pub(super) fn preflight_dependencies_with_reads<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
    reads: &mut ReadPreflight,
) -> ModelResult<()> {
    for dependency in root_dependencies(backend, root, request)? {
        let materialized = backend.is_repository(&dependency.path)?;
        if !materialized {
            backend.ls_remote_url(root, &dependency.url, &dependency.remote, None)?;
            continue;
        }
        let identity_repo = dependency.path.as_path();
        if !reads.contains(identity_repo, &dependency.remote, &dependency.url) {
            backend.ls_remote_url(
                root,
                &dependency.url,
                &dependency.remote,
                Some(identity_repo),
            )?;
            reads.record(identity_repo, &dependency.remote, &dependency.url);
        }
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
            dependencies.push(PublicationDependency {
                member_id: id.clone(),
                path: root.join(&member.path),
                commit: oid.to_owned(),
                remote: remote.name.clone(),
                url: remote.url.clone(),
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
        }));
    }
    error
}
