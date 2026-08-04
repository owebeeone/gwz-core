use std::collections::BTreeMap;

use crate::git::{GitMergeAnalysisKind, GitPreparedCommit, GitPreparedMerge, GitPreparedSignature};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::{
    MergeParticipantPlan, MergeParticipantRecord, PendingCommitSpec, PendingGitSignature,
    PendingMergeAction, PendingMergeActionKind, PendingMergeExpectedResult,
};

const INCOMPLETE_ACTION: &str =
    "pending action lacks a complete result class and exact commit specification";
const INTENT_MISMATCH: &str = "pending action inputs do not match the frozen participant record";
const MERGE_HEAD_MISMATCH: &str =
    "pending action source does not match the participant's expected merge head";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IntegrationIntent {
    pub(crate) target_branch: String,
    pub(crate) before_commit: String,
    pub(crate) source_commit: String,
    pub(crate) commit_message: String,
}

impl IntegrationIntent {
    pub(crate) fn from_plan(plan: &MergeParticipantPlan) -> Self {
        Self {
            target_branch: plan.target_branch.clone(),
            before_commit: plan.before_commit.clone(),
            source_commit: plan.source_commit.clone(),
            commit_message: plan.commit_message.clone(),
        }
    }

    pub(crate) fn from_record(participant: &MergeParticipantRecord) -> Self {
        Self {
            target_branch: participant.target_branch.clone(),
            before_commit: participant.before_commit.clone(),
            source_commit: participant.source_commit.clone(),
            commit_message: participant.commit_message.clone(),
        }
    }

