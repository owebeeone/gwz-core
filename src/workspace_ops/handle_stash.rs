use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::artifact::{self, ArtifactSourceKind, ManifestArtifact, ManifestMember};
use crate::git::{
    GitBackend, GitStashPushOptions, GitStashRestoreOptions, GitStashTarget, GitStatus,
    GitStatusOptions,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OperationRequest, WorkspaceMutatorLock};
use crate::stash::{
    self, STASH_BUNDLE_SCHEMA, StashBundle, StashBundleMember, StashDirtySummary, StashDrift,
    StashErrorDetail, StashParticipation, StashPushLifecycle, StashRestoreState, StashWarning,
};

use super::*;

pub fn handle_stash<B>(
    backend: &B,
    start: &Path,
    request: crate::StashRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::StashResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Stash(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;

    match request.op {
        crate::StashOp::Push => handle_stash_push(backend, root, manifest, request, context),
        crate::StashOp::List => handle_stash_list(backend, root, manifest, &lock, request, context),
        crate::StashOp::Apply | crate::StashOp::Pop | crate::StashOp::Drop => {
            handle_stash_restore(backend, root, manifest, &lock, request, context)
        }
    }
}

fn handle_stash_push<B>(
    backend: &B,
    root: PathBuf,
    manifest: ManifestArtifact,
    request: crate::StashRequest,
    context: crate::operation::OperationContext,
) -> ModelResult<crate::StashResponse>
where
    B: GitBackend,
{
    let selected = resolve_locked_selection(
        &manifest,
        &artifact::read_lock(&root)?,
        request.meta.selection.as_ref(),
    )?;
    let include_ignored = request.include_ignored.unwrap_or(false);
    let include_untracked = request.include_untracked.unwrap_or(false) || include_ignored;
    let plans = stash_member_plans(backend, &root, &manifest, &selected, include_ignored)?;
    let stash_id = request
        .stash_id
        .clone()
        .map(validate_stash_id)
        .transpose()?
        .unwrap_or_else(generate_stash_id);
    if stash::bundle_path(&root, &stash_id).exists() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("stash bundle '{stash_id}' already exists"),
        ));
    }

    let message_suffix = request
        .message
        .clone()
        .unwrap_or_else(|| "workspace stash".to_owned());
    let prefix = stash_prefix(&stash_id);
    let full_message = format!("{prefix} {message_suffix}");
    let created_at = now_marker();
    let mut bundle = StashBundle {
        schema: STASH_BUNDLE_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        stash_id: stash_id.clone(),
        created_at,
        message_suffix,
        include_untracked,
        include_ignored,
        selected_members: selected.clone(),
        members: plans
            .iter()
            .map(|plan| initial_bundle_member(plan, &full_message))
            .collect(),
        warnings: Vec::new(),
        drift: Vec::new(),
    };

    if request.meta.dry_run.unwrap_or(false) {
        return Ok(stash_response(
            context,
            crate::AggregateStatus::Accepted,
            planned_member_responses(&plans),
            vec![project_bundle(&bundle)],
        ));
    }

    stash::write_bundle(&root, &bundle)?;
    let _guard = WorkspaceMutatorLock::try_acquire(&root)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::UnsupportedOperation,
            "workspace mutator lock is already held",
        )
    })?;

    let options = GitStashPushOptions {
        include_untracked,
        include_ignored,
        preserve_index: request.preserve_index.unwrap_or(false),
    };
    let mut responses = Vec::with_capacity(plans.len());
    for (index, plan) in plans.iter().enumerate() {
        if !stash_push_needed(&plan.status, include_untracked, include_ignored) {
            bundle.members[index].participation = StashParticipation::Empty;
            bundle.members[index].push_lifecycle = StashPushLifecycle::Empty;
            bundle.members[index].restore_state = StashRestoreState::Noop;
            responses.push(stash_member_response(
                plan.member,
                crate::MemberStatus::Noop,
                None,
            ));
            stash::write_bundle(&root, &bundle)?;
            continue;
        }

        bundle.members[index].participation = StashParticipation::Stashed;
        bundle.members[index].restore_state = StashRestoreState::Missing;
        bundle.members[index].push_lifecycle = StashPushLifecycle::Saving;
        stash::write_bundle(&root, &bundle)?;
        match backend.stash_push(&plan.root, &full_message, options) {
            Ok(result) => {
                let native = find_native_stash(backend, &plan.root, &result.object_id, &prefix)?;
                bundle.members[index].push_lifecycle = StashPushLifecycle::Saved;
                bundle.members[index].restore_state = StashRestoreState::Pending;
                bundle.members[index].native_stash_object_id = Some(result.object_id);
                bundle.members[index].native_stash_display_ref =
                    Some(format!("stash@{{{}}}", native.index));
                responses.push(stash_member_response(
                    plan.member,
                    crate::MemberStatus::Ok,
                    None,
                ));
            }
            Err(error) => {
                bundle.members[index].push_lifecycle = StashPushLifecycle::Failed;
                bundle.members[index].restore_state = StashRestoreState::Missing;
                bundle.members[index].error = Some(StashErrorDetail {
                    code: format!("{:?}", error.code),
                    message: error.message.clone(),
                });
                responses.push(stash_member_response(
                    plan.member,
                    crate::MemberStatus::Failed,
                    Some(error),
                ));
            }
        }
        stash::write_bundle(&root, &bundle)?;
    }

    Ok(stash_response(
        context,
        aggregate_from_members(&responses),
        responses,
        vec![project_bundle(&bundle)],
    ))
}

