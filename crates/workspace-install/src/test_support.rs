//! A recording, scripted [`InstallPorts`] fake for orchestration tests.

use std::path::Path;

use crate::{
    ConfigurationPlan, ConstructionRequest, InstallPortError, InstallPorts, SourceSnapshot,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallPortCall {
    SnapshotSource,
    RecheckSource,
    ConstructRepositories,
    InstallConfiguration,
}

#[derive(Debug, Default)]
pub struct RecordingInstallPorts {
    calls: Vec<InstallPortCall>,
    snapshot: Option<SourceSnapshot>,
    failures: Vec<(InstallPortCall, InstallPortError)>,
}

impl RecordingInstallPorts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> &[InstallPortCall] {
        &self.calls
    }

    /// Script the snapshot `snapshot_source` answers with.
    pub fn snapshot(&mut self, snapshot: SourceSnapshot) {
        self.snapshot = Some(snapshot);
    }

    pub fn fail_next(&mut self, call: InstallPortCall, error: InstallPortError) {
        self.failures.push((call, error));
    }

    fn record(&mut self, call: InstallPortCall) -> Result<(), InstallPortError> {
        self.calls.push(call.clone());
        if let Some(index) = self.failures.iter().position(|(name, _)| *name == call) {
            return Err(self.failures.remove(index).1);
        }
        Ok(())
    }
}

impl InstallPorts for RecordingInstallPorts {
    fn snapshot_source(&mut self, _source: &Path) -> Result<SourceSnapshot, InstallPortError> {
        self.record(InstallPortCall::SnapshotSource)?;
        self.snapshot
            .clone()
            .ok_or(InstallPortError::Unimplemented {
                operation: "unscripted snapshot_source",
            })
    }

    fn recheck_source(&mut self, _snapshot: &SourceSnapshot) -> Result<(), InstallPortError> {
        self.record(InstallPortCall::RecheckSource)
    }

    fn construct_repositories(
        &mut self,
        _request: &ConstructionRequest,
    ) -> Result<(), InstallPortError> {
        self.record(InstallPortCall::ConstructRepositories)
    }

    fn install_configuration(&mut self, _plan: &ConfigurationPlan) -> Result<(), InstallPortError> {
        self.record(InstallPortCall::InstallConfiguration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unscripted_snapshot_fails_typed_and_failures_are_consumed_in_order() {
        let mut ports = RecordingInstallPorts::new();
        assert!(matches!(
            ports.snapshot_source(Path::new("/src")),
            Err(InstallPortError::Unimplemented { .. })
        ));
        ports.fail_next(
            InstallPortCall::RecheckSource,
            InstallPortError::Drift {
                detail: "HEAD moved".to_owned(),
            },
        );
        let snapshot = SourceSnapshot {
            repositories: Vec::new(),
            configuration_digest: "d".to_owned(),
        };
        assert!(ports.recheck_source(&snapshot).is_err());
        assert!(ports.recheck_source(&snapshot).is_ok());
        assert_eq!(ports.calls().len(), 3);
    }
}
