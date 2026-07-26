use super::*;

#[derive(Clone, Copy)]
pub(super) struct StashListScope<'a> {
    pub(super) selected: &'a BTreeSet<String>,
    pub(super) include_root: bool,
}

pub(super) fn reconcile_bundle<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    selected: &BTreeSet<String>,
    include_root: bool,
    bundle: &mut StashBundle,
) -> ModelResult<()> {
    for member in &mut bundle.members {
        if !(selected.contains(&member.member_id) || include_root && member.member_id == "@root")
            || member.push_lifecycle != StashPushLifecycle::Saved
            || !matches!(
                member.restore_state,
                StashRestoreState::Pending | StashRestoreState::Applied
            )
        {
            continue;
        }
        let member_root = if member.member_id == "@root" {
            if member.path != "." {
                return Err(ModelError::new(
                    ErrorCode::StashIncomplete,
                    "root preservation stash path must be '.'",
                )
                .with_member("@root", "."));
            }
            root.to_path_buf()
        } else {
            let manifest_member = manifest_member(manifest, &member.member_id)?;
            root.join(&manifest_member.path)
        };
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

pub(super) fn append_orphan_warnings<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    workspace_id: &str,
    scope: StashListScope<'_>,
    known_bundle_targets: &BTreeSet<(String, String)>,
    bundles: &mut Vec<StashBundle>,
) -> ModelResult<()> {
    let mut orphan_warnings = Vec::new();
    for member_id in scope.selected {
        let member = manifest_member(manifest, member_id)?;
        let member_root = root.join(&member.path);
        if !backend.is_repository(&member_root)? {
            continue;
        }
        for native in backend.stash_list(&member_root)? {
            if let Some(stash_id) = native_gwz_stash_id(&native.message)
                && !known_bundle_targets.contains(&(stash_id.clone(), member.id.clone()))
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
    if scope.include_root && backend.is_repository(root)? {
        for native in backend.stash_list(root)? {
            if let Some(stash_id) = native_gwz_stash_id(&native.message)
                && !known_bundle_targets.contains(&(stash_id.clone(), "@root".to_owned()))
            {
                orphan_warnings.push((
                    stash_id.clone(),
                    StashWarning {
                        code: "orphan_native_stash".to_owned(),
                        message: format!(
                            "native GWZ stash '{stash_id}' has no local bundle metadata"
                        ),
                        member_id: Some("@root".to_owned()),
                    },
                ));
            }
        }
    }
    for (stash_id, warning) in orphan_warnings {
        if let Some(bundle) = bundles
            .iter_mut()
            .find(|bundle| bundle.stash_id == stash_id)
        {
            if !bundle.warnings.iter().any(|existing| {
                existing.code == warning.code && existing.member_id == warning.member_id
            }) {
                bundle.warnings.push(warning);
            }
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

pub(super) fn resolve_requested_bundle(
    root: &Path,
    requested: Option<&str>,
    op: crate::StashOp,
) -> ModelResult<StashBundle> {
    match requested {
        Some(stash_id) => stash::read_bundle(root, stash_id),
        None => stash::list_bundles(root)?
            .into_iter()
            .find(|bundle| !eligible_bundle_members(bundle, op).is_empty())
            .ok_or_else(|| ModelError::new(ErrorCode::StashNotFound, "no eligible stash bundle")),
    }
}

pub(super) fn eligible_bundle_members(bundle: &StashBundle, op: crate::StashOp) -> Vec<String> {
    bundle
        .members
        .iter()
        .filter(|member| {
            member.push_lifecycle == StashPushLifecycle::Saved
                && restore_state_eligible(op, member.restore_state)
        })
        .map(|member| member.member_id.clone())
        .collect()
}

pub(super) fn ensure_selected_in_bundle(
    bundle: &StashBundle,
    selected: &[String],
) -> ModelResult<()> {
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

pub(super) fn bundle_is_complete(bundle: &StashBundle) -> bool {
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

pub(super) fn native_stash_exists<B: GitBackend>(
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

pub(super) fn find_native_stash<B: GitBackend>(
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

pub(super) fn stash_target(stash_id: &str, member: &StashBundleMember) -> GitStashTarget {
    GitStashTarget {
        object_id: member.native_stash_object_id.clone(),
        gwz_message_prefix: Some(stash_prefix(stash_id)),
    }
}

pub(super) fn stash_prefix(stash_id: &str) -> String {
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

pub(super) fn validate_stash_id(stash_id: String) -> ModelResult<String> {
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

pub(super) fn generate_stash_id() -> String {
    format!("stash_{}", now_marker().replace([':', '-'], "_"))
}

pub(super) fn manifest_member<'a>(
    manifest: &'a ManifestArtifact,
    member_id: &str,
) -> ModelResult<&'a ManifestMember> {
    manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))
}

pub(super) fn resolve_stash_selection(
    manifest: &ManifestArtifact,
    lock: &artifact::LockArtifact,
    selection: Option<&crate::Selection>,
) -> ModelResult<Vec<String>> {
    let Some(selection) = selection else {
        return resolve_locked_selection(manifest, lock, None);
    };
    let mut members = selection.clone();
    let root_selected = members.targets.iter().any(|target| target == "@root");
    let root_excluded = members
        .exclude_targets
        .iter()
        .any(|target| target == "@root");
    if root_selected && root_excluded {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "@root cannot be both selected and excluded",
        ));
    }
    members.targets.retain(|target| target != "@root");
    members.exclude_targets.retain(|target| target != "@root");
    let has_member_selector = members.all == Some(true)
        || !members.member_ids.is_empty()
        || !members.paths.is_empty()
        || !members.targets.is_empty();
    let mut selected = if has_member_selector {
        resolve_locked_selection(manifest, lock, Some(&members))?
    } else {
        Vec::new()
    };
    if root_selected {
        selected.push("@root".to_owned());
    }
    Ok(selected)
}

pub(super) fn ensure_git_member(member: &ManifestMember) -> ModelResult<()> {
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

pub(super) fn restore_error(error: ModelError) -> ModelError {
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

pub(super) fn restore_target_error(
    error: ModelError,
    target_id: &str,
    target_path: &str,
) -> ModelError {
    restore_error(error).with_member(target_id, target_path)
}

pub(super) fn initial_bundle_member(
    plan: &StashMemberPlan<'_>,
    full_message: &str,
) -> StashBundleMember {
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

pub(super) fn stash_push_needed(
    status: &GitStatus,
    include_untracked: bool,
    include_ignored: bool,
) -> bool {
    status.staged > 0
        || status.unstaged > 0
        || (include_untracked && status.untracked > 0)
        || (include_ignored
            && status
                .files
                .iter()
                .any(|file| file.index_status == "!" || file.worktree_status == "!"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_race_errors_keep_member_and_root_context() {
        for (target_id, path) in [("mem_app", "app"), ("@root", ".")] {
            let error = restore_target_error(
                ModelError::new(ErrorCode::GitCommandFailed, "stash entry not found"),
                target_id,
                path,
            );
            assert_eq!(error.code, ErrorCode::StashIncomplete);
            assert_eq!(error.member_id.as_deref(), Some(target_id));
            assert_eq!(error.member_path.as_deref(), Some(path));
        }
    }
}