fn handle_stash_list<B>(
    backend: &B,
    root: PathBuf,
    manifest: ManifestArtifact,
    lock: &artifact::LockArtifact,
    request: crate::StashRequest,
    context: crate::operation::OperationContext,
) -> ModelResult<crate::StashResponse>
where
    B: GitBackend,
{
    let selected = resolve_locked_selection(&manifest, lock, request.meta.selection.as_ref())?;
    let selected_set = selected.iter().cloned().collect::<BTreeSet<_>>();
    let mut bundles = stash::list_bundles(&root)?;
    for bundle in &mut bundles {
        reconcile_bundle(backend, &root, &manifest, &selected_set, bundle)?;
    }
    let bundle_ids = bundles
        .iter()
        .map(|bundle| bundle.stash_id.clone())
        .collect::<BTreeSet<_>>();
    append_orphan_warnings(
        backend,
        &root,
        &manifest,
        &manifest.workspace.id,
        &selected_set,
        &bundle_ids,
        &mut bundles,
    )?;
    Ok(stash_response(
        context,
        crate::AggregateStatus::Ok,
        Vec::new(),
        bundles.iter().map(project_bundle).collect(),
    ))
}

fn handle_stash_restore<B>(
    backend: &B,
    root: PathBuf,
    manifest: ManifestArtifact,
    lock: &artifact::LockArtifact,
    request: crate::StashRequest,
    context: crate::operation::OperationContext,
) -> ModelResult<crate::StashResponse>
where
    B: GitBackend,
{
    let explicit_selection = request.meta.selection.is_some();
    let mut bundle = resolve_requested_bundle(&root, request.stash_id.as_deref())?;
    let selected = if explicit_selection {
        resolve_locked_selection(&manifest, lock, request.meta.selection.as_ref())?
    } else {
        eligible_bundle_members(&bundle)
    };
    if selected.is_empty() {
        return Err(ModelError::new(
            ErrorCode::StashIncomplete,
            "stash bundle has no eligible members for this operation",
        ));
    }
    if explicit_selection {
        ensure_selected_in_bundle(&bundle, &selected)?;
    } else {
        let selected_set = selected.iter().cloned().collect::<BTreeSet<_>>();
        for member in &bundle.members {
            if member.restore_state == StashRestoreState::Pending
                && !selected_set.contains(&member.member_id)
            {
                return Err(ModelError::new(
                    ErrorCode::StashIncomplete,
                    "partial stash restore requires an explicit member selection",
                ));
            }
        }
    }

    let selected_set = selected.iter().cloned().collect::<BTreeSet<_>>();
    let plans = restore_plans(backend, &root, &manifest, &bundle, &selected_set)?;
    let _guard = WorkspaceMutatorLock::try_acquire(&root)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::UnsupportedOperation,
            "workspace mutator lock is already held",
        )
    })?;

    let mut responses = Vec::with_capacity(plans.len());
    let preserve_index = request.preserve_index.unwrap_or(true);
    for plan in plans {
        let target = stash_target(&bundle.stash_id, &plan.bundle_member);
        let result = match request.op {
            crate::StashOp::Apply => backend.stash_apply(
                &plan.root,
                &target,
                GitStashRestoreOptions { preserve_index },
            ),
            crate::StashOp::Pop => backend.stash_pop(
                &plan.root,
                &target,
                GitStashRestoreOptions { preserve_index },
            ),
            crate::StashOp::Drop => backend.stash_drop(&plan.root, &target),
            crate::StashOp::Push | crate::StashOp::List => unreachable!(),
        };
        let member_index = bundle
            .members
            .iter()
            .position(|member| member.member_id == plan.bundle_member.member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::InternalError, "bundle member missing"))?;
        match result {
            Ok(()) => {
                bundle.members[member_index].restore_state = match request.op {
                    crate::StashOp::Apply => StashRestoreState::Applied,
                    crate::StashOp::Pop => StashRestoreState::Popped,
                    crate::StashOp::Drop => StashRestoreState::Dropped,
                    crate::StashOp::Push | crate::StashOp::List => unreachable!(),
                };
                bundle.members[member_index].native_stash_display_ref = None;
                responses.push(stash_member_response(
                    plan.member,
                    crate::MemberStatus::Ok,
                    None,
                ));
            }
            Err(error) => {
                let mapped = restore_error(error);
                if mapped.code == ErrorCode::StashIncomplete {
                    bundle.members[member_index].restore_state = StashRestoreState::Missing;
                }
                bundle.members[member_index].error = Some(StashErrorDetail {
                    code: format!("{:?}", mapped.code),
                    message: mapped.message.clone(),
                });
                stash::write_bundle(&root, &bundle)?;
                return Err(mapped);
            }
        }
    }

    if bundle_is_complete(&bundle) {
        fs::remove_file(stash::bundle_path(&root, &bundle.stash_id)).map_err(io_error)?;
    } else {
        stash::write_bundle(&root, &bundle)?;
    }

    Ok(stash_response(
        context,
        aggregate_from_members(&responses),
        responses,
        vec![project_bundle(&bundle)],
    ))
}

