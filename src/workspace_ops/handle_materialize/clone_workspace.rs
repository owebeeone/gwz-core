use std::path::{Path, PathBuf};

use crate::artifact::{self, ManifestArtifact};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, EventSink, OperationRequest};
use crate::workspace::WORKSPACE_MANIFEST;

use super::super::*;
use super::apply::apply_materialize_plans;
use super::materialize::{
    MaterializeApplyOptions, prepare_materialize_execution, validate_materialize_identities,
};

/// Clone a workspace from its root repository URL and complete it.
///
/// This is the one-shot form of `git clone <url> <target>` followed by
/// `gwz materialize --lock`: it clones the workspace root (the git repository
/// that owns `gwz.conf/`), verifies it is a GWZ workspace, then materializes
/// every member to the committed lock — cloning missing member repositories and
/// checking out their locked commits.
pub fn handle_clone_workspace_request<B>(
    backend: &B,
    start: &Path,
    request: crate::CloneWorkspaceRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::CloneWorkspaceResponse>
where
    B: GitBackend + Sync,
{
    let mut request = request;
    let start = invocation_start(start, &request.meta)?;
    request.url = resolve_invocation_git_source(&start, &request.url)?;
    // No workspace exists yet, so only the request can name a scheme; the root
    // URL is derived like a member URL and a refusal stops before any network.
    let scheme = resolve_url_scheme(None, requested_url_scheme(&request.meta))?;
    let root_resolution = resolve_root_url(&request.url, scheme)?;
    request.url = root_resolution.effective_url.clone();
    let context = OperationRequest::CloneWorkspace(request.clone()).context(operation_id.into())?;
    let scoped_backend = backend.with_transport(&start, request.meta.transport.as_ref())?;
    let backend = scoped_backend.as_ref().unwrap_or(backend);
    let error_context = context.clone();
    let result: ModelResult<crate::CloneWorkspaceResponse> = (|| {
        if request.meta.dry_run.unwrap_or(false) {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "--dry-run is not supported for clone",
            ));
        }
        let target_path = resolve_invocation_path(&start, &request.target)?;
        // Refuse to clone over an existing workspace rather than corrupt it.
        if target_path.join(WORKSPACE_MANIFEST).exists() {
            return Err(ModelError::new(
                ErrorCode::WorkspaceAlreadyExists,
                "clone target already contains a GWZ workspace",
            ));
        }
        backend.validate_url_identity(None, "origin", &request.url)?;
        if has_explicit_target_selection(request.meta.selection.as_ref())
            || request
                .meta
                .transport
                .as_ref()
                .is_some_and(|options| !options.remote_identities.is_empty())
        {
            let bytes = backend
                .read_remote_file(&request.url, "origin", WORKSPACE_MANIFEST)?
                .ok_or_else(|| {
                    ModelError::new(
                        ErrorCode::WorkspaceNotFound,
                        "remote has no workspace manifest",
                    )
                })?;
            let manifest = ManifestArtifact::from_yaml(
                std::str::from_utf8(&bytes)
                    .map_err(|_| invalid("remote workspace manifest is not UTF-8"))?,
            )?;
            let selected = resolve_action_targets(
                &manifest,
                request.meta.selection.as_ref(),
                crate::ActionKind::CloneWorkspace,
            )?;
            let mut names = vec!["origin".to_owned()];
            for target in selected {
                if let SelectedTarget::Member(member) = target
                    && let Some(remote) = member.remotes.iter().find(|remote| remote.fetch)
                {
                    names.push(remote.name.clone());
                }
            }
            backend.validate_transport_remotes(&names)?;
        }
        let progress_interval = request
            .meta
            .policy
            .as_ref()
            .and_then(|policy| policy.progress_min_interval_ms)
            .unwrap_or(0);
        let emitter = EventEmitter::new(&context, events, progress_interval);
        emitter.operation_started();
        let response = clone_workspace_with_emitter(
            backend,
            request,
            target_path,
            context,
            &emitter,
            scheme,
            &root_resolution,
        );
        emitter.operation_finished();
        response
    })();
    result
        .map_err(|error| {
            super::super::publication::attach_transport_error(backend, error, &error_context)
        })
        .map(|mut response| {
            super::super::publication::attach_transport(backend, &mut response.response);
            response
        })
}

