use super::*;

/// A reading of the clock that bounds a setup, for measuring what a stretch of the setup spent.
#[derive(Clone, Copy)]
pub(crate) enum Mark {
    /// The job's own clock.
    Local(Instant),
    /// The shared setup clock's milliseconds.
    Shared(u64),
}

impl Control {
    /// The clock that bounds the setup, read now.
    pub(crate) fn mark(&self) -> Mark {
        match &self.setup {
            Some(setup) => Mark::Shared(setup.clock.deadline_after(0).deliver().unwrap_or(0)),
            None => Mark::Local(self.now()),
        }
    }

    /// Whether the setup's budget (the nearer of its aggregate and stall bounds) left now is more than what was
    /// spent since `since`: room for a second try of what took that long. A setup with no bound has room.
    pub(crate) fn has_room_since(&self, since: Mark) -> bool {
        let (spent, left) = match (&self.setup, since) {
            (Some(setup), Mark::Shared(then)) => {
                // An ended setup has no clock reading left.
                let Ok(now) = setup.clock.deadline_after(0).deliver() else {
                    return false;
                };
                let left = setup.clock.remaining().deliver().map(Duration::from_millis);
                (Duration::from_millis(now.saturating_sub(then)), left)
            }
            (None, Mark::Local(then)) => {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let left = [
                    self.aggregate_remaining(&state),
                    self.stall_remaining(&state),
                ]
                .into_iter()
                .flatten()
                .min();
                (self.now().saturating_duration_since(then), left)
            }
            _ => return false,
        };
        left.is_none_or(|left| left > spent)
    }
}