struct StashMemberPlan<'a> {
    member: &'a ManifestMember,
    root: PathBuf,
    status: GitStatus,
    branch: Option<String>,
    head: Option<String>,
}

fn stash_member_plans<'a, B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &'a ManifestArtifact,
    selected: &[String],
    include_ignored: bool,
) -> ModelResult<Vec<StashMemberPlan<'a>>> {
    let mut plans = Vec::with_capacity(selected.len());
    for member_id in selected {
        let member = manifest_member(manifest, member_id)?;
        ensure_git_member(member)?;
        let member_root = root.join(&member.path);
        if !backend.is_repository(&member_root)? {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("selected member '{}' is not materialized", member.id),
            ));
        }
        let head = backend.head(&member_root)?;
        let status = backend.status_with_options(
            &member_root,
            GitStatusOptions { include_ignored },
        )?;
        plans.push(StashMemberPlan {
            member,
            root: member_root,
            status,
            branch: head.branch,
            head: head.commit,
        });
    }
    Ok(plans)
}

fn restore_plans<'a, B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &'a ManifestArtifact,
    bundle: &StashBundle,
    selected: &BTreeSet<String>,
) -> ModelResult<Vec<RestorePlan<'a>>> {
    let mut plans = Vec::new();
    for member in &bundle.members {
        if !selected.contains(&member.member_id)
            || member.restore_state != StashRestoreState::Pending
        {
            continue;
        }
        let manifest_member = manifest_member(manifest, &member.member_id)?;
        ensure_git_member(manifest_member)?;
        let member_root = root.join(&manifest_member.path);
        let status = backend.status(&member_root)?;
        if status.is_dirty {
            return Err(ModelError::new(
                ErrorCode::DirtyMember,
                format!(
                    "member '{}' has local changes; stash restore requires a clean destination",
                    manifest_member.id
                ),
            ));
        }
        let target = stash_target(&bundle.stash_id, member);
        if !native_stash_exists(backend, &member_root, &target)? {
            return Err(ModelError::new(
                ErrorCode::StashIncomplete,
                format!(
                    "native stash payload missing for member '{}'",
                    member.member_id
                ),
            ));
        }
        plans.push(RestorePlan {
            member: manifest_member,
            root: member_root,
            bundle_member: member.clone(),
        });
    }
    Ok(plans)
}

