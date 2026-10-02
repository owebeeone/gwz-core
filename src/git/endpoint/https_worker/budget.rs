use super::*;

impl Budget {
    pub(crate) fn shorten(&mut self, cap: Self) {
        fn bounded(value: Option<Duration>, cap: Option<Duration>) -> Option<Duration> {
            match (value, cap) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        }
        self.allocation = self.allocation.min(cap.allocation);
        self.helper = self.helper.min(cap.helper);
        self.connect = bounded(self.connect, cap.connect);
        self.network = bounded(self.network, cap.network);
        self.cleanup = self.cleanup.min(cap.cleanup);
    }
}
impl Client {
    pub(crate) fn budget(&self) -> Budget {
        budget_for_config(&self.config, self.io_timeout_ms)
    }
}

fn budget_for_config(config: &pool::Config, io_timeout_ms: u64) -> Budget {
    Budget {
        allocation: Duration::from_millis(config.allocation_timeout_ms),
        helper: Duration::from_millis(config.interaction_timeout_ms),
        connect: (config.connect_timeout_ms != 0)
            .then(|| Duration::from_millis(config.connect_timeout_ms)),
        // Active I/O consumes one cumulative budget across redirects and
        // authentication attempts; zero deliberately disables the deadline.
        network: (io_timeout_ms != 0).then(|| Duration::from_millis(io_timeout_ms)),
        cleanup: Duration::from_millis(config.cleanup_timeout_ms),
    }
}
impl Client {
    pub(crate) fn budget_for_open(&self, deadlines: &Deadlines) -> Budget {
        let mut budget = self.budget();
        if deadlines.allocation_ms > 0 {
            budget.allocation = budget
                .allocation
                .min(Duration::from_millis(deadlines.allocation_ms as u64));
        }
        if deadlines.connect_ms > 0 {
            let requested = Duration::from_millis(deadlines.connect_ms as u64);
            budget.connect = Some(
                budget
                    .connect
                    .map_or(requested, |current| current.min(requested)),
            );
        }
        if deadlines.interaction_ms >= 0 {
            budget.helper = budget
                .helper
                .min(Duration::from_millis(deadlines.interaction_ms as u64));
        }
        if deadlines.io_ms > 0 {
            let requested = Duration::from_millis(deadlines.io_ms as u64);
            budget.network = Some(
                budget
                    .network
                    .map_or(requested, |current| current.min(requested)),
            );
        }
        if deadlines.cleanup_ms > 0 {
            budget.cleanup = budget
                .cleanup
                .min(Duration::from_millis(deadlines.cleanup_ms as u64));
        }
        budget
    }
}
