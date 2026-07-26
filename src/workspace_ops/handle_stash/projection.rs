use super::*;

pub(super) fn stash_member_response(
    member: &ManifestMember,
    status: crate::MemberStatus,
    error: Option<ModelError>,
) -> crate::MemberResponse {
    stash_target_response(
        &member.id,
        &member.path,
        crate::TargetKind::Member,
        status,
        error,
    )
}

pub(super) fn stash_target_response(
    target_id: &str,
    target_path: &str,
    target_kind: crate::TargetKind,
    status: crate::MemberStatus,
    error: Option<ModelError>,
) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: target_id.to_owned(),
        member_path: target_path.to_owned(),
        source_kind: crate::SourceKind::Git,
        status,
        error: error.map(|error| crate::GwzError {
            code: error.code.into(),
            message: error.message,
            member_id: Some(target_id.to_owned()),
            member_path: Some(target_path.to_owned()),
            target_kind: Some(target_kind),
            detail: None,
        }),
        planned: None,
        state: None,
        git_status: None,
        target_kind: Some(target_kind),
        lock_match: None,
    }
}

pub(super) fn planned_member_responses(
    plans: &[StashMemberPlan<'_>],
) -> Vec<crate::MemberResponse> {
    plans
        .iter()
        .map(|plan| stash_member_response(plan.member, crate::MemberStatus::Planned, None))
        .collect()
}

pub(super) fn aggregate_from_members(
    responses: &[crate::MemberResponse],
) -> crate::AggregateStatus {
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

pub(super) fn stash_response(
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

pub(super) fn project_bundle(bundle: &StashBundle) -> crate::StashBundle {
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
