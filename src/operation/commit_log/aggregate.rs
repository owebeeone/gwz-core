use std::collections::BTreeMap;

use super::coalesce::CommitLogGroup;
use super::{CommitLogDegradation, CommitLogDegradationKind, CommitLogTarget};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommitLogMemberOutcomeKind {
    Empty,
    Contributed,
    Degraded,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CommitLogMemberOutcome {
    target: CommitLogTarget,
    kind: CommitLogMemberOutcomeKind,
}

impl CommitLogMemberOutcome {
    pub(super) fn target(&self) -> &CommitLogTarget {
        &self.target
    }

    pub(super) fn kind(&self) -> CommitLogMemberOutcomeKind {
        self.kind
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CommitLogAggregate {
    outcomes: Vec<CommitLogMemberOutcome>,
    status: crate::AggregateStatus,
}

impl CommitLogAggregate {
    pub(super) fn outcomes(&self) -> &[CommitLogMemberOutcome] {
        &self.outcomes
    }

    pub(super) fn status(&self) -> crate::AggregateStatus {
        self.status
    }
}

pub(super) struct CommitLogAggregateCollector {
    outcomes: Vec<CommitLogMemberOutcome>,
    index: BTreeMap<String, usize>,
    strict: bool,
    degradation_seen: bool,
}

impl CommitLogAggregateCollector {
    pub(super) fn new(targets: Vec<CommitLogTarget>, strict: bool) -> Self {
        let index = targets
            .iter()
            .enumerate()
            .map(|(index, target)| (target.member_id.clone(), index))
            .collect();
        Self {
            outcomes: targets
                .into_iter()
                .map(|target| CommitLogMemberOutcome {
                    target,
                    kind: CommitLogMemberOutcomeKind::Empty,
                })
                .collect(),
            index,
            strict,
            degradation_seen: false,
        }
    }

    pub(super) fn observe_group(&mut self, group: &CommitLogGroup) {
        for entry in group.entries() {
            let outcome = &mut self.outcomes[self.index[&entry.target.member_id]];
            if outcome.kind == CommitLogMemberOutcomeKind::Empty {
                outcome.kind = CommitLogMemberOutcomeKind::Contributed;
            }
        }
    }

    pub(super) fn observe_degradation(&mut self, record: &CommitLogDegradation) {
        self.degradation_seen = true;
        let kind = if is_read_failure(record.kind) {
            CommitLogMemberOutcomeKind::Failed
        } else {
            CommitLogMemberOutcomeKind::Degraded
        };
        let outcome = &mut self.outcomes[self.index[&record.target.member_id]];
        if outcome.kind != CommitLogMemberOutcomeKind::Failed {
            outcome.kind = kind;
        }
    }

    pub(super) fn finish(self) -> CommitLogAggregate {
        let any_failed = self
            .outcomes
            .iter()
            .any(|outcome| outcome.kind == CommitLogMemberOutcomeKind::Failed);
        let any_contributed = self
            .outcomes
            .iter()
            .any(|outcome| outcome.kind == CommitLogMemberOutcomeKind::Contributed);
        CommitLogAggregate {
            outcomes: self.outcomes,
            status: if self.strict && self.degradation_seen {
                crate::AggregateStatus::Failed
            } else if any_failed && any_contributed {
                crate::AggregateStatus::Partial
            } else if any_failed {
                crate::AggregateStatus::Failed
            } else {
                crate::AggregateStatus::Ok
            },
        }
    }
}

fn is_read_failure(kind: CommitLogDegradationKind) -> bool {
    matches!(
        kind,
        CommitLogDegradationKind::UnsupportedSourceKind
            | CommitLogDegradationKind::RepositoryUnreadable
            | CommitLogDegradationKind::HistoryUnreadable
    )
}
