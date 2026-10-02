//! Bounded logical association; it never owns or waits for physical cleanup.
use super::*;
use gwz_transport::pool::ClockUpdate;

pub(super) struct Publication<'a> {
    context: &'a SetupContext,
    failed: Option<Failure>,
    cause: SetupCause,
    update: Option<ClockUpdate<Result<SetupTerminal, SetupTerminal>>>,
}
impl<'a> Publication<'a> {
    pub(super) fn new(context: &'a SetupContext, failed: Failure) -> Self {
        let cause = SetupCause::ResourceFailure {
            code: failed.code,
            effect: failed.effect,
            setup_cause: failed.setup_cause,
        };
        let mut state = context.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.publishing {
            state = context
                .published
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        state.publishing = true;
        drop(state);
        Self {
            context,
            failed: Some(failed),
            cause,
            update: None,
        }
    }
    pub(super) fn commit(&mut self) -> SetupTerminal {
        // No core lock crosses the authority call. Do not deliver its wakes
        // until Drop has made the detailed association visible to all readers.
        self.update = Some(self.context.clock.terminate_if_alive(self.cause));
        match self.update.as_ref().expect("committed publication").value {
            Ok(record) | Err(record) => record,
        }
    }
}
impl Drop for Publication<'_> {
    fn drop(&mut self) {
        let mut state = self.context.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(update) = &self.update
            && let Ok(record) = update.value
            && state.first.is_none()
        {
            state.first = self.failed.take().map(|failed| (record, failed));
        }
        state.publishing = false;
        drop(state);
        self.context.published.notify_all();
        if let Some(update) = self.update.take() {
            let _ = update.deliver();
        }
    }
}
