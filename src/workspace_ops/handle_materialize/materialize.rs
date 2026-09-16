use std::collections::BTreeMap;
use std::path::Path;

use crate::artifact::{self, LockArtifact, ManifestArtifact, ResolvedMemberArtifact};
use crate::git::{GitBackend, MergeAuthorityBackend};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, EventSink, OpenMergeCommand, OperationRequest};

use super::super::*;
use super::apply::{apply_materialize_plans, materialize_clone_remote};
use super::branch::handle_materialize_branch;

pub fn handle_materialize<B>(
    backend: &B,
    start: &Path,
    request: crate::MaterializeRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend + MergeAuthorityBackend + Sync,
{
    let start = invocation_start(start, &request.meta)?;
    let context = OperationRequest::Materialize(request.clone()).context(operation_id.into())?;
    let scoped_backend = backend.with_transport(&start, request.meta.transport.as_ref())?;
    let backend = scoped_backend.as_ref().unwrap_or(backend);
    let services = crate::operation_context::OperationServices::for_merge(backend);
    let error_context = context.clone();
    let result: ModelResult<crate::MaterializeResponse> = (|| {
        let (_guard, root) = guarded_workspace_root_for_request_in(
            &services,
            &start,
            &request.meta,
            OpenMergeCommand::Materialize,
            request.meta.dry_run.unwrap_or(false),
        )?;
        assert_conf_unmodified_for(
            backend,
            &root,
            OpenMergeCommand::Materialize,
            reconcile_authority(_guard.as_ref(), request.meta.dry_run.unwrap_or(false)),
        )?;
        let manifest = artifact::read_manifest(&root)?;
        assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
        if request.target.kind == crate::MaterializeTargetKind::Branch {
            return handle_materialize_branch(backend, root, manifest, request, context);
        }
        let scheme = resolve_url_scheme(Some(&root), requested_url_scheme(&request.meta))?;
        let (plans, rewrite_lock) =
            prepare_materialize_execution(backend, &root, &manifest, &request, scheme)?;
        validate_materialize_identities(backend, &manifest, &plans, &[])?;
        let dry_run = request.meta.dry_run.unwrap_or(false);
        if dry_run {
            return Ok(crate::MaterializeResponse {
                response: response_envelope(
                    context,
                    crate::AggregateStatus::Accepted,
                    plans.into_iter().map(|plan| plan.response).collect(),
                ),
            });
        }

        let progress_interval = request
            .meta
            .policy
            .as_ref()
            .and_then(|policy| policy.progress_min_interval_ms)
            .unwrap_or(0);
        let emitter = EventEmitter::new(&context, events, progress_interval);
        emitter.operation_started();
        // Lock target tracks branch heads for detached:false members; snapshot/tag/head pin.
        let follow_branch_head = request.target.kind == crate::MaterializeTargetKind::Lock;
        let response = apply_materialize_plans(
            backend,
            &root,
            &manifest,
            plans,
            MaterializeApplyOptions {
                rewrite_lock,
                skip_private_access: request.target.kind == crate::MaterializeTargetKind::Lock,
                follow_branch_head,
                policy: request.meta.policy.as_ref(),
                url_scheme: scheme.scheme,
            },
            context,
            &emitter,
        );
        emitter.operation_finished();
        let response = response?;
        // Remember an explicitly requested scheme only once the members are in place.
        record_workspace_url_scheme(&root, scheme, "materialize")?;
        Ok(response)
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

pub(super) fn prepare_materialize_execution<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    request: &crate::MaterializeRequest,
    scheme: EffectiveUrlScheme,
) -> ModelResult<(Vec<MaterializePlan>, bool)>
where
    B: GitBackend,
{
    let (target_members, rewrite_lock) =
        materialize_target_members(backend, root, manifest, &request.target)?;
    let target_lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        members: target_members,
    };
    // A tag covers only the members that carry it. With the default (None) selection, restrict to
    // exactly the tagged subset (the lock we just built) rather than the full manifest — otherwise
    // resolve_locked_selection errors LockNotFound on members that lack the tag. An explicit
    // selection still validates against the tagged set (a selected-but-untagged member errors).
    let selected = match (&request.target.kind, request.meta.selection.as_ref()) {
        (crate::MaterializeTargetKind::Tag, None) => target_lock.members.keys().cloned().collect(),
        _ => resolve_locked_action_selection(
            manifest,
            &target_lock,
            request.meta.selection.as_ref(),
            crate::ActionKind::Materialize,
        )?,
    };
    let destructive_allowed = request
        .meta
        .policy
        .as_ref()
        .and_then(|policy| policy.destructive)
        == Some(crate::DestructiveBehavior::Allow);
    let plans = materialize_preflight(
        backend,
        root,
        manifest,
        &target_lock,
        &selected,
        destructive_allowed,
        scheme,
    )?;
    Ok((plans, rewrite_lock))
}

pub(super) struct MaterializeApplyOptions<'a> {
    pub(super) rewrite_lock: bool,
    pub(super) skip_private_access: bool,
    // True for the lock target (and clone): detached:false members follow their branch
    // head. False for snapshot/tag: pin the recorded commit so the capture is reproducible.
    pub(super) follow_branch_head: bool,
    pub(super) policy: Option<&'a crate::OperationPolicy>,
    /// The scheme in force, for the remedy hints on ssh failures.
    pub(super) url_scheme: crate::git::UrlScheme,
}

pub(super) fn validate_materialize_identities<B: GitBackend>(
    backend: &B,
    manifest: &ManifestArtifact,
    plans: &[MaterializePlan],
    previous_names: &[String],
) -> ModelResult<()> {
    // Identity checks look at the URL that will actually be cloned, so an SSH
    // identity override for a remote that resolves to https is refused up front.
    let remotes = plans
        .iter()
        .filter_map(|plan| {
            plan.clone_url.as_deref().map(|url| {
                materialize_clone_remote(manifest, &plan.member_id)
                    .map(|remote| (remote.name.clone(), url.to_owned()))
            })
        })
        .collect::<ModelResult<Vec<_>>>()?;
    let names = previous_names
        .iter()
        .cloned()
        .chain(remotes.iter().map(|(name, _)| name.clone()))
        .collect::<Vec<_>>();
    backend.validate_transport_remotes(&names)?;
    for (name, url) in remotes {
        backend.validate_url_identity(None, &name, &url)?;
    }
    Ok(())
}
pub fn load_snapshot_target(
    root: &Path,
    snapshot_id: &str,
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    Ok(artifact::read_snapshot(root, snapshot_id)?.members)
}

pub(crate) fn materialize_target_members<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    target: &crate::MaterializeTarget,
) -> ModelResult<(BTreeMap<String, ResolvedMemberArtifact>, bool)> {
    match target.kind {
        crate::MaterializeTargetKind::Lock => {
            if !root.join(artifact::LOCK_PATH).exists() {
                return Err(ModelError::new(ErrorCode::LockNotFound, "lock not found"));
            }
            Ok((artifact::read_lock(root)?.members, false))
        }
        crate::MaterializeTargetKind::Snapshot => {
            let name = target
                .name
                .as_ref()
                .ok_or_else(|| invalid("snapshot target requires a name"))?;
            if !root
                .join(artifact::SNAPSHOT_DIR)
                .join(format!("{name}.yaml"))
                .exists()
            {
                return Err(ModelError::new(
                    ErrorCode::SnapshotNotFound,
                    "snapshot not found",
                ));
            }
            Ok((load_snapshot_target(root, name)?, true))
        }
        crate::MaterializeTargetKind::Tag => {
            // Re-meaned (GWZTagPlan): materialize each member to the commit its git tag
            // `refs/tags/<name>` points at. Members lacking the tag are skipped.
            let name = target
                .name
                .as_ref()
                .ok_or_else(|| invalid("tag target requires a name"))?;
            let tag_ref = format!("refs/tags/{name}^{{commit}}");
            let mut targets = BTreeMap::new();
            for member in manifest.members.iter().filter(|member| member.active) {
                let member_root = root.join(&member.path);
                if !backend.is_repository(&member_root)? {
                    continue;
                }
                if let Some(commit) = backend.read_ref(&member_root, &tag_ref)? {
                    targets.insert(
                        member.id.clone(),
                        ResolvedMemberArtifact {
                            path: member.path.clone(),
                            commit: Some(commit),
                            ..Default::default()
                        },
                    );
                }
            }
            if targets.is_empty() {
                return Err(ModelError::new(
                    ErrorCode::TagNotFound,
                    format!("tag '{name}' not found in any member"),
                ));
            }
            Ok((targets, true))
        }
        crate::MaterializeTargetKind::Commit
        | crate::MaterializeTargetKind::Head
        | crate::MaterializeTargetKind::Branch => Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "target is not supported here",
        )),
    }
}