struct RestorePlan<'a> {
    member: &'a ManifestMember,
    root: PathBuf,
    bundle_member: StashBundleMember,
}

fn initial_bundle_member(plan: &StashMemberPlan<'_>, full_message: &str) -> StashBundleMember {
    let dirty = dirty_summary(&plan.status);
    let empty = !plan.status.is_dirty;
    StashBundleMember {
        member_id: plan.member.id.clone(),
        path: plan.member.path.clone(),
        participation: if empty {
            StashParticipation::Empty
        } else {
            StashParticipation::Stashed
        },
        push_lifecycle: if empty {
            StashPushLifecycle::Empty
        } else {
            StashPushLifecycle::Unattempted
        },
        restore_state: if empty {
            StashRestoreState::Noop
        } else {
            StashRestoreState::Missing
        },
        branch_before: plan.branch.clone(),
        head_before: plan.head.clone(),
        full_stash_message: full_message.to_owned(),
        dirty_summary: dirty,
        native_stash_object_id: None,
        native_stash_display_ref: None,
        error: None,
    }
}

fn dirty_summary(status: &GitStatus) -> StashDirtySummary {
    StashDirtySummary {
        staged: status.staged > 0,
        unstaged: status.unstaged > 0,
        untracked: status.untracked > 0,
        ignored: status
            .files
            .iter()
            .any(|file| file.index_status == "!" || file.worktree_status == "!"),
    }
}

fn stash_push_needed(status: &GitStatus, include_untracked: bool, include_ignored: bool) -> bool {
    status.staged > 0
        || status.unstaged > 0
        || (include_untracked && status.untracked > 0)
        || (include_ignored
            && status
                .files
                .iter()
                .any(|file| file.index_status == "!" || file.worktree_status == "!"))
}

fn reconcile_bundle<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    selected: &BTreeSet<String>,
    bundle: &mut StashBundle,
) -> ModelResult<()> {
    for member in &mut bundle.members {
        if !selected.contains(&member.member_id)
            || member.push_lifecycle != StashPushLifecycle::Saved
            || !matches!(
                member.restore_state,
                StashRestoreState::Pending | StashRestoreState::Applied
            )
        {
            continue;
        }
        let manifest_member = manifest_member(manifest, &member.member_id)?;
        let member_root = root.join(&manifest_member.path);
        if !native_stash_exists(
            backend,
            &member_root,
            &stash_target(&bundle.stash_id, member),
        )? {
            bundle.drift.push(StashDrift {
                code: "missing_native_stash".to_owned(),
                message: "registered stash payload is missing from native Git stash list"
                    .to_owned(),
                member_id: member.member_id.clone(),
            });
            member.restore_state = StashRestoreState::Missing;
        }
    }
    Ok(())
}

