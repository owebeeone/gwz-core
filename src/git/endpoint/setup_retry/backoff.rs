//! The waits between setup attempts (the retry plan's §5) and the jitter added
//! to each.

/// Jitter is drawn from `0..JITTER_MS`.
pub(crate) const JITTER_MS: u64 = 250;
const FIRST_WAIT_MS: u64 = 1_000;
/// The cap is the aggregate default, 30 s. It is not a separate setting.
const MAX_WAIT_MS: u64 = 30_000;

/// The wait after attempt `attempt` (1 through R) fails: 1 s, doubling, and
/// never past 30 s. Jitter is added separately.
pub(crate) fn wait_ms(attempt: u32) -> u64 {
    match attempt.saturating_sub(1) {
        doublings @ 0..=14 => (FIRST_WAIT_MS << doublings).min(MAX_WAIT_MS),
        _ => MAX_WAIT_MS,
    }
}

/// The most that every wait of `max_retries` attempts adds, jitter included:
/// the waits after attempts 1 through R.
pub(crate) fn wait_bound_ms(max_retries: u32) -> u64 {
    // From the sixth wait on, each is the 30 s cap.
    let doubling = max_retries.min(5);
    let doubled: u64 = (1..=doubling).map(wait_ms).sum();
    let capped = u64::from(max_retries - doubling).saturating_mul(MAX_WAIT_MS);
    doubled
        .saturating_add(capped)
        .saturating_add(u64::from(max_retries).saturating_mul(JITTER_MS))
}

/// A draw of jitter for each wait. Production draws from the operating
/// system's random source; tests supply a fixed value.
pub(crate) struct Jitter(Box<dyn FnMut() -> u64 + Send>);

impl Jitter {
    pub(crate) fn random() -> Self {
        Self(Box::new(|| {
            getrandom::u64().map_or(0, |value| value % JITTER_MS)
        }))
    }
    pub(crate) fn draw(&mut self) -> u64 {
        (self.0)().min(JITTER_MS - 1)
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl Jitter {
            pub(crate) fn fixed(milliseconds: u64) -> Self {
                Self(Box::new(move || milliseconds))
            }
        }
    }
}
