//! The user notes of §9: data from the machine, and the words they are
//! printed in, with the host and port of the site they are about. The reader
//! (the CLI) decides whether to print them at all.
use gwz_transport::pool::Site;

/// The coalescing window of a changed-limit note.
pub(crate) const CHANGE_NOTE_MS: u64 = 2_000;

/// A site's host and port as a message names it: `github.com:443`, and
/// `[::1]:443` for an IPv6 host.
pub(crate) fn site_name(site: &Site) -> String {
    if site.host.contains(':') {
        format!("[{}]:{}", site.host, site.port)
    } else {
        format!("{}:{}", site.host, site.port)
    }
}

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
    /// The host stopped answering and its members are parked (§5.5).
    Down,
}

/// Milliseconds between a Down key's retests, which the note names.
const DOWN_RETEST_SECONDS: u64 = 30;

impl Note {
    /// The note as one line of stderr: `note: ` and the words of §9, naming
    /// the site's host and port, since SSH and HTTPS to one host are separate
    /// limits.
    pub(crate) fn render(&self, site: &Site) -> String {
        let host = site_name(site);
        match self {
            Self::Overload { n, ceiling } => format!(
                "note: {host} refused extra connections; using {n} at a time (ceiling {ceiling} from --max-per-host); continuing, no action needed"
            ),
            Self::Changed { n } => {
                let noun = if *n == 1 { "connection" } else { "connections" };
                format!("note: {host}: now using {n} {noun}")
            }
            Self::BackAtCeiling { ceiling } => {
                format!("note: {host}: back at the ceiling of {ceiling} connections")
            }
            // A part of a second is a whole one: the wait is never understated.
            Self::Hold { wait_ms } => format!(
                "note: {host} asked gwz to wait {} s; waiting",
                wait_ms.div_ceil(1_000)
            ),
            Self::Down => format!(
                "note: {host} is not answering; waiting up to {DOWN_RETEST_SECONDS} s to retry"
            ),
        }
    }
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
    /// The Down note of this episode has been made.
    down_noted: bool,
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
    /// The key went Down: one note per episode.
    pub(crate) fn down(&mut self) {
        if !self.down_noted {
            self.down_noted = true;
            self.out.push(Note::Down);
        }
    }
    /// The key is Healthy again: the next Down is a new episode.
    pub(crate) fn healthy(&mut self) {
        self.down_noted = false;
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
