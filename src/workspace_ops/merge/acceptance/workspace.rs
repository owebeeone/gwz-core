use crate::artifact::{LockArtifact, ManifestArtifact};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::{MergeOperationRecord, MergeParticipantRecord, MergeTargetKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum CompleteLockErrorKind {
    Metadata,
    Record,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace_ops::merge) struct CompleteLockError {
    pub(in crate::workspace_ops::merge) kind: CompleteLockErrorKind,
    pub(in crate::workspace_ops::merge) error: ModelError,
}

impl CompleteLockError {
    fn metadata(error: ModelError) -> Self {
        Self {
            kind: CompleteLockErrorKind::Metadata,
            error,
        }
    }

    fn record(error: ModelError) -> Self {
        Self {
            kind: CompleteLockErrorKind::Record,
            error,
        }
    }
}

pub(in crate::workspace_ops::merge) fn construct_complete_lock(
    record: &MergeOperationRecord,
    manifest: &ManifestArtifact,
    mut lock: LockArtifact,
) -> Result<LockArtifact, CompleteLockError> {
    for target_id in &record.selected_targets {
        let participant = record.participants.get(target_id).ok_or_else(|| {
            CompleteLockError::record(unreadable(format!("participant '{target_id}' is missing")))
        })?;
        if participant.target_kind == MergeTargetKind::Root {
            if target_id != "@root" || participant.path != "." {
                return Err(CompleteLockError::record(unreadable(
                    "root participant identity is inconsistent",
                )));
            }
            continue;
        }
        let member = manifest
            .members
            .iter()
            .find(|member| member.id == *target_id && member.active)
            .ok_or_else(|| {
                CompleteLockError::metadata(
                    metadata(format!("active member '{target_id}' is missing"))
                        .with_member(target_id, &participant.path),
                )
            })?;
        let locked = lock.members.get_mut(target_id).ok_or_else(|| {
            CompleteLockError::metadata(
                metadata(format!("lock member '{target_id}' is missing"))
                    .with_member(target_id, &participant.path),
            )
        })?;
        if locked.path != participant.path
            || locked.path != member.path
            || locked.source_id.as_deref() != Some(member.source_id.as_str())
            || locked.source_kind != member.source_kind
        {
            return Err(CompleteLockError::metadata(
                metadata(format!(
                    "member '{target_id}' identity changed before finalization"
                ))
                .with_member(target_id, &participant.path),
            ));
        }
        let result = participant.resulting_commit.clone().ok_or_else(|| {
            CompleteLockError::record(
                unreadable(format!("participant '{target_id}' has no resulting commit"))
                    .with_member(target_id, &participant.path),
            )
        })?;
        locked.commit = Some(result);
        locked.branch = Some(participant.target_branch.clone());
        locked.detached = Some(false);
        locked.dirty = Some(false);
        locked.materialized = Some(true);
    }
    Ok(lock)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum AcceptedRootBase {
    BornAttached {
        commit: String,
        symbolic_branch: String,
    },
    BornDetached {
        commit: String,
    },
    UnbornAttached {
        symbolic_branch: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct AcceptedRootCheckout {
    pub(in crate::workspace_ops::merge) base: AcceptedRootBase,
    pub(in crate::workspace_ops::merge) root_selected: bool,
}

impl AcceptedRootCheckout {
    pub(in crate::workspace_ops::merge) fn evidence_parent(&self) -> Option<&str> {
        match &self.base {
            AcceptedRootBase::BornAttached { commit, .. }
            | AcceptedRootBase::BornDetached { commit } => Some(commit),
            AcceptedRootBase::UnbornAttached { .. } => None,
        }
    }

    pub(in crate::workspace_ops::merge) fn publication_branch(&self) -> Option<&str> {
        match &self.base {
            AcceptedRootBase::BornAttached {
                symbolic_branch, ..
            }
            | AcceptedRootBase::UnbornAttached { symbolic_branch } => Some(symbolic_branch),
            AcceptedRootBase::BornDetached { .. } => None,
        }
    }
}

#[cfg(test)]
pub(crate) fn accepted_root_checkout(
    record: &MergeOperationRecord,
) -> ModelResult<AcceptedRootCheckout> {
    accepted_root_checkout_with_observation(record, None)
}

pub(in crate::workspace_ops::merge) fn accepted_root_checkout_with_observation(
    record: &MergeOperationRecord,
    observation: Option<&crate::git::GitHeadState>,
) -> ModelResult<AcceptedRootCheckout> {
    if let Some(participant) = selected_root_participant(record)? {
        let commit = participant
            .resulting_commit
            .clone()
            .ok_or_else(|| unreadable("root participant has no resulting commit"))?;
        return Ok(AcceptedRootCheckout {
            base: AcceptedRootBase::BornAttached {
                commit,
                symbolic_branch: participant.target_branch.clone(),
            },
            root_selected: true,
        });
    }
    let base = match (&record.baseline.root_head, &record.baseline.root_branch) {
        (Some(commit), Some(symbolic_branch)) => AcceptedRootBase::BornAttached {
            commit: commit.clone(),
            symbolic_branch: symbolic_branch.clone(),
        },
        (Some(commit), None) => observation
            .filter(|head| {
                !head.is_detached
                    && head.commit.as_deref() == Some(commit.as_str())
                    && head.branch.is_some()
            })
            .map_or_else(
                || AcceptedRootBase::BornDetached {
                    commit: commit.clone(),
                },
                |head| AcceptedRootBase::BornAttached {
                    commit: commit.clone(),
                    symbolic_branch: head.branch.clone().expect("observed attached branch"),
                },
            ),
        (None, Some(symbolic_branch)) => AcceptedRootBase::UnbornAttached {
            symbolic_branch: symbolic_branch.clone(),
        },
        (None, None) => {
            let observed = observation
                .filter(|head| !head.is_detached && head.commit.is_none())
                .and_then(|head| head.branch.as_ref())
                .ok_or_else(|| unreadable("workspace root branch is missing"))?;
            AcceptedRootBase::UnbornAttached {
                symbolic_branch: observed.clone(),
            }
        }
    };
    Ok(AcceptedRootCheckout {
        base,
        root_selected: false,
    })
}

pub(in crate::workspace_ops::merge) fn selected_root_participant(
    record: &MergeOperationRecord,
) -> ModelResult<Option<&MergeParticipantRecord>> {
    let participant = record.participants.get("@root");
    let selected = record
        .selected_targets
        .iter()
        .any(|target| target == "@root");
    match (selected, participant) {
        (false, None) => Ok(None),
        (true, Some(participant))
            if participant.target_kind == MergeTargetKind::Root
                && participant.path == "."
                && super::super::participant_semantics::result::is_successful_result(
                    participant.state,
                ) =>
        {
            Ok(Some(participant))
        }
        _ => Err(unreadable(
            "selected root participant identity or successful state is inconsistent",
        )),
    }
}

pub(in crate::workspace_ops::merge) fn publication_required(record: &MergeOperationRecord) -> bool {
    record
        .participants
        .values()
        .any(super::super::participant_semantics::result::has_changed_result)
}

fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}

fn metadata(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::ManifestInvalid, message)
}