fn append_orphan_warnings<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    workspace_id: &str,
    selected: &BTreeSet<String>,
    known_bundle_ids: &BTreeSet<String>,
    bundles: &mut Vec<StashBundle>,
) -> ModelResult<()> {
    let mut orphan_warnings = Vec::new();
    for member_id in selected {
        let member = manifest_member(manifest, member_id)?;
        let member_root = root.join(&member.path);
        if !backend.is_repository(&member_root)? {
            continue;
        }
        for native in backend.stash_list(&member_root)? {
            if let Some(stash_id) = native_gwz_stash_id(&native.message)
                && !known_bundle_ids.contains(&stash_id)
            {
                orphan_warnings.push((
                    stash_id.clone(),
                    StashWarning {
                        code: "orphan_native_stash".to_owned(),
                        message: format!(
                            "native GWZ stash '{stash_id}' has no local bundle metadata"
                        ),
                        member_id: Some(member.id.clone()),
                    },
                ));
            }
        }
    }
    for (stash_id, warning) in orphan_warnings {
        if let Some(bundle) = bundles.iter_mut().find(|bundle| {
            bundle.stash_id == stash_id && bundle.members.is_empty()
        }) {
            bundle.warnings.push(warning);
        } else {
            bundles.push(orphan_warning_bundle(workspace_id, stash_id, warning));
        }
    }
    Ok(())
}

fn orphan_warning_bundle(
    workspace_id: &str,
    stash_id: String,
    warning: StashWarning,
) -> StashBundle {
    StashBundle {
        schema: STASH_BUNDLE_SCHEMA.to_owned(),
        workspace_id: workspace_id.to_owned(),
        stash_id,
        created_at: "unknown".to_owned(),
        message_suffix: "orphan native stash".to_owned(),
        include_untracked: false,
        include_ignored: false,
        selected_members: Vec::new(),
        members: Vec::new(),
        warnings: vec![warning],
        drift: Vec::new(),
    }
}

fn resolve_requested_bundle(root: &Path, requested: Option<&str>) -> ModelResult<StashBundle> {
    match requested {
        Some(stash_id) => stash::read_bundle(root, stash_id),
        None => stash::list_bundles(root)?
            .into_iter()
            .find(|bundle| !eligible_bundle_members(bundle).is_empty())
            .ok_or_else(|| ModelError::new(ErrorCode::StashNotFound, "no eligible stash bundle")),
    }
}

fn eligible_bundle_members(bundle: &StashBundle) -> Vec<String> {
    bundle
        .members
        .iter()
        .filter(|member| {
            member.push_lifecycle == StashPushLifecycle::Saved
                && member.restore_state == StashRestoreState::Pending
        })
        .map(|member| member.member_id.clone())
        .collect()
}

fn ensure_selected_in_bundle(bundle: &StashBundle, selected: &[String]) -> ModelResult<()> {
    let members = bundle
        .members
        .iter()
        .map(|member| member.member_id.as_str())
        .collect::<BTreeSet<_>>();
    for member_id in selected {
        if !members.contains(member_id.as_str()) {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "member '{member_id}' is not part of stash '{}'",
                    bundle.stash_id
                ),
            ));
        }
    }
    Ok(())
}

fn bundle_is_complete(bundle: &StashBundle) -> bool {
    bundle.members.iter().all(|member| {
        matches!(
            member.restore_state,
            StashRestoreState::Noop | StashRestoreState::Popped | StashRestoreState::Dropped
        ) && matches!(
            member.push_lifecycle,
            StashPushLifecycle::Saved | StashPushLifecycle::Empty
        )
    })
}

fn native_stash_exists<B: GitBackend>(
    backend: &B,
    root: &Path,
    target: &GitStashTarget,
) -> ModelResult<bool> {
    let entries = backend.stash_list(root)?;
    Ok(entries.iter().any(|entry| {
        target
            .object_id
            .as_ref()
            .is_some_and(|object_id| &entry.object_id == object_id)
            || target.gwz_message_prefix.as_ref().is_some_and(|prefix| {
                entry.message.starts_with(prefix)
                    || entry
                        .message
                        .split_once(": ")
                        .is_some_and(|(_, suffix)| suffix.starts_with(prefix))
            })
    }))
}

