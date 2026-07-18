use crate::model::{ErrorCode, ModelError, ModelResult};

/// A semantic command presented to the single pre-dispatch open-merge gate.
/// Drivers classify syntax once; lifecycle handlers retain only narrower
/// participant checks for conditionally allowed recovery operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenMergeCommand {
    StageConflictResolution,
    BranchList,
    BranchMutate,
    Capture,
    CloneWorkspace,
    Commit,
    Diff,
    Forall,
    InitNewWorkspace,
    InitExistingPlan,
    InitUpdate,
    Ls,
    Materialize,
    Pull,
    Push,
    RepoMutate,
    Snapshot,
    SnapshotList,
    StashList,
    StashMutate,
    Status,
    TagList,
    TagMutate,
    MergeStatus,
    MergeRecovery,
    MergeGc,
    MergeStart,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenMergeGateDecision {
    Allow,
    Conditional,
    Block,
    NotGated,
}

impl OpenMergeCommand {
    pub fn gate_decision(self) -> OpenMergeGateDecision {
        use OpenMergeCommand as Command;
        use OpenMergeGateDecision as Decision;

        match self {
            Command::StageConflictResolution => Decision::Conditional,
            Command::BranchList
            | Command::Diff
            | Command::InitExistingPlan
            | Command::Ls
            | Command::SnapshotList
            | Command::StashList
            | Command::Status
            | Command::TagList
            | Command::MergeStatus
            | Command::MergeRecovery
            | Command::MergeGc => Decision::Allow,
            Command::CloneWorkspace | Command::InitNewWorkspace => Decision::NotGated,
            Command::BranchMutate
            | Command::Capture
            | Command::Commit
            | Command::Forall
            | Command::InitUpdate
            | Command::Materialize
            | Command::Pull
            | Command::Push
            | Command::RepoMutate
            | Command::Snapshot
            | Command::StashMutate
            | Command::TagMutate
            | Command::MergeStart => Decision::Block,
        }
    }
}

/// Applies the centralized allowlist after workspace/open-record discovery and
/// before command-handler dispatch.
pub fn enforce_open_merge_gate(
    open_merge_id: Option<&str>,
    command: OpenMergeCommand,
) -> ModelResult<()> {
    let Some(merge_id) = open_merge_id else {
        return Ok(());
    };
    if command.gate_decision() != OpenMergeGateDecision::Block {
        return Ok(());
    }
    Err(ModelError::new(
        ErrorCode::OpenOperation,
        format!(
            "merge '{merge_id}' is open; this command is blocked until it is recovered; \
             use merge status, merge continue, or merge abort"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_table_covers_every_design_row() {
        use OpenMergeCommand as Command;
        use OpenMergeGateDecision as Decision;

        let rows = [
            (Command::StageConflictResolution, Decision::Conditional),
            (Command::BranchList, Decision::Allow),
            (Command::BranchMutate, Decision::Block),
            (Command::Capture, Decision::Block),
            (Command::CloneWorkspace, Decision::NotGated),
            (Command::Commit, Decision::Block),
            (Command::Diff, Decision::Allow),
            (Command::Forall, Decision::Block),
            (Command::InitNewWorkspace, Decision::NotGated),
            (Command::InitExistingPlan, Decision::Allow),
            (Command::InitUpdate, Decision::Block),
            (Command::Ls, Decision::Allow),
            (Command::Materialize, Decision::Block),
            (Command::Pull, Decision::Block),
            (Command::Push, Decision::Block),
            (Command::RepoMutate, Decision::Block),
            (Command::Snapshot, Decision::Block),
            (Command::SnapshotList, Decision::Allow),
            (Command::StashList, Decision::Allow),
            (Command::StashMutate, Decision::Block),
            (Command::Status, Decision::Allow),
            (Command::TagList, Decision::Allow),
            (Command::TagMutate, Decision::Block),
            (Command::MergeStatus, Decision::Allow),
            (Command::MergeRecovery, Decision::Allow),
            (Command::MergeGc, Decision::Allow),
            (Command::MergeStart, Decision::Block),
        ];
        for (command, expected) in rows {
            assert_eq!(command.gate_decision(), expected, "{command:?}");
        }
    }

    #[test]
    fn blocked_error_names_open_merge_and_recovery_commands() {
        let error = enforce_open_merge_gate(Some("merge_42"), OpenMergeCommand::Push)
            .expect_err("push must be blocked");
        assert_eq!(error.code, ErrorCode::OpenOperation);
        assert!(error.message.contains("merge_42"));
        assert!(error.message.contains("merge status"));
        assert!(error.message.contains("merge continue"));
        assert!(error.message.contains("merge abort"));

        assert!(enforce_open_merge_gate(None, OpenMergeCommand::Push).is_ok());
        assert!(enforce_open_merge_gate(Some("merge_42"), OpenMergeCommand::Status).is_ok());
    }
}
