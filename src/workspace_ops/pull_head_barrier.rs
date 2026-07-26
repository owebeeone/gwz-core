use std::path::Path;

#[cfg(test)]
use std::cell::RefCell;

use crate::git::{GitBackend, GitPreparedMerge};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::pull_head_merge_preflight::{RootMergePullPlan, validate_root_merge_pull};
use super::pull_head_plan::{PullHeadAction, PullHeadPlan, PullHeadSource};

#[cfg(test)]
thread_local! {
    static BEFORE_PULL_BARRIER: RefCell<Option<Box<dyn FnOnce()>>> =
        const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn before_next_pull_barrier(callback: impl FnOnce() + 'static) {
    BEFORE_PULL_BARRIER.with(|slot| {
        assert!(
            slot.borrow_mut().replace(Box::new(callback)).is_none(),
            "a pull-barrier callback is already installed"
        );
    });
}

#[cfg(test)]
fn run_before_pull_barrier() {
    if let Some(callback) = BEFORE_PULL_BARRIER.with(|slot| slot.borrow_mut().take()) {
        callback();
    }
}

#[cfg(not(test))]
fn run_before_pull_barrier() {}

pub(crate) fn validate_pull_barrier<B: GitBackend>(
    backend: &B,
    root: &Path,
    root_plan: Option<&RootMergePullPlan>,
    member_plans: &[PullHeadPlan],
) -> ModelResult<()> {
    run_before_pull_barrier();
    if let Some(plan) = root_plan {
        validate_root_merge_pull(backend, root, plan)?;
    }
    for plan in member_plans {
        let member_root = root.join(&plan.state.path);
        validate_member_plan(backend, &member_root, plan)
            .map_err(|error| error.with_member(&plan.member_id, &plan.state.path))?;
    }
    Ok(())
}

fn validate_member_plan<B: GitBackend>(
    backend: &B,
    member_root: &Path,
    plan: &PullHeadPlan,
) -> ModelResult<()> {
    match &plan.action {
        PullHeadAction::Noop
        | PullHeadAction::SkipNoFetchRemote
        | PullHeadAction::FetchOnly
        | PullHeadAction::PredictedConflict { .. } => Ok(()),
        PullHeadAction::UpToDate {
            source,
            prepared: Some(prepared),
        } => {
            validate_source_ref(backend, member_root, source)?;
            validate_prepared(backend, member_root, &plan.branch, source, prepared)
        }
        PullHeadAction::UpToDate {
            source,
            prepared: None,
        } => {
            validate_source_ref(backend, member_root, source)?;
            validate_attached_head(backend, member_root, &plan.branch, source)
        }
        PullHeadAction::FastForward { source, prepared }
        | PullHeadAction::Merge { source, prepared } => {
            validate_source_ref(backend, member_root, source)?;
            validate_prepared(backend, member_root, &plan.branch, source, prepared)
        }
        PullHeadAction::Rebase { source } | PullHeadAction::Reset { source } => {
            validate_source_ref(backend, member_root, source)?;
            validate_attached_head(backend, member_root, &plan.branch, source)
        }
    }
}

fn validate_source_ref<B: GitBackend>(
    backend: &B,
    member_root: &Path,
    source: &PullHeadSource,
) -> ModelResult<()> {
    if backend
        .read_ref(member_root, &source.remote_ref)?
        .as_deref()
        != Some(source.source_commit.as_str())
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!(
                "remote-tracking ref '{}' changed after pull preparation",
                source.remote_ref
            ),
        ));
    }
    Ok(())
}

fn validate_prepared<B: GitBackend>(
    backend: &B,
    member_root: &Path,
    branch: &str,
    source: &PullHeadSource,
    prepared: &GitPreparedMerge,
) -> ModelResult<()> {
    backend.validate_prepared_merge_upstream_state(
        member_root,
        branch,
        &source.expected_local,
        &source.source_commit,
        prepared,
    )
}

fn validate_attached_head<B: GitBackend>(
    backend: &B,
    member_root: &Path,
    branch: &str,
    source: &PullHeadSource,
) -> ModelResult<()> {
    let head = backend.head(member_root)?;
    if head.is_detached
        || head.branch.as_deref() != Some(branch)
        || head.commit.as_deref() != Some(source.expected_local.as_str())
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!("target branch '{branch}' changed after pull preparation"),
        ));
    }
    Ok(())
}