fn clone_workspace_with_emitter<B>(
    backend: &B,
    request: crate::CloneWorkspaceRequest,
    target_path: PathBuf,
    context: crate::operation::OperationContext,
    emitter: &EventEmitter<'_>,
    scheme: EffectiveUrlScheme,
    root_resolution: &crate::git::UrlResolution,
) -> ModelResult<crate::CloneWorkspaceResponse>
where
    B: GitBackend + Sync,
{
    let target_display = target_path.to_string_lossy().into_owned();
    // Emit the root repository clone as a member-like lifecycle so consumers can render
    // the full one-shot clone operation without a separate event schema.
    emitter.member_started("workspace_root", &target_display);
    backend
        .clone_repo_with_progress(&request.url, &target_path, &|progress| {
            emitter.member_progress("workspace_root", &target_display, progress)
        })
        .map_err(|error| ModelError {
            message: append_url_scheme_hint(error.message, &request.url, scheme.scheme),
            ..error
        })?;
    emitter.member_finished("workspace_root", &target_display);

    // Verify the cloned repository really is a GWZ workspace before mutating it.
    if !target_path.join(WORKSPACE_MANIFEST).is_file() {
        return Err(ModelError::new(
            ErrorCode::WorkspaceNotFound,
            format!("cloned repository is not a GWZ workspace: {WORKSPACE_MANIFEST} missing"),
        ));
    }

    // Complete the clone: materialize members to the committed lock using the same
    // emitter so event sequence numbers remain monotonic for subscribers.
    let workspace_id = request
        .meta
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.workspace_id.clone());
    let materialize = crate::MaterializeRequest {
        meta: crate::RequestMeta {
            workspace: Some(crate::WorkspaceRef {
                root: Some(target_display),
                workspace_id,
            }),
            ..request.meta
        },
        target: crate::MaterializeTarget {
            kind: crate::MaterializeTargetKind::Lock,
            name: None,
            commit: None,
        },
    };
    let manifest = artifact::read_manifest(&target_path)?;
    assert_workspace_id(&manifest, materialize.meta.workspace.as_ref())?;
    let (plans, rewrite_lock) =
        prepare_materialize_execution(backend, &target_path, &manifest, &materialize, scheme)?;
    validate_materialize_identities(backend, &manifest, &plans, &["origin".into()])?;
    // Clone materializes the lock target: detached:false members land on their branch head.
    let mut response = apply_materialize_plans(
        backend,
        &target_path,
        &manifest,
        plans,
        MaterializeApplyOptions {
            rewrite_lock,
            skip_private_access: true,
            follow_branch_head: true,
            policy: materialize.meta.policy.as_ref(),
            url_scheme: scheme.scheme,
        },
        context,
        emitter,
    )?;
    // Remember an explicitly requested scheme in the new workspace, and say when the
    // root itself was reached through a derived URL.
    record_workspace_url_scheme(&target_path, scheme, "clone")?;
    if root_resolution.derived {
        response.response.meta.message = Some(format!(
            "cloned the workspace root from {} (derived from {})",
            root_resolution.effective_url, root_resolution.manifest_url
        ));
    }
    Ok(crate::CloneWorkspaceResponse {
        response: response.response,
    })
}

/// Compatibility wrapper for the Rust CLI command path.
pub fn handle_clone_workspace<B>(
    backend: &B,
    meta: crate::RequestMeta,
    url: &str,
    target: &str,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend + Sync,
{
    let target_path = Path::new(target);
    if !target_path.is_absolute() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "legacy clone target must be an absolute path",
        ));
    }
    let start = target_path.parent().ok_or_else(|| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            "legacy clone target must have an absolute parent directory",
        )
    })?;
    handle_clone_workspace_at(backend, start, meta, url, target, operation_id, events)
}

/// Clone wrapper for a driver that has captured an explicit caller directory.
pub fn handle_clone_workspace_at<B>(
    backend: &B,
    start: &Path,
    meta: crate::RequestMeta,
    url: &str,
    target: &str,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend + Sync,
{
    handle_clone_workspace_request(
        backend,
        start,
        crate::CloneWorkspaceRequest {
            meta,
            url: url.to_owned(),
            target: target.to_owned(),
        },
        operation_id,
        events,
    )
    .map(|response| crate::MaterializeResponse {
        response: response.response,
    })
}
