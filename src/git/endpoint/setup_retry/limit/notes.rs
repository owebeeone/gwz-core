//! The user notes of §9, as data. The endpoint names the host and prints.

/// The coalescing window of a changed-limit note.
pub(crate) const CHANGE_NOTE_MS: u64 = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Note {
    /// The host refused extra connections; using `n` (ceiling `ceiling`).
    Overload { n: usize, ceiling: usize },
    /// The limit changed later: now using `n`.
    Changed { n: usize },
    /// Back at the ceiling.
    BackAtCeiling { ceiling: usize },
    /// A `Retry-After` hold longer than 1 s began.
    Hold { wait_ms: u64 },
}

#[derive(Default)]
pub(crate) struct Notes {
    out: Vec<Note>,
    overload_pending: bool,
    overload_noted: bool,
    lowered: bool,
    change_pending: Option<usize>,
    last_at: u64,
    last_n: usize,
}

impl Notes {
    /// An Overload was taken.
    pub(crate) fn overload(&mut self) {
        self.lowered = true;
        self.overload_pending |= !self.overload_noted;
    }
    /// `N` changed to `n`.
    pub(crate) fn n_changed(&mut self, n: usize) {
        if self.overload_noted {
            self.change_pending = Some(n);
        }
    }
    /// The machine is SATURATED. After a decrease that was noted, say so; one
    /// that was never noted is cancelled.
    pub(crate) fn saturated(&mut self, ceiling: usize) {
        self.change_pending = None;
        if self.overload_pending {
            self.overload_pending = false;
        } else if self.lowered {
            self.out.push(Note::BackAtCeiling { ceiling });
        }
        self.lowered = false;
    }
    pub(crate) fn hold(&mut self, wait_ms: u64) {
        self.out.push(Note::Hold { wait_ms });
    }
    /// Emits what is ready: the overload note once its wave has resolved,
    /// and a changed-limit note at most once per 2 s.
    pub(crate) fn flush(&mut self, now: u64, n: usize, ceiling: usize, wave_resolved: bool) {
        if self.overload_pending && wave_resolved {
            self.overload_pending = false;
            self.overload_noted = true;
            self.change_pending = None;
            (self.last_at, self.last_n) = (now, n);
            self.out.push(Note::Overload { n, ceiling });
        } else if let Some(changed) = self.change_pending
            && now >= self.last_at.saturating_add(CHANGE_NOTE_MS)
        {
            self.change_pending = None;
            if changed != self.last_n {
                (self.last_at, self.last_n) = (now, changed);
                self.out.push(Note::Changed { n: changed });
            }
        }
    }
    /// When a coalesced change note may be said.
    pub(crate) fn deadline(&self) -> Option<u64> {
        self.change_pending
            .map(|_| self.last_at.saturating_add(CHANGE_NOTE_MS))
    }
    pub(crate) fn drain(&mut self) -> Vec<Note> {
        std::mem::take(&mut self.out)
    }
}
