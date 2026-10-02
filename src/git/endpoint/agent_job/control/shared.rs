use super::super::super::ssh_setup_context::SetupContext;
use super::*;
use gwz_transport::pool::{Observation, PublicationError, SetupCause};

impl Control {
    pub(crate) fn new_shared(
        setup: Arc<SetupContext>,
        cleanup: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> Self {
        let mut control = Self::new(None, Duration::ZERO, cleanup, clock);
        control.setup = Some(setup);
        control
    }
    pub(in crate::git::endpoint::agent_job) fn sync_shared(&self) {
        let Some(setup) = &self.setup else {
            return;
        };
        let Observation::Terminal(record) = setup.clock.observe().deliver() else {
            return;
        };
        if record.cause == SetupCause::Completed {
            return;
        }
        let error = setup.error(record);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.consumed && state.failure.is_none() {
            state.failure = Some(Fail {
                kind: error.kind(),
                reason: None,
                terminal: Some(record),
            });
            state.cancelled_at = Some(self.now());
        }
    }
    pub(super) fn check_shared(&self) -> io::Result<()> {
        self.sync_shared();
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .failure
            .map_or(Ok(()), |failure| Err(failure.into_io()))
    }
    pub(super) fn shared_publication_error(&self, error: PublicationError) -> io::Error {
        match error {
            PublicationError::ActiveTerminal(record)
            | PublicationError::PreparationExpired(record) => {
                self.setup.as_ref().expect("shared control").error(record)
            }
            // Local work must not manufacture network progress.
            _ => io::ErrorKind::InvalidInput.into(),
        }
    }
}
