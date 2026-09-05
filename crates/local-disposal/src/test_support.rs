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
    evidence: Option<TargetEvidence>,
    history: Option<HistoryAnswer>,
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
        self.evidence = Some(evidence);
    }

    pub fn history(&mut self, answer: HistoryAnswer) {
        self.history = Some(answer);
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
        self.evidence.clone().ok_or(PortError::Unimplemented {
            operation: "unscripted observe_target",
        })
    }

    fn check_history(&mut self, query: &HistoryQuery) -> HistoryAnswer {
        self.calls.push(DisposalCall::CheckHistory {
            query: query.clone(),
        });
        self.history.clone().unwrap_or(HistoryAnswer::Unknown {
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
