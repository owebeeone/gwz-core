//! Root publication is a dependency barrier, not a cross-server transaction.
//! Inspect the exact source commit, prove its lock dependencies at committed
//! fetch URLs, then freeze the push source so a branch move cannot swap its lock.
use std::path::Path;

use crate::artifact::{self, ArtifactSourceKind, LockArtifact, ManifestArtifact};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;

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
) -> ModelResult<crate::PushRequest> {
    let pinned = freeze_root_request(backend, root, request)?;
    let refspec = pinned
        .refspec
        .as_deref()
        .ok_or_else(|| refused("missing root refspec"))?;
    let (source_object, _) = refspec
        .strip_prefix('+')
        .unwrap_or(refspec)
        .split_once(':')
        .ok_or_else(|| refused("invalid frozen root refspec"))?;
    if source_object.is_empty() {
        return Ok(pinned);
    }
    // Inspect the commit behind an annotated tag but publish the tag object,
    // preserving its annotation/signature and its exact source identity.
    let commit = backend
        .read_ref(root, &format!("{source_object}^{{commit}}"))?
        .ok_or_else(|| refused("root publication source does not resolve to a commit"))?;
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
            let member_path = root.join(&member.path);
            let materialized = backend.is_repository(&member_path)?;
            let advertised = backend.ls_remote_url(root, &remote.url, &remote.name, materialized.then_some(member_path.as_path()))?;
            let mut available = advertised.iter().any(|reference| reference.target == oid);
            if !available && materialized {
                // A known descendant advertised by the server proves the pinned
                // ancestor is reachable there. Unknown graph evidence never passes.
                for reference in &advertised {
                    if backend
                        .is_ancestor(&member_path, oid, &reference.target)
                        .unwrap_or(false)
                    {
                        available = true;
                        break;
                    }
                }
            }
            if !available {
                return Err(refused(format!(
                    "root publication blocked: cannot prove member {id} commit {oid} is available at its committed fetch remote {}; publish the member, or fetch its advertised history and retry",
                    remote.name
                )));
            }
        }
    }
    Ok(pinned)
}

fn refused(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::RemoteRejected, message)
}
