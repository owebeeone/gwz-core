//! A recording, scripted [`DisposalPorts`] fake.

use std::path::{Path, PathBuf};

use crate::{
    DisposalPorts, HistoryAnswer, HistoryQuery, PortError, RemovalFailure, TargetEvidence,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposalCall {
    ObserveTarget { target: PathBuf },
    CheckHistory { query: HistoryQuery },
    RemoveDirectory { target: PathBuf },
}

#[derive(Debug, Default)]
pub struct RecordingDisposalPorts {
    calls: Vec<DisposalCall>,
    evidence: Option<Result<TargetEvidence, PortError>>,
    history: Option<HistoryAnswer>,
    /// Answers for the next calls, in order, before `history` applies.
    history_queue: Vec<HistoryAnswer>,
    removal: Option<RemovalFailure>,
}

impl RecordingDisposalPorts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> &[DisposalCall] {
        &self.calls
    }

    pub fn evidence(&mut self, evidence: TargetEvidence) {
        self.evidence = Some(Ok(evidence));
    }

    /// Make `observe_target` fail instead of answering.
    pub fn fail_evidence(&mut self, error: PortError) {
        self.evidence = Some(Err(error));
    }

    /// The answer every unqueued `check_history` call gets.
    pub fn history(&mut self, answer: HistoryAnswer) {
        self.history = Some(answer);
    }

    /// Answer the next calls with these, in order; later calls fall back to
    /// [`history`](Self::history). The deletion tree holds one repository
    /// per call, so a scripted sequence is how a consumer drives a mixed
    /// tree.
    pub fn history_sequence(&mut self, answers: impl IntoIterator<Item = HistoryAnswer>) {
        let mut queue: Vec<HistoryAnswer> = answers.into_iter().collect();
        queue.reverse();
        self.history_queue = queue;
    }

    /// Make the next removal stop with this failure.
    pub fn fail_removal(&mut self, failure: RemovalFailure) {
        self.removal = Some(failure);
    }
}

impl DisposalPorts for RecordingDisposalPorts {
    fn observe_target(&mut self, target: &Path) -> Result<TargetEvidence, PortError> {
        self.calls.push(DisposalCall::ObserveTarget {
            target: target.to_path_buf(),
        });
        self.evidence
            .clone()
            .unwrap_or(Err(PortError::Unimplemented {
                operation: "unscripted observe_target",
            }))
    }

    fn check_history(&mut self, query: &HistoryQuery) -> HistoryAnswer {
        self.calls.push(DisposalCall::CheckHistory {
            query: query.clone(),
        });
        self.history_queue
            .pop()
            .or_else(|| self.history.clone())
            .unwrap_or(HistoryAnswer::Unknown {
                reasons: vec![gwz_repo_contract::UnknownReason::unimplemented(
                    "unscripted check_history",
                )],
            })
    }

    fn remove_directory(&mut self, target: &Path) -> Result<(), RemovalFailure> {
        self.calls.push(DisposalCall::RemoveDirectory {
            target: target.to_path_buf(),
        });
        match self.removal.take() {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_answers_are_served_in_order_then_fall_back() {
        let mut ports = RecordingDisposalPorts::new();
        ports.fail_evidence(PortError::Evidence {
            detail: "the target could not be listed".to_owned(),
        });
        assert!(matches!(
            ports.observe_target(Path::new("/ws-A")),
            Err(PortError::Evidence { .. })
        ));
        ports.history(HistoryAnswer::Preserved);
        ports.history_sequence([
            HistoryAnswer::Unpreserved {
                detail: "first".to_owned(),
            },
            HistoryAnswer::Unknown {
                reasons: Vec::new(),
            },
        ]);
        let query = HistoryQuery {
            target: gwz_repo_contract::RepoKey::Root,
            protected: gwz_repo_contract::ProtectedRoots::default(),
        };
        assert!(matches!(
            ports.check_history(&query),
            HistoryAnswer::Unpreserved { .. }
        ));
        assert!(matches!(
            ports.check_history(&query),
            HistoryAnswer::Unknown { .. }
        ));
        assert_eq!(
            ports.check_history(&query),
            HistoryAnswer::Preserved,
            "the queue is exhausted, so the standing answer applies"
        );
    }

    #[test]
    fn unscripted_answers_are_unknown_or_typed_never_clean() {
        let mut ports = RecordingDisposalPorts::new();
        assert!(matches!(
            ports.observe_target(Path::new("/ws-A")),
            Err(PortError::Unimplemented { .. })
        ));
        let query = HistoryQuery {
            target: gwz_repo_contract::RepoKey::Root,
            protected: gwz_repo_contract::ProtectedRoots::default(),
        };
        assert!(matches!(
            ports.check_history(&query),
            HistoryAnswer::Unknown { .. }
        ));
        ports.fail_removal(RemovalFailure {
            error: PortError::Removal {
                path: PathBuf::from("/ws-A/x"),
                detail: "busy".to_owned(),
            },
            remaining: vec![PathBuf::from("/ws-A/x")],
        });
        assert!(ports.remove_directory(Path::new("/ws-A")).is_err());
        assert!(ports.remove_directory(Path::new("/ws-A")).is_ok());
        assert_eq!(ports.calls().len(), 4);
    }
}