fn find_native_stash<B: GitBackend>(
    backend: &B,
    root: &Path,
    object_id: &str,
    prefix: &str,
) -> ModelResult<crate::git::GitStashEntry> {
    backend
        .stash_list(root)?
        .into_iter()
        .find(|entry| entry.object_id == object_id || entry.message.starts_with(prefix))
        .ok_or_else(|| ModelError::new(ErrorCode::StashIncomplete, "saved stash was not listed"))
}

fn stash_target(stash_id: &str, member: &StashBundleMember) -> GitStashTarget {
    GitStashTarget {
        object_id: member.native_stash_object_id.clone(),
        gwz_message_prefix: Some(stash_prefix(stash_id)),
    }
}

fn stash_prefix(stash_id: &str) -> String {
    format!("gwz:{stash_id}:")
}

fn native_gwz_stash_id(message: &str) -> Option<String> {
    let after_marker = message.split_once("gwz:")?.1.split_once(':')?.0.to_owned();
    if after_marker.starts_with("stash_") {
        Some(after_marker)
    } else {
        None
    }
}

fn validate_stash_id(stash_id: String) -> ModelResult<String> {
    let valid = stash_id.starts_with("stash_")
        && stash_id.len() > "stash_".len()
        && stash_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    if !valid {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "stash_id must start with stash_ and contain only portable characters",
        ));
    }
    Ok(stash_id)
}

fn generate_stash_id() -> String {
    format!("stash_{}", now_marker().replace([':', '-'], "_"))
}

fn manifest_member<'a>(
    manifest: &'a ManifestArtifact,
    member_id: &str,
) -> ModelResult<&'a ManifestMember> {
    manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))
}

fn ensure_git_member(member: &ManifestMember) -> ModelResult<()> {
    if member.source_kind == ArtifactSourceKind::Git {
        Ok(())
    } else {
        Err(ModelError::new(
            ErrorCode::UnsupportedSourceKind,
            format!(
                "stash supports only git members; '{}' is not git",
                member.id
            ),
        ))
    }
}

fn restore_error(error: ModelError) -> ModelError {
    match error.code {
        ErrorCode::StashConflict => error,
        ErrorCode::GitCommandFailed if error.message.contains("conflict") => {
            ModelError::new(ErrorCode::StashConflict, error.message)
        }
        ErrorCode::GitCommandFailed if error.message.contains("stash entry not found") => {
            ModelError::new(ErrorCode::StashIncomplete, error.message)
        }
        _ => error,
    }
}

fn stash_member_response(
    member: &ManifestMember,
    status: crate::MemberStatus,
    error: Option<ModelError>,
) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: member.path.clone(),
        source_kind: artifact_source_kind_to_protocol(member.source_kind),
        status,
        error: error.map(|error| crate::GwzError {
            code: error.code.into(),
            message: error.message,
            member_id: Some(member.id.clone()),
            member_path: Some(member.path.clone()),
            detail: None,
        }),
        planned: None,
        state: None,
        git_status: None,
        lock_match: None,
    }
}

fn planned_member_responses(plans: &[StashMemberPlan<'_>]) -> Vec<crate::MemberResponse> {
    plans
        .iter()
        .map(|plan| stash_member_response(plan.member, crate::MemberStatus::Planned, None))
        .collect()
}

fn aggregate_from_members(responses: &[crate::MemberResponse]) -> crate::AggregateStatus {
    if responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Failed)
    {
        if responses
            .iter()
            .any(|response| response.status == crate::MemberStatus::Ok)
        {
            crate::AggregateStatus::Partial
        } else {
            crate::AggregateStatus::Failed
        }
    } else {
        crate::AggregateStatus::Ok
    }
}

