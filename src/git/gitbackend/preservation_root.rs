use super::*;
use crate::checked_artifact::entry::MergeArtifactTransition;
use crate::filesystem::FileSystem;
use crate::operation_context::BorrowedOperationContext;
pub(super) mod files;
pub(super) mod index;
pub(super) mod index_format;
pub(super) mod parent;
pub(crate) use preservation::FaultBoundary;
use preservation::fault;
#[cfg(test)]
pub(crate) use preservation::{fail_next_at, run_next_at};
pub(super) fn prepare_root_preservation_stash(
    filesystem: &dyn FileSystem,
    backend: &impl GitBackend,
    root: &Path,
    spec: &GitRootPreservationSpec,
) -> ModelResult<GitPreparedRootStash> {
    let context = BorrowedOperationContext::new(backend, filesystem);
    backend.validate_root_preservation_spec(root, spec)?;
    if !exact_head(backend, root, &spec.attached_branch, &spec.attached_commit)?
        || backend.repository_state(root)? != GitRepositoryState::Clean
        || !full_form_matches(context, root, spec, &spec.handoff_form)?
        || !files::observe_boundary(filesystem, backend, root, &spec.handoff_boundary)?
    {
        return Err(evidence_error(
            "root preservation preparation requires the exact durable handoff",
        ));
    }
    Ok(GitPreparedRootStash {
        normalized_image: backend.root_preservation_image(
            root,
            &spec.attached_clean_form,
            &spec.excluded_worktree_paths,
        )?,
    })
}
pub(super) fn observe_root_preservation_step(
    filesystem: &dyn FileSystem,
    backend: &impl GitBackend,
    root: &Path,
    spec: &GitRootPreservationSpec,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
) -> ModelResult<GitRootPreservationStepObservation> {
    let context = BorrowedOperationContext::new(backend, filesystem);
    observe_root_preservation_step_pinned(context, root, spec, step, guard, None)
}
fn observe_root_preservation_step_pinned(
    context: BorrowedOperationContext<'_>,
    root: &Path,
    spec: &GitRootPreservationSpec,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
    pinned: Option<&parent::PinnedConfig>,
) -> ModelResult<GitRootPreservationStepObservation> {
    let backend = context.repository;
    backend.validate_root_preservation_spec(root, spec)?;
    match step {
        GitRootPreservationPhysicalStep::Managed(transition) => {
            observe_managed(context, root, spec, transition, guard, pinned)
        }
        GitRootPreservationPhysicalStep::CreateStash { merge_id } => {
            observe_create_stash(context, root, spec, merge_id, guard)
        }
        GitRootPreservationPhysicalStep::ResetAttachedRef => {
            observe_reset(context, root, spec, guard)
        }
    }
}
pub(super) fn execute_root_preservation_step_checked(
    filesystem: &dyn FileSystem,
    backend: &impl GitBackend,
    root: &Path,
    spec: &GitRootPreservationSpec,
    step: &GitRootPreservationPhysicalStep,
    guard: &GitRootPreservationGuard,
) -> ModelResult<GitCheckedPreservationMutation> {
    let context = BorrowedOperationContext::new(backend, filesystem);
    let pinned = match step {
        GitRootPreservationPhysicalStep::Managed(transition)
            if transition.object == GitRootManagedObject::MarkerParentDirectory =>
        {
            Some(parent::PinnedConfig::open(filesystem, root)?)
        }
        _ => None,
    };
    let observe =
        |pinned| observe_root_preservation_step_pinned(context, root, spec, step, guard, pinned);
    match observe(pinned.as_ref())? {
        GitRootPreservationStepObservation::After => {
            return Ok(GitCheckedPreservationMutation::AlreadyComplete);
        }
        GitRootPreservationStepObservation::AfterNeedsDurability => {
            let GitRootPreservationPhysicalStep::Managed(transition) = step else {
                return Err(evidence_error(
                    "non-parent step requested a durability barrier",
                ));
            };
            pinned
                .as_ref()
                .expect("parent durability step retains its pinned parent")
                .barrier(&parent::staging_name(
                    spec,
                    transition.source,
                    transition.goal,
                ))?;
            if observe(pinned.as_ref())? != GitRootPreservationStepObservation::AfterNeedsDurability
            {
                return Err(evidence_error("marker-parent barrier lost its exact goal"));
            }
            return Ok(GitCheckedPreservationMutation::AlreadyComplete);
        }
        GitRootPreservationStepObservation::Ambiguous => {
            return Err(evidence_error(
                "root preservation step has ambiguous physical evidence",
            ));
        }
        GitRootPreservationStepObservation::Before => {}
    }
    fault(FaultBoundary::Before)?;
    let mutation = match step {
        GitRootPreservationPhysicalStep::Managed(transition) => {
            mutate_managed(context, root, spec, transition, pinned.as_ref())?;
            GitCheckedPreservationMutation::Applied
        }
        GitRootPreservationPhysicalStep::CreateStash { merge_id } => {
            GitCheckedPreservationMutation::StashCreated(
                backend.stash_for_merge_preservation(root, merge_id, true)?,
            )
        }
        GitRootPreservationPhysicalStep::ResetAttachedRef => {
            GitCheckedPreservationMutation::RefReset(backend.set_branch_target_checked(
                root,
                &spec.attached_branch,
                &spec.attached_commit,
                &spec.restore_commit,
            )?)
        }
    };
    fault(FaultBoundary::After)?;
    let observed = observe(pinned.as_ref())?;
    let expected = if matches!(
        step,
        GitRootPreservationPhysicalStep::Managed(GitRootManagedTransition {
            object: GitRootManagedObject::MarkerParentDirectory,
            ..
        })
    ) {
        GitRootPreservationStepObservation::AfterNeedsDurability
    } else {
        GitRootPreservationStepObservation::After
    };
    if observed != expected {
        return Err(evidence_error(
            "root preservation mutation failed exact post-verification",
        ));
    }
    Ok(mutation)
}
fn observe_managed(
    context: BorrowedOperationContext<'_>,
    root: &Path,
    spec: &GitRootPreservationSpec,
    transition: &GitRootManagedTransition,
    guard: &GitRootPreservationGuard,
    pinned: Option<&parent::PinnedConfig>,
) -> ModelResult<GitRootPreservationStepObservation> {
    let filesystem = context.filesystem;
    let backend = context.repository;
    let restore_side = matches!(
        (transition.source, transition.goal),
        (
            GitRootManagedFormName::RestoreClean,
            GitRootManagedFormName::Handoff
        ) | (
            GitRootManagedFormName::Handoff,
            GitRootManagedFormName::RestoreClean
        )
    );
    let (commit, clean) = if restore_side {
        (&spec.restore_commit, &spec.restore_clean_form)
    } else {
        (&spec.attached_commit, &spec.attached_clean_form)
    };
    if backend.repository_state(root)? != GitRepositoryState::Clean
        || !exact_head(backend, root, &spec.attached_branch, commit)?
        || !files::observe_boundary(filesystem, backend, root, &spec.handoff_boundary)?
        || !guard_matches(backend, root, spec, guard, clean)?
    {
        return Ok(GitRootPreservationStepObservation::Ambiguous);
    }
    let source = form(spec, transition.source);
    let goal = form(spec, transition.goal);
    if transition.object == GitRootManagedObject::MarkerParentDirectory {
        return observe_parent(context, root, spec, transition, source, goal, pinned);
    }
    let staging = parent::staging_name(spec, transition.source, transition.goal);
    if matches!(
        transition.object,
        GitRootManagedObject::MarkerWorktree | GitRootManagedObject::LockWorktree
    ) {
        let (path, source_file, goal_file) = match transition.object {
            GitRootManagedObject::MarkerWorktree => (
                spec.managed_marker_path.as_str(),
                source.marker.as_ref(),
                goal.marker.as_ref(),
            ),
            GitRootManagedObject::LockWorktree => (
                crate::artifact::LOCK_PATH,
                Some(&source.lock),
                Some(&goal.lock),
            ),
            GitRootManagedObject::Index | GitRootManagedObject::MarkerParentDirectory => {
                unreachable!("checked above")
            }
        };
        let observed = files::observe_transition(filesystem, root, path, source_file, goal_file)?;
        let after = observed == MergeArtifactTransition::After;
        if observed == MergeArtifactTransition::Ambiguous
            || !pattern_matches(
                context,
                root,
                spec,
                &staging,
                transition,
                after,
                Some(transition.object),
            )?
        {
            return Ok(GitRootPreservationStepObservation::Ambiguous);
        }
        return Ok(if after {
            GitRootPreservationStepObservation::After
        } else {
            GitRootPreservationStepObservation::Before
        });
    }
    if pattern_matches(context, root, spec, &staging, transition, true, None)? {
        return Ok(GitRootPreservationStepObservation::After);
    }
    if object_matches(context, root, spec, &staging, transition.object, source)?
        && !object_matches(context, root, spec, &staging, transition.object, goal)?
        && pattern_matches(context, root, spec, &staging, transition, false, None)?
    {
        return Ok(GitRootPreservationStepObservation::Before);
    }
    Ok(GitRootPreservationStepObservation::Ambiguous)
}
fn observe_create_stash(
    context: BorrowedOperationContext<'_>,
    root: &Path,
    spec: &GitRootPreservationSpec,
    merge_id: &str,
    guard: &GitRootPreservationGuard,
) -> ModelResult<GitRootPreservationStepObservation> {
    let filesystem = context.filesystem;
    let backend = context.repository;
    let GitRootPreservationGuard::NormalizedPreimage { sha256 } = guard else {
        return Err(invalid("CreateStash requires a normalized-preimage guard"));
    };
    if backend.repository_state(root)? != GitRepositoryState::Clean
        || !exact_head(backend, root, &spec.attached_branch, &spec.attached_commit)?
        || !files::observe_boundary(filesystem, backend, root, &spec.handoff_boundary)?
        || !full_form_matches(context, root, spec, &spec.attached_clean_form)?
    {
        return Ok(GitRootPreservationStepObservation::Ambiguous);
    }
    let stashes = backend.preservation_stashes(root, merge_id)?;
    if let [stash] = stashes.as_slice()
        && stash.head_commit == spec.attached_commit
        && stash.image.preimage_sha256 == *sha256
        && otherwise_clean(backend, root, spec, &spec.attached_clean_form)?
    {
        return Ok(GitRootPreservationStepObservation::After);
    }
    if stashes.is_empty()
        && backend
            .root_preservation_image(
                root,
                &spec.attached_clean_form,
                &spec.excluded_worktree_paths,
            )?
            .preimage_sha256
            == *sha256
    {
        return Ok(GitRootPreservationStepObservation::Before);
    }
    Ok(GitRootPreservationStepObservation::Ambiguous)
}
fn observe_reset(
    context: BorrowedOperationContext<'_>,
    root: &Path,
    spec: &GitRootPreservationSpec,
    guard: &GitRootPreservationGuard,
) -> ModelResult<GitRootPreservationStepObservation> {
    let filesystem = context.filesystem;
    let backend = context.repository;
    if !matches!(guard, GitRootPreservationGuard::OtherwiseClean)
        || backend.repository_state(root)? != GitRepositoryState::Clean
        || !files::observe_boundary(filesystem, backend, root, &spec.handoff_boundary)?
    {
        return Ok(GitRootPreservationStepObservation::Ambiguous);
    }
    if exact_head(backend, root, &spec.attached_branch, &spec.restore_commit)?
        && full_form_matches(context, root, spec, &spec.restore_clean_form)?
        && otherwise_clean(backend, root, spec, &spec.restore_clean_form)?
    {
        return Ok(GitRootPreservationStepObservation::After);
    }
    if exact_head(backend, root, &spec.attached_branch, &spec.attached_commit)?
        && full_form_matches(context, root, spec, &spec.attached_clean_form)?
        && otherwise_clean(backend, root, spec, &spec.attached_clean_form)?
    {
        return Ok(GitRootPreservationStepObservation::Before);
    }
    Ok(GitRootPreservationStepObservation::Ambiguous)
}
fn pattern_matches(
    context: BorrowedOperationContext<'_>,
    root: &Path,
    spec: &GitRootPreservationSpec,
    staging: &str,
    transition: &GitRootManagedTransition,
    after: bool,
    ignored: Option<GitRootManagedObject>,
) -> ModelResult<bool> {
    let source = form(spec, transition.source);
    let goal = form(spec, transition.goal);
    let reverse = transition.goal == GitRootManagedFormName::Handoff;
    let objects = if reverse {
        vec![
            GitRootManagedObject::Index,
            GitRootManagedObject::LockWorktree,
            GitRootManagedObject::MarkerParentDirectory,
            GitRootManagedObject::MarkerWorktree,
        ]
    } else {
        vec![
            GitRootManagedObject::MarkerParentDirectory,
            GitRootManagedObject::MarkerWorktree,
            GitRootManagedObject::LockWorktree,
            GitRootManagedObject::Index,
        ]
    };
    let named_index = objects
        .iter()
        .position(|object| *object == transition.object)
        .expect("closed managed-object set");
    for (position, object) in objects.into_iter().enumerate() {
        if Some(object) == ignored {
            continue;
        }
        let expected = if position < named_index || after && position == named_index {
            goal
        } else {
            source
        };
        if !object_matches(context, root, spec, staging, object, expected)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn observe_parent(
    context: BorrowedOperationContext<'_>,
    root: &Path,
    spec: &GitRootPreservationSpec,
    transition: &GitRootManagedTransition,
    source: &GitRootManagedForm,
    goal: &GitRootManagedForm,
    pinned: Option<&parent::PinnedConfig>,
) -> ModelResult<GitRootPreservationStepObservation> {
    let filesystem = context.filesystem;
    use parent::State as P;
    let staging = parent::staging_name(spec, transition.source, transition.goal);
    let surrounding = if transition.goal == GitRootManagedFormName::Handoff {
        goal
    } else {
        source
    };
    if !object_matches(
        context,
        root,
        spec,
        &staging,
        GitRootManagedObject::Index,
        surrounding,
    )? || !object_matches(
        context,
        root,
        spec,
        &staging,
        GitRootManagedObject::LockWorktree,
        surrounding,
    )? || !object_matches(
        context,
        root,
        spec,
        &staging,
        GitRootManagedObject::MarkerWorktree,
        source,
    )? {
        return Ok(GitRootPreservationStepObservation::Ambiguous);
    }
    let state = match pinned {
        Some(parent) => parent.observe(&spec.managed_marker_path, &staging),
        None => parent::observe(filesystem, root, &spec.managed_marker_path, &staging),
    }?;
    Ok(
        match (source.marker.is_some(), goal.marker.is_some(), state) {
            (_, false, P::Missing | P::Empty) | (true, false, P::ExpectedMarker) => {
                GitRootPreservationStepObservation::After
            }
            (true, true, P::ExpectedMarker) => GitRootPreservationStepObservation::After,
            (false, true, P::Missing | P::StagingOnly) => {
                GitRootPreservationStepObservation::Before
            }
            (false, true, P::Empty) => GitRootPreservationStepObservation::AfterNeedsDurability,
            _ => GitRootPreservationStepObservation::Ambiguous,
        },
    )
}

fn marker_parent_matches(
    filesystem: &dyn FileSystem,
    root: &Path,
    spec: &GitRootPreservationSpec,
    staging: &str,
    marker_present: bool,
) -> ModelResult<bool> {
    use parent::State as P;
    let state = parent::observe(filesystem, root, &spec.managed_marker_path, staging)?;
    Ok(if marker_present {
        matches!(state, P::Empty | P::ExpectedMarker)
    } else {
        matches!(state, P::Missing | P::Empty | P::ExpectedMarker)
    })
}

fn object_matches(
    context: BorrowedOperationContext<'_>,
    root: &Path,
    spec: &GitRootPreservationSpec,
    staging: &str,
    object: GitRootManagedObject,
    expected: &GitRootManagedForm,
) -> ModelResult<bool> {
    let filesystem = context.filesystem;
    let backend = context.repository;
    match object {
        GitRootManagedObject::MarkerWorktree => files::observe_relative(
            filesystem,
            root,
            &expected.marker,
            expected.index.marker.path_str()?,
        ),
        GitRootManagedObject::LockWorktree => {
            files::observe_required(filesystem, root, &expected.lock)
        }
        GitRootManagedObject::Index => backend.root_managed_index_matches(root, &expected.index),
        GitRootManagedObject::MarkerParentDirectory => {
            marker_parent_matches(filesystem, root, spec, staging, expected.marker.is_some())
        }
    }
}

fn full_form_matches(
    context: BorrowedOperationContext<'_>,
    root: &Path,
    spec: &GitRootPreservationSpec,
    expected: &GitRootManagedForm,
) -> ModelResult<bool> {
    let staging = parent::staging_name(
        spec,
        GitRootManagedFormName::AttachedClean,
        GitRootManagedFormName::Handoff,
    );
    Ok(object_matches(
        context,
        root,
        spec,
        &staging,
        GitRootManagedObject::MarkerWorktree,
        expected,
    )? && object_matches(
        context,
        root,
        spec,
        &staging,
        GitRootManagedObject::LockWorktree,
        expected,
    )? && object_matches(
        context,
        root,
        spec,
        &staging,
        GitRootManagedObject::Index,
        expected,
    )? && object_matches(
        context,
        root,
        spec,
        &staging,
        GitRootManagedObject::MarkerParentDirectory,
        expected,
    )?)
}

fn mutate_managed(
    context: BorrowedOperationContext<'_>,
    root: &Path,
    spec: &GitRootPreservationSpec,
    transition: &GitRootManagedTransition,
    pinned: Option<&parent::PinnedConfig>,
) -> ModelResult<()> {
    let filesystem = context.filesystem;
    let backend = context.repository;
    let source = form(spec, transition.source);
    let goal = form(spec, transition.goal);
    match transition.object {
        GitRootManagedObject::MarkerWorktree => files::replace_relative(
            filesystem,
            root,
            &spec.managed_marker_path,
            source.marker.as_ref(),
            goal.marker.as_ref(),
        ),
        GitRootManagedObject::LockWorktree => files::replace_relative(
            filesystem,
            root,
            crate::artifact::LOCK_PATH,
            Some(&source.lock),
            Some(&goal.lock),
        ),
        GitRootManagedObject::Index => {
            backend.rewrite_root_managed_index_checked(root, &goal.index)
        }
        GitRootManagedObject::MarkerParentDirectory => pinned
            .expect("parent mutation retains its pinned parent")
            .publish(&parent::staging_name(
                spec,
                transition.source,
                transition.goal,
            )),
    }
}

fn guard_matches(
    backend: &(impl GitBackend + ?Sized),
    root: &Path,
    spec: &GitRootPreservationSpec,
    guard: &GitRootPreservationGuard,
    clean: &GitRootManagedForm,
) -> ModelResult<bool> {
    match guard {
        GitRootPreservationGuard::NormalizedPreimage { sha256 } => Ok(backend
            .root_preservation_image(
                root,
                &spec.attached_clean_form,
                &spec.excluded_worktree_paths,
            )?
            .preimage_sha256
            == *sha256),
        GitRootPreservationGuard::OtherwiseClean => otherwise_clean(backend, root, spec, clean),
    }
}

fn otherwise_clean(
    backend: &(impl GitBackend + ?Sized),
    root: &Path,
    spec: &GitRootPreservationSpec,
    clean: &GitRootManagedForm,
) -> ModelResult<bool> {
    Ok(backend
        .root_preservation_image(root, clean, &spec.excluded_worktree_paths)?
        .dirty
        == GitPreservationDirtySummary::default())
}

fn exact_head(
    backend: &(impl GitBackend + ?Sized),
    root: &Path,
    branch: &str,
    commit: &str,
) -> ModelResult<bool> {
    let head = backend.head(root)?;
    Ok(!head.is_detached
        && head.branch.as_deref() == Some(branch)
        && head.commit.as_deref() == Some(commit)
        && backend
            .read_ref(root, &format!("refs/heads/{branch}"))?
            .as_deref()
            == Some(commit))
}

fn form(spec: &GitRootPreservationSpec, name: GitRootManagedFormName) -> &GitRootManagedForm {
    match name {
        GitRootManagedFormName::AttachedClean => &spec.attached_clean_form,
        GitRootManagedFormName::RestoreClean => &spec.restore_clean_form,
        GitRootManagedFormName::Handoff => &spec.handoff_form,
    }
}

impl GitRootManagedIndexFact {
    fn path_str(&self) -> ModelResult<&str> {
        std::str::from_utf8(index::fact_path(self))
            .map_err(|_| invalid("managed index path is not UTF-8"))
    }
}

fn invalid(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, detail.into())
}
fn evidence_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
}
