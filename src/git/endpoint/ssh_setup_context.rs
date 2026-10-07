//! Core-owned timing provenance and first sanitized setup failure.
use gwz_transport::{
    pool::{
        LocalPhase, Observation, PhaseId, PublicationError, SetupCause, SetupClock, SetupTerminal,
    },
    protocol::{Effect, ErrorCode, Failure, FailureDetail, SetupFailureCause},
};
use std::{
    io,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct Witness {
    phase: PhaseId,
    cause: SetupFailureCause,
    milliseconds: i64,
}
#[derive(Default)]
struct State {
    witnesses: [Option<Witness>; 2],
    first: Option<(SetupTerminal, Failure)>,
    publishing: bool,
}
pub(crate) struct SetupContext {
    pub(crate) clock: SetupClock,
    pub(crate) origin: Instant,
    connection: gwz_transport::pool::ConnectionId,
    state: Mutex<State>,
    published: Condvar,
}

#[derive(Debug)]
pub(crate) struct SetupEnded(pub(crate) SetupTerminal);
impl std::fmt::Display for SetupEnded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SSH setup authority ended")
    }
}
impl std::error::Error for SetupEnded {}
impl SetupContext {
    pub(crate) fn new(clock: SetupClock, origin: Instant) -> Arc<Self> {
        let connection = clock.connection();
        Arc::new(Self {
            clock,
            origin,
            connection,
            state: Mutex::new(State::default()),
            published: Condvar::new(),
        })
    }
    pub(crate) fn failure(&self, record: SetupTerminal) -> Failure {
        if record.connection != self.connection {
            return Failure {
                code: ErrorCode::InvalidRequest,
                effect: Effect::None,
                ..Default::default()
            };
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // A detailed publisher reserves its association before the clock can
        // expose the terminal. Wait for that logical association only: no
        // authority lock is held here and physical cleanup is independent.
        while state.publishing {
            state = self
                .published
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        if let Some((first, failed)) = &state.first
            && *first == record
        {
            return failed.clone();
        }
        let mut failed = Failure {
            code: ErrorCode::Io,
            effect: Effect::None,
            ..Default::default()
        };
        match record.cause {
            SetupCause::LocalDeadline | SetupCause::PreparationDeadline => {
                failed.code = ErrorCode::Timeout;
                if let Some(witness) = state
                    .witnesses
                    .iter()
                    .flatten()
                    .find(|witness| witness.phase == record.phase)
                {
                    failed.setup_cause = Some(witness.cause);
                    failed.detail = Some(Box::new(FailureDetail {
                        helper_budget_ms: Some(witness.milliseconds),
                        ..Default::default()
                    }));
                }
            }
            SetupCause::NetworkAggregate | SetupCause::NetworkStall => {
                failed.code = ErrorCode::Timeout;
                failed.setup_cause = Some(if record.cause == SetupCause::NetworkAggregate {
                    SetupFailureCause::Aggregate
                } else {
                    SetupFailureCause::Stall
                });
            }
            // The setup's own wait for a local budget (a job place, a shared
            // reservation) ran out the open's allocation: a local failure with
            // no server cause, which the retry classifier returns.
            SetupCause::LocalWaitExpired => failed.code = ErrorCode::Capacity,
            SetupCause::Cancelled => failed.code = ErrorCode::Cancelled,
            SetupCause::DriverLost => failed.code = ErrorCode::CarrierLost,
            SetupCause::ResourceFailure {
                code,
                effect,
                setup_cause,
            } => {
                failed.code = code;
                failed.effect = effect;
                failed.setup_cause = setup_cause;
            }
            SetupCause::Completed => failed.code = ErrorCode::InvalidRequest,
        }
        if state.first.is_none() {
            state.first = Some((record, failed.clone()));
        }
        failed
    }
    pub(crate) fn check(&self) -> io::Result<()> {
        match self.clock.observe().deliver() {
            Observation::Alive { .. } => Ok(()),
            Observation::Terminal(record) if record.cause == SetupCause::Completed => Ok(()),
            Observation::Terminal(record) => Err(self.error(record)),
        }
    }
    pub(crate) fn error(&self, record: SetupTerminal) -> io::Error {
        let kind = match record.cause {
            SetupCause::NetworkAggregate
            | SetupCause::NetworkStall
            | SetupCause::LocalDeadline
            | SetupCause::PreparationDeadline => io::ErrorKind::TimedOut,
            SetupCause::LocalWaitExpired => io::ErrorKind::WouldBlock,
            SetupCause::Cancelled | SetupCause::DriverLost => io::ErrorKind::ConnectionAborted,
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, SetupEnded(record))
    }
    pub(crate) fn terminate_failure(&self, failed: Failure) -> io::Error {
        let mut publication = publication::Publication::new(self, failed);
        let record = publication.commit();
        drop(publication); // associate first, notify/wake outside both locks
        self.error(record)
    }
    pub(crate) async fn enter(&self, kind: LocalPhase, milliseconds: u64) -> io::Result<Instant> {
        let cause = match kind {
            LocalPhase::Admission => SetupFailureCause::Allocation,
            LocalPhase::Interaction => SetupFailureCause::Interaction,
            // A wait on a local budget is entered by the pool on the resource's
            // report, never by a setup.
            LocalPhase::Wait => return Err(self.invalid()),
        };
        if milliseconds == 0 && kind == LocalPhase::Admission {
            return Err(self.terminate_failure(Failure {
                code: ErrorCode::Timeout,
                effect: Effect::None,
                setup_cause: Some(cause),
                detail: Some(Box::new(FailureDetail {
                    helper_budget_ms: Some(0),
                    ..Default::default()
                })),
                facts: None,
            }));
        }
        let until = self
            .clock
            .deadline_after(milliseconds)
            .deliver()
            .map_err(|error| self.publication_error(error))?;
        let prepared = self
            .clock
            .prepare_local(kind, until)
            .deliver()
            .map_err(|error| self.publication_error(error))?;
        let index = if kind == LocalPhase::Admission { 0 } else { 1 };
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.witnesses[index].is_some() {
                drop(state);
                return Err(self.invalid());
            }
            state.witnesses[index] = Some(Witness {
                phase: prepared.phase(),
                cause,
                milliseconds: milliseconds as i64,
            });
        }
        let receipt = self
            .clock
            .publish_local(prepared)
            .deliver()
            .map_err(|error| self.publication_error(error))?;
        self.clock
            .wait_acknowledged(receipt)
            .await
            .map_err(|error| self.publication_error(error))?;
        self.check()?;
        self.origin
            .checked_add(Duration::from_millis(until))
            .ok_or_else(|| self.invalid())
    }
    pub(crate) async fn resume(&self) -> io::Result<()> {
        let receipt = self
            .clock
            .publish_network()
            .deliver()
            .map_err(|error| self.publication_error(error))?;
        self.clock
            .wait_acknowledged(receipt)
            .await
            .map_err(|error| self.publication_error(error))?;
        self.check()
    }
    fn invalid(&self) -> io::Error {
        self.terminate_failure(Failure {
            code: ErrorCode::InvalidRequest,
            effect: Effect::None,
            ..Default::default()
        })
    }
    fn publication_error(&self, error: PublicationError) -> io::Error {
        match error {
            PublicationError::ActiveTerminal(record)
            | PublicationError::PreparationExpired(record) => self.error(record),
            _ => self.invalid(),
        }
    }
}

mod publication;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
