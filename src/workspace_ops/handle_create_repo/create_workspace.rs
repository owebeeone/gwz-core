use std::collections::BTreeMap;
use std::path::Path;

use crate::artifact::{self, LockArtifact, ManifestArtifact, WorkspaceHeader};
use crate::git::{Git2Backend, GitBackend};
use crate::model::ModelResult;
use crate::operation::{OperationRequest, WorkspaceMutatorLock};
use crate::workspace::preflight_create_workspace;

use super::super::*;
use super::invocation::{invocation_start, normalize_absolute_path, resolve_invocation_path};
use super::response::response_envelope;

pub fn handle_create_workspace(
    request: crate::CreateWorkspaceRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CreateWorkspaceResponse> {
    let backend = Git2Backend::new();
    let services = crate::operation_context::OperationServices::for_merge(&backend);
    handle_create_workspace_in(&services, &backend, request, operation_id)
}

pub(crate) fn handle_create_workspace_in<B>(
    services: &crate::operation_context::OperationServices,
    backend: &B,
    request: crate::CreateWorkspaceRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CreateWorkspaceResponse>
where
    B: GitBackend,
{
    let context =
        OperationRequest::CreateWorkspace(request.clone()).context(operation_id.into())?;
    let root = if request.meta.invocation.is_some() {
        let caller_start = invocation_start(Path::new(&request.workspace_root), &request.meta)?;
        resolve_invocation_path(&caller_start, &request.workspace_root)?
    } else {
        normalize_absolute_path(Path::new(&request.workspace_root), "workspace root")?
    };
    preflight_create_workspace(&root)?;
    preflight_workspace_bootstrap_files_in(
        services.filesystem(),
        &root,
        force_bootstrap_overwrite(&request.meta),
    )?;
    let workspace_id = request
        .workspace_id
        .clone()
        .unwrap_or_else(|| "ws_default".to_owned());
    crate::model::WorkspaceId::parse_str(&workspace_id)?;
    // DR-4: everything above is validation — `preflight_create_workspace` refuses an
    // existing workspace, `preflight_workspace_bootstrap_files` refuses hand-edited
    // bootstrap files, and the id is parsed. A dry run stops here, before the git repo,
    // the manifest/lock pair and the AGENTS/.claude bootstrap files are created. The
    // `init --update` shape is gated separately in `workspace_bootstrap.rs`.
    if request.meta.dry_run.unwrap_or(false) {
        let mut response = response_envelope(context, crate::AggregateStatus::Accepted, Vec::new());
        response.meta.message = Some(format!(
            "would create workspace '{workspace_id}' at {}",
            root.display()
        ));
        return Ok(crate::CreateWorkspaceResponse { response });
    }
    ensure_workspace_git_repo(backend, &root)?;
    let _guard = WorkspaceMutatorLock::acquire_in(services, &root)?;

    let manifest = ManifestArtifact {
        schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: workspace_id.clone(),
        },
        members: Vec::new(),
    };
    let lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id,
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        members: BTreeMap::new(),
    };
    // CAPABILITY-FREE EXCEPTION, §10 rows `:278`/`:279`: `gwz repo create`, add-existing and workspace create are all capability-free (E0.2 §5.2), so all four writer pairs in this file stay raw permanently (2026-09-02, GwzM5-8R2E-CapabilityFreeAmendment.md §3).
    artifact::write_manifest_and_lock_in(services.filesystem(), &root, &manifest, &lock)?;
    sync_workspace_boundary_in(services.filesystem(), backend, &root, &manifest, &lock)?;
    let bootstrap = ensure_workspace_bootstrap_files_in(
        services.filesystem(),
        backend,
        &root,
        false,
        force_bootstrap_overwrite(&request.meta),
    )?;

    let mut response = response_envelope(context, crate::AggregateStatus::Ok, Vec::new());
    // A `.claude/settings.json` the deny-rule merge declined to touch must not vanish
    // silently just because this handler has no other message to carry.
    if !bootstrap.notes.is_empty() {
        response.meta.message = Some(bootstrap.notes.join("; "));
    }
    Ok(crate::CreateWorkspaceResponse { response })
}

pub(crate) fn ensure_workspace_git_repo<B: GitBackend>(
    backend: &B,
    root: &Path,
) -> ModelResult<()> {
    if backend.is_repository(root)? {
        Ok(())
    } else {
        backend.create_repo(root).map(|_| ())
    }
}
