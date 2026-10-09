//! The probe timer (§4.7): `T0` doubling to `Tmax`, each expiry drawn with
//! jitter. Time is a number the caller supplies.

pub(crate) const T0_MS: u64 = 500;
pub(crate) const TMAX_MS: u64 = 30_000;

/// The jitter factor of each expiry, in thousandths: 800 to 1200.
pub(crate) struct Spread(Box<dyn FnMut() -> u64 + Send>);

impl Spread {
    pub(crate) fn new(draw: impl FnMut() -> u64 + Send + 'static) -> Self {
        Self(Box::new(draw))
    }
    pub(crate) fn fixed(permille: u64) -> Self {
        Self::new(move || permille)
    }
    fn draw(&mut self) -> u64 {
        (self.0)().clamp(800, 1_200)
    }
}

pub(crate) struct Timer {
    spread: Spread,
    gap: u64,
    deadline: Option<u64>,
}

impl Timer {
    pub(crate) fn new(spread: Spread) -> Self {
        Self {
            spread,
            gap: T0_MS,
            deadline: None,
        }
    }
    /// `T := T0`.
    pub(crate) fn reset(&mut self) {
        self.gap = T0_MS;
    }
    /// `T := min(2T, Tmax)`.
    pub(crate) fn back_off(&mut self) {
        self.gap = self.gap.saturating_mul(2).min(TMAX_MS);
    }
    /// The next expiry: `now` plus the gap in force, jittered.
    pub(crate) fn arm(&mut self, now: u64) {
        let jittered = self.gap.saturating_mul(self.spread.draw()) / 1_000;
        self.deadline = Some(now.saturating_add(jittered));
    }
    /// Expired already: a test that must run again when the key is quiet.
    pub(crate) fn make_due(&mut self, now: u64) {
        self.deadline = Some(now);
    }
    pub(crate) fn disarm(&mut self) {
        self.deadline = None;
    }
    pub(crate) fn deadline(&self) -> Option<u64> {
        self.deadline
    }
    pub(crate) fn expired(&self, now: u64) -> bool {
        self.deadline.is_some_and(|deadline| deadline <= now)
    }
}