    fn from_pending(pending: &PendingMergeAction) -> Self {
        Self {
            target_branch: pending.target_branch.clone(),
            before_commit: pending.before_commit.clone(),
            source_commit: pending.source_commit.clone(),
            commit_message: pending.commit_message.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PreparedIntegrationAction {
    VerifyUpToDate,
    FastForward,
    TrueMergeExpectedConflict,
    TrueMergeCommit(GitPreparedCommit),
    ResolveConflict(GitPreparedCommit),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedIntegration {
    pub(crate) intent: IntegrationIntent,
    pub(crate) action: PreparedIntegrationAction,
}

impl PreparedIntegration {
    pub(crate) fn from_merge(
        intent: IntegrationIntent,
        kind: GitMergeAnalysisKind,
        prepared: &GitPreparedMerge,
    ) -> Result<Self, &'static str> {
        let action = match (kind, prepared) {
            (GitMergeAnalysisKind::UpToDate, GitPreparedMerge::Unchanged) => {
                PreparedIntegrationAction::VerifyUpToDate
            }
            (GitMergeAnalysisKind::FastForward, GitPreparedMerge::FastForward) => {
                PreparedIntegrationAction::FastForward
            }
            (GitMergeAnalysisKind::TrueMerge, GitPreparedMerge::ExpectedConflict) => {
                PreparedIntegrationAction::TrueMergeExpectedConflict
            }
            (GitMergeAnalysisKind::TrueMerge, GitPreparedMerge::Commit(spec)) => {
                PreparedIntegrationAction::TrueMergeCommit(spec.clone())
            }
            _ => return Err("prepared merge kind does not match its exact result"),
        };
        Ok(Self { intent, action })
    }

    pub(crate) fn resolution(intent: IntegrationIntent, prepared: &GitPreparedCommit) -> Self {
        Self {
            intent,
            action: PreparedIntegrationAction::ResolveConflict(prepared.clone()),
        }
    }

    pub(crate) fn to_pending(&self) -> PendingMergeAction {
        let (kind, expected_result, commit_spec) = match &self.action {
            PreparedIntegrationAction::VerifyUpToDate => (
                PendingMergeActionKind::VerifyUpToDate,
                PendingMergeExpectedResult::Unchanged,
                None,
            ),
            PreparedIntegrationAction::FastForward => (
                PendingMergeActionKind::FastForward,
                PendingMergeExpectedResult::FastForward,
                None,
            ),
            PreparedIntegrationAction::TrueMergeExpectedConflict => (
                PendingMergeActionKind::TrueMerge,
                PendingMergeExpectedResult::ExpectedConflict,
                None,
            ),
            PreparedIntegrationAction::TrueMergeCommit(spec) => (
                PendingMergeActionKind::TrueMerge,
                PendingMergeExpectedResult::Commit,
                Some(pending_commit_spec(spec)),
            ),
            PreparedIntegrationAction::ResolveConflict(spec) => (
                PendingMergeActionKind::ResolveConflict,
                PendingMergeExpectedResult::Commit,
                Some(pending_commit_spec(spec)),
            ),
        };
        PendingMergeAction {
            kind,
            target_branch: self.intent.target_branch.clone(),
            before_commit: self.intent.before_commit.clone(),
            source_commit: self.intent.source_commit.clone(),
            commit_message: self.intent.commit_message.clone(),
            expected_result: Some(expected_result),
            commit_spec,
            extensions: BTreeMap::new(),
        }
    }
}

pub(crate) fn decode_pending(
    pending: &PendingMergeAction,
) -> Result<PreparedIntegration, &'static str> {
    use PendingMergeActionKind as Kind;
    use PendingMergeExpectedResult as ResultKind;

    let action = match (
        pending.kind,
        pending.expected_result,
        pending.commit_spec.as_ref(),
    ) {
        (Kind::VerifyUpToDate, None | Some(ResultKind::Unchanged), None) => {
            PreparedIntegrationAction::VerifyUpToDate
        }
        (Kind::FastForward, None | Some(ResultKind::FastForward), None) => {
            PreparedIntegrationAction::FastForward
        }
        (Kind::TrueMerge, Some(ResultKind::ExpectedConflict), None) => {
            PreparedIntegrationAction::TrueMergeExpectedConflict
        }
        (Kind::TrueMerge, Some(ResultKind::Commit), Some(spec)) => {
            PreparedIntegrationAction::TrueMergeCommit(prepared_commit(spec))
        }
        (Kind::ResolveConflict, Some(ResultKind::Commit), Some(spec)) => {
            PreparedIntegrationAction::ResolveConflict(prepared_commit(spec))
        }
        _ => return Err(INCOMPLETE_ACTION),
    };
    Ok(PreparedIntegration {
        intent: IntegrationIntent::from_pending(pending),
        action,
    })
}

pub(crate) fn decode_for_participant(
    pending: &PendingMergeAction,
    participant: &MergeParticipantRecord,
) -> Result<PreparedIntegration, &'static str> {
    let intent = IntegrationIntent::from_pending(pending);
    if intent != IntegrationIntent::from_record(participant) {
        return Err(INTENT_MISMATCH);
    }
    if participant
        .expected_merge_head
        .as_deref()
        .is_some_and(|merge_head| merge_head != intent.source_commit)
    {
        return Err(MERGE_HEAD_MISMATCH);
    }
    decode_pending(pending)
}

pub(crate) fn final_member_commit_message(
    custom_body: Option<&str>,
    source_ref: &str,
    target_branch: &str,
    merge_id: &str,
    operation_id: &str,
) -> ModelResult<String> {
    let body = match custom_body {
        Some(body) => normalize_custom_body(body)?,
        None => format!("Merge '{source_ref}' into '{target_branch}'"),
    };
    Ok(format!(
        "{body}\n\nGWZ-Merge-ID: {merge_id}\nGWZ-Operation-ID: {operation_id}"
    ))
}

pub(crate) fn validate_custom_commit_message(body: &str) -> ModelResult<()> {
    normalize_custom_body(body).map(|_| ())
}

fn normalize_custom_body(body: &str) -> ModelResult<String> {
    if body.contains('\0') {
        return Err(invalid_message("merge commit message contains a NUL byte"));
    }
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.trim_end_matches('\n');
    if normalized.trim().is_empty() {
        return Err(invalid_message("merge commit message must not be empty"));
    }
    Ok(normalized.to_owned())
}

fn invalid_message(message: &str) -> ModelError {
    ModelError::new(ErrorCode::MergeValidationFailed, message)
}

pub(crate) fn pending_commit_spec(spec: &GitPreparedCommit) -> PendingCommitSpec {
    PendingCommitSpec {
        tree_oid: spec.tree_oid.clone(),
        author: pending_signature(&spec.author),
        committer: pending_signature(&spec.committer),
        extensions: BTreeMap::new(),
    }
}

fn pending_signature(signature: &GitPreparedSignature) -> PendingGitSignature {
    PendingGitSignature {
        name: signature.name.clone(),
        email: signature.email.clone(),
        time_seconds: signature.time_seconds,
        timezone_offset_minutes: signature.timezone_offset_minutes,
        extensions: BTreeMap::new(),
    }
}

fn prepared_commit(spec: &PendingCommitSpec) -> GitPreparedCommit {
    GitPreparedCommit {
        tree_oid: spec.tree_oid.clone(),
        author: prepared_signature(&spec.author),
        committer: prepared_signature(&spec.committer),
    }
}

fn prepared_signature(signature: &PendingGitSignature) -> GitPreparedSignature {
    GitPreparedSignature {
        name: signature.name.clone(),
        email: signature.email.clone(),
        time_seconds: signature.time_seconds,
        timezone_offset_minutes: signature.timezone_offset_minutes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(name: &str) -> GitPreparedSignature {
        GitPreparedSignature {
            name: name.to_owned(),
            email: format!("{name}@example.test"),
            time_seconds: 123,
            timezone_offset_minutes: 600,
        }
    }

    fn intent() -> IntegrationIntent {
        IntegrationIntent {
            target_branch: "main".to_owned(),
            before_commit: "before".to_owned(),
            source_commit: "source".to_owned(),
            commit_message: "message".to_owned(),
        }
    }

    #[test]
    fn every_typed_action_round_trips_through_v0() {
        let commit = GitPreparedCommit {
            tree_oid: "tree".to_owned(),
            author: signature("author"),
            committer: signature("committer"),
        };
        for action in [
            PreparedIntegrationAction::VerifyUpToDate,
            PreparedIntegrationAction::FastForward,
            PreparedIntegrationAction::TrueMergeExpectedConflict,
            PreparedIntegrationAction::TrueMergeCommit(commit.clone()),
            PreparedIntegrationAction::ResolveConflict(commit),
        ] {
            let integration = PreparedIntegration {
                intent: intent(),
                action,
            };
            assert_eq!(decode_pending(&integration.to_pending()), Ok(integration));
        }
    }

    #[test]
    fn default_and_custom_message_bytes_are_frozen() {
        let default =
            final_member_commit_message(None, "feature/x", "main", "merge_1", "op_1").unwrap();
        assert_eq!(
            default,
            "Merge 'feature/x' into 'main'\n\nGWZ-Merge-ID: merge_1\nGWZ-Operation-ID: op_1"
        );

        for (body, expected) in [
            ("subject", "subject"),
            ("subject\n\nbody\n\n", "subject\n\nbody"),
            ("subject\r\nbody\r", "subject\nbody"),
            ("  ünicode  \n", "  ünicode  "),
            (
                "GWZ-Merge-ID: user\nGWZ-Operation-ID: user",
                "GWZ-Merge-ID: user\nGWZ-Operation-ID: user",
            ),
        ] {
            let message =
                final_member_commit_message(Some(body), "ignored", "ignored", "merge_1", "op_1")
                    .unwrap();
            assert_eq!(
                message,
                format!("{expected}\n\nGWZ-Merge-ID: merge_1\nGWZ-Operation-ID: op_1")
            );
            assert!(!message.ends_with('\n'));
        }
    }

    #[test]
    fn invalid_custom_messages_fail_before_construction() {
        for body in ["", " \t\n", "\u{2003}\n", "subject\0body"] {
            let error =
                final_member_commit_message(Some(body), "feature/x", "main", "merge_1", "op_1")
                    .unwrap_err();
            assert_eq!(error.code, ErrorCode::MergeValidationFailed);
        }
    }
}