fn stash_response(
    context: crate::operation::OperationContext,
    status: crate::AggregateStatus,
    members: Vec<crate::MemberResponse>,
    bundles: Vec<crate::StashBundle>,
) -> crate::StashResponse {
    crate::StashResponse {
        response: response_envelope(context, status, members),
        bundles: Some(bundles),
    }
}

fn project_bundle(bundle: &StashBundle) -> crate::StashBundle {
    crate::StashBundle {
        schema: bundle.schema.clone(),
        workspace_id: bundle.workspace_id.clone(),
        stash_id: bundle.stash_id.clone(),
        created_at: bundle.created_at.clone(),
        message_suffix: bundle.message_suffix.clone(),
        include_untracked: bundle.include_untracked,
        include_ignored: bundle.include_ignored,
        members: bundle.members.iter().map(project_member).collect(),
        warnings: bundle.warnings.iter().map(project_warning).collect(),
        drift: bundle.drift.iter().map(project_drift).collect(),
        selected_members: bundle.selected_members.clone(),
    }
}

fn project_member(member: &StashBundleMember) -> crate::StashBundleMember {
    crate::StashBundleMember {
        member_id: member.member_id.clone(),
        path: member.path.clone(),
        participation: project_participation(member.participation),
        push_lifecycle: project_push_lifecycle(member.push_lifecycle),
        restore_state: project_restore_state(member.restore_state),
        branch_before: member.branch_before.clone(),
        head_before: member.head_before.clone(),
        full_stash_message: member.full_stash_message.clone(),
        dirty_summary: crate::StashDirtySummary {
            staged: member.dirty_summary.staged,
            unstaged: member.dirty_summary.unstaged,
            untracked: member.dirty_summary.untracked,
            ignored: member.dirty_summary.ignored,
        },
        native_stash_object_id: member.native_stash_object_id.clone(),
        native_stash_display_ref: member.native_stash_display_ref.clone(),
        error: member.error.as_ref().map(|error| crate::StashErrorDetail {
            code: error.code.clone(),
            message: error.message.clone(),
        }),
    }
}

fn project_warning(warning: &StashWarning) -> crate::StashWarning {
    crate::StashWarning {
        code: warning.code.clone(),
        message: warning.message.clone(),
        member_id: warning.member_id.clone(),
    }
}

fn project_drift(drift: &StashDrift) -> crate::StashDrift {
    crate::StashDrift {
        code: drift.code.clone(),
        message: drift.message.clone(),
        member_id: drift.member_id.clone(),
    }
}

fn project_participation(value: StashParticipation) -> crate::StashParticipation {
    match value {
        StashParticipation::Stashed => crate::StashParticipation::Stashed,
        StashParticipation::Empty => crate::StashParticipation::Empty,
        StashParticipation::Skipped => crate::StashParticipation::Skipped,
    }
}

fn project_push_lifecycle(value: StashPushLifecycle) -> crate::StashPushLifecycle {
    match value {
        StashPushLifecycle::Unattempted => crate::StashPushLifecycle::Unattempted,
        StashPushLifecycle::Saving => crate::StashPushLifecycle::Saving,
        StashPushLifecycle::Saved => crate::StashPushLifecycle::Saved,
        StashPushLifecycle::Empty => crate::StashPushLifecycle::Empty,
        StashPushLifecycle::Failed => crate::StashPushLifecycle::Failed,
    }
}

fn project_restore_state(value: StashRestoreState) -> crate::StashRestoreState {
    match value {
        StashRestoreState::Pending => crate::StashRestoreState::Pending,
        StashRestoreState::Applied => crate::StashRestoreState::Applied,
        StashRestoreState::Popped => crate::StashRestoreState::Popped,
        StashRestoreState::Dropped => crate::StashRestoreState::Dropped,
        StashRestoreState::Noop => crate::StashRestoreState::Noop,
        StashRestoreState::Missing => crate::StashRestoreState::Missing,
    }
}
