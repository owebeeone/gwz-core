//! A `Retry-After` hold (§4.5): no new connection starts on the key, and no
//! open begins its first exchange on a leased one.

pub(crate) const HOLD_CAP_MS: u64 = 30_000;
/// A hold longer than this discards the key's idle connections when it ends.
pub(crate) const DISCARD_AFTER_MS: u64 = 1_000;

/// What set the hold: a discovery response, or a POST response (which also
/// holds the continuing members' next POST).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Discovery,
    Post,
}

#[derive(Clone)]
struct Active {
    until: u64,
    origin: Origin,
    /// Longer than 1 s: its end discards the key's idle connections.
    long: bool,
    discard_asked: bool,
}

#[derive(Clone, Default)]
pub(crate) struct Hold {
    active: Option<Active>,
}

impl Hold {
    /// Sets or extends the hold to `now + min(retry_after_ms, 30 s)`. Returns
    /// the wait in ms when a hold longer than 1 s began, for the note.
    pub(crate) fn set(&mut self, retry_after_ms: u64, origin: Origin, now: u64) -> Option<u64> {
        let wait = retry_after_ms.min(HOLD_CAP_MS);
        let until = now.saturating_add(wait);
        if self.in_force(now)
            && let Some(active) = &mut self.active
        {
            active.until = active.until.max(until);
            active.long |= wait > DISCARD_AFTER_MS;
            if origin == Origin::Post {
                active.origin = Origin::Post;
            }
            return None;
        }
        let long = wait > DISCARD_AFTER_MS;
        self.active = Some(Active {
            until,
            origin,
            long,
            discard_asked: false,
        });
        long.then_some(wait)
    }
    pub(crate) fn in_force(&self, now: u64) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| now < active.until || active.long)
    }
    /// Whether a member past its discovery may not send its next exchange.
    pub(crate) fn blocks_continuing(&self, now: u64) -> bool {
        self.in_force(now)
            && self
                .active
                .as_ref()
                .is_some_and(|a| a.origin == Origin::Post)
    }
    /// True once, when a long hold has run its time: the caller discards the
    /// key's idle connections and then calls [`Hold::idle_discarded`]. The
    /// hold lifts only after that.
    pub(crate) fn discard_due(&mut self, now: u64) -> bool {
        match &mut self.active {
            Some(active) if active.long && now >= active.until && !active.discard_asked => {
                active.discard_asked = true;
                true
            }
            _ => false,
        }
    }
    pub(crate) fn idle_discarded(&mut self) {
        self.active = None;
    }
    /// When the hold ends, or is next looked at.
    pub(crate) fn deadline(&self, now: u64) -> Option<u64> {
        match &self.active {
            Some(active) if active.until > now => Some(active.until),
            Some(active) if active.long => Some(now),
            _ => None,
        }
    }
}
