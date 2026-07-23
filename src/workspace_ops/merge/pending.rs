use crate::git::{GitPreparedCommit, GitPreparedMerge, GitPreparedSignature};

use super::{
    PendingGitSignature, PendingMergeAction, PendingMergeActionKind, PendingMergeExpectedResult,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DurablePreparedAction {
    Merge(GitPreparedMerge),
    Resolution(GitPreparedCommit),
}

pub(crate) fn decode_durable_prepared_action(
    pending: &PendingMergeAction,
) -> Result<DurablePreparedAction, &'static str> {
    use PendingMergeActionKind as Kind;
    use PendingMergeExpectedResult as ResultKind;

    match (
        pending.kind,
        pending.expected_result,
        pending.commit_spec.as_ref(),
    ) {
        (Kind::VerifyUpToDate, None | Some(ResultKind::Unchanged), None) => {
            Ok(DurablePreparedAction::Merge(GitPreparedMerge::Unchanged))
        }
        (Kind::FastForward, None | Some(ResultKind::FastForward), None) => {
            Ok(DurablePreparedAction::Merge(GitPreparedMerge::FastForward))
        }
        (Kind::TrueMerge, Some(ResultKind::ExpectedConflict), None) => Ok(
            DurablePreparedAction::Merge(GitPreparedMerge::ExpectedConflict),
        ),
        (Kind::TrueMerge, Some(ResultKind::Commit), Some(spec)) => Ok(
            DurablePreparedAction::Merge(GitPreparedMerge::Commit(prepared_commit(spec))),
        ),
        (Kind::ResolveConflict, Some(ResultKind::Commit), Some(spec)) => {
            Ok(DurablePreparedAction::Resolution(prepared_commit(spec)))
        }
        _ => Err("pending action lacks a complete result class and exact commit specification"),
    }
}

fn prepared_commit(spec: &super::PendingCommitSpec) -> GitPreparedCommit {
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
    use std::collections::BTreeMap;

    use super::*;
    use crate::workspace_ops::merge::{PendingCommitSpec, PendingGitSignature};

    fn signature(name: &str) -> PendingGitSignature {
        PendingGitSignature {
            name: name.to_owned(),
            email: format!("{name}@example.test"),
            time_seconds: 123,
            timezone_offset_minutes: 600,
            extensions: BTreeMap::new(),
        }
    }

    fn pending(kind: PendingMergeActionKind) -> PendingMergeAction {
        PendingMergeAction {
            kind,
            target_branch: "main".to_owned(),
            before_commit: "before".to_owned(),
            source_commit: "source".to_owned(),
            commit_message: "message".to_owned(),
            expected_result: None,
            commit_spec: None,
            extensions: BTreeMap::new(),
        }
    }

    #[test]
    fn decoder_reconstructs_complete_commit_without_environmental_input() {
        let mut value = pending(PendingMergeActionKind::ResolveConflict);
        value.expected_result = Some(PendingMergeExpectedResult::Commit);
        value.commit_spec = Some(PendingCommitSpec {
            tree_oid: "tree".to_owned(),
            author: signature("author"),
            committer: signature("committer"),
            extensions: BTreeMap::new(),
        });

        assert_eq!(
            decode_durable_prepared_action(&value),
            Ok(DurablePreparedAction::Resolution(GitPreparedCommit {
                tree_oid: "tree".to_owned(),
                author: GitPreparedSignature {
                    name: "author".to_owned(),
                    email: "author@example.test".to_owned(),
                    time_seconds: 123,
                    timezone_offset_minutes: 600,
                },
                committer: GitPreparedSignature {
                    name: "committer".to_owned(),
                    email: "committer@example.test".to_owned(),
                    time_seconds: 123,
                    timezone_offset_minutes: 600,
                },
            }))
        );
    }

    #[test]
    fn decoder_rejects_weak_commit_producing_records() {
        let mut value = pending(PendingMergeActionKind::TrueMerge);
        value.expected_result = Some(PendingMergeExpectedResult::Commit);

        assert!(decode_durable_prepared_action(&value).is_err());
    }
}
