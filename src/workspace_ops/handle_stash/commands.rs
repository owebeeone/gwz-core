use super::*;

pub(super) fn handle_stash_push<B>(
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

    // CAPABILITY-FREE EXCEPTION, §10 row `:276`: every `stash::write_bundle` here runs under `guarded_workspace_root(StashMutate)` and stays raw permanently (2026-09-02, GwzM5-8R2E-CapabilityFreeAmendment.md §3).
    stash::write_bundle(&root, &bundle)?;

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

pub(super) fn handle_stash_list<B>(
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
    let selected = resolve_stash_selection(&manifest, lock, request.meta.selection.as_ref())?;
    let selected_set = selected.iter().cloned().collect::<BTreeSet<_>>();
    let include_root = request.meta.selection.is_none() || selected_set.contains("@root");
    let selected_members = selected_set
        .iter()
        .filter(|target_id| target_id.as_str() != "@root")
        .cloned()
        .collect::<BTreeSet<_>>();
    let scope = StashListScope {
        selected: &selected_members,
        include_root,
    };
    let mut bundles = stash::list_bundles(&root)?;
    for bundle in &mut bundles {
        reconcile_bundle(
            backend,
            &root,
            &manifest,
            &selected_members,
            include_root,
            bundle,
        )?;
    }
    let bundle_targets = bundles
        .iter()
        .flat_map(|bundle| {
            bundle
                .members
                .iter()
                .map(|member| (bundle.stash_id.clone(), member.member_id.clone()))
        })
        .collect::<BTreeSet<_>>();
    append_orphan_warnings(
        backend,
        &root,
        &manifest,
        &manifest.workspace.id,
        scope,
        &bundle_targets,
        &mut bundles,
    )?;
    Ok(stash_response(
        context,
        crate::AggregateStatus::Ok,
        Vec::new(),
        bundles.iter().map(project_bundle).collect(),
    ))
}

pub(super) fn handle_stash_restore<B>(
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
    let mut bundle = resolve_requested_bundle(&root, request.stash_id.as_deref(), request.op)?;
    let selected = if explicit_selection {
        resolve_stash_selection(&manifest, lock, request.meta.selection.as_ref())?
    } else {
        eligible_bundle_members(&bundle, request.op)
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
            if restore_state_eligible(request.op, member.restore_state)
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
    let plans = restore_plans(
        backend,
        &root,
        &manifest,
        &bundle,
        &selected_set,
        request.op,
    )?;

    // Everything above is validation and planning: the bundle is resolved, the
    // selection is checked against it, and `restore_plans` has already refused a
    // dirty destination or a missing native payload. A dry run stops here, before
    // the first `stash_apply`/`stash_pop`/`stash_drop` and before any bundle
    // rewrite (DR-1/DR-2).
    if request.meta.dry_run.unwrap_or(false) {
        return Ok(stash_response(
            context,
            crate::AggregateStatus::Accepted,
            plans
                .iter()
                .map(|plan| {
                    stash_target_response(
                        &plan.target_id,
                        &plan.relative_path,
                        plan.target_kind,
                        crate::MemberStatus::Planned,
                        None,
                    )
                })
                .collect(),
            vec![project_bundle(&bundle)],
        ));
    }

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
            crate::StashOp::Drop if !plan.native_present => Ok(()),
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
                responses.push(stash_target_response(
                    &plan.target_id,
                    &plan.relative_path,
                    plan.target_kind,
                    crate::MemberStatus::Ok,
                    None,
                ));
            }
            Err(error) => {
                let mapped = restore_target_error(error, &plan.target_id, &plan.relative_path);
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

pub(super) struct StashMemberPlan<'a> {
    pub(super) member: &'a ManifestMember,
    pub(super) root: PathBuf,
    pub(super) status: GitStatus,
    pub(super) branch: Option<String>,
    pub(super) head: Option<String>,
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
        let status =
            backend.status_with_options(&member_root, GitStatusOptions { include_ignored })?;
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

fn restore_plans<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    bundle: &StashBundle,
    selected: &BTreeSet<String>,
    op: crate::StashOp,
) -> ModelResult<Vec<RestorePlan>> {
    let mut plans = Vec::new();
    for member in &bundle.members {
        if !selected.contains(&member.member_id)
            || member.push_lifecycle != StashPushLifecycle::Saved
            || !restore_state_eligible(op, member.restore_state)
        {
            continue;
        }
        let (target_id, relative_path, member_root, target_kind) = if member.member_id == "@root" {
            if member.path != "." {
                return Err(ModelError::new(
                    ErrorCode::StashIncomplete,
                    "root preservation stash path must be '.'",
                )
                .with_member("@root", "."));
            }
            (
                "@root".to_owned(),
                ".".to_owned(),
                root.to_path_buf(),
                crate::TargetKind::Root,
            )
        } else {
            let manifest_member = manifest_member(manifest, &member.member_id)?;
            ensure_git_member(manifest_member)?;
            (
                manifest_member.id.clone(),
                manifest_member.path.clone(),
                root.join(&manifest_member.path),
                crate::TargetKind::Member,
            )
        };
        if !backend.is_repository(&member_root)? {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("stash target '{target_id}' is not a repository"),
            )
            .with_member(&target_id, &relative_path));
        }
        let status = backend.status(&member_root)?;
        if op != crate::StashOp::Drop && status.is_dirty {
            return Err(ModelError::new(
                ErrorCode::DirtyMember,
                format!(
                    "target '{target_id}' has local changes; stash restore requires a clean destination"
                ),
            )
            .with_member(&target_id, &relative_path));
        }
        let target = stash_target(&bundle.stash_id, member);
        let native_present = native_stash_exists(backend, &member_root, &target)?;
        if !native_present && op != crate::StashOp::Drop {
            return Err(ModelError::new(
                ErrorCode::StashIncomplete,
                format!(
                    "native stash payload missing for member '{}'",
                    member.member_id
                ),
            )
            .with_member(&target_id, &relative_path));
        }
        plans.push(RestorePlan {
            target_id,
            relative_path,
            target_kind,
            root: member_root,
            bundle_member: member.clone(),
            native_present,
        });
    }
    Ok(plans)
}

struct RestorePlan {
    target_id: String,
    relative_path: String,
    target_kind: crate::TargetKind,
    root: PathBuf,
    bundle_member: StashBundleMember,
    native_present: bool,
}

pub(super) fn restore_state_eligible(op: crate::StashOp, state: StashRestoreState) -> bool {
    match op {
        crate::StashOp::Apply | crate::StashOp::Pop => state == StashRestoreState::Pending,
        crate::StashOp::Drop => matches!(
            state,
            StashRestoreState::Pending | StashRestoreState::Applied | StashRestoreState::Missing
        ),
        crate::StashOp::Push | crate::StashOp::List => false,
    }
}
