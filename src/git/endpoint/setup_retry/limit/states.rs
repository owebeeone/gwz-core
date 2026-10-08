//! The connection-state table (§4.1) and the settle time `Ts`.
use std::collections::{BTreeMap, BTreeSet};

/// Names a connection on the key. The endpoint or pool mints it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ConnId(pub(crate) u64);

/// §4.1's states. `Gone` is not stored: an entry that reaches it is removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    SettingUp,
    Connected,
    Closing,
    Settling,
    Gone,
}

/// What happened to a connection, as its owner saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnEvent {
    /// Socket connect begun. `clocked` is false for a setup with no clock
    /// (`--ssh-timeout 0`), which never keeps the key from being quiet.
    Started { clocked: bool },
    /// Setup complete (§4.1: authenticated on SSH, first exchange answered on
    /// HTTPS).
    Connected,
    /// The server refused or dropped it during setup: Gone, no hold.
    SetupEnded,
    /// An abandoned setup's job retired, or the client cancelled the connect:
    /// Settling.
    Retired,
    /// The client began to close it, or found it dead on use.
    Closing,
    /// The client disposed of a Closing connection: Settling.
    Disposed,
    /// The server ended a Connected or Closing connection: Gone.
    ServerClosed,
}

/// One transition: `from` is `None` for a connection new to the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) conn: ConnId,
    pub(crate) from: Option<Phase>,
    pub(crate) to: Phase,
}

const MIN_SETTLE_MS: u64 = 250;

/// `Ts`, the settle time: `max(250 ms, 2 x SRTT + 100 ms)`, with `SRTT` the
/// smoothed connect time measured on this key in this command.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Settle {
    srtt: Option<u64>,
}

impl Settle {
    pub(crate) fn observe_connect(&mut self, ms: u64) {
        self.srtt = Some(self.srtt.map_or(ms, |srtt| (srtt * 7 + ms) / 8));
    }
    pub(crate) fn ts(&self) -> u64 {
        self.srtt.map_or(MIN_SETTLE_MS, |srtt| {
            srtt.saturating_mul(2)
                .saturating_add(100)
                .max(MIN_SETTLE_MS)
        })
    }
}

struct Entry {
    phase: Phase,
    until: u64,
    clocked: bool,
}

/// Every connection on one key, with its state. Queries are as of the last
/// [`Table::apply`] or [`Table::advance`].
#[derive(Default)]
pub(crate) struct Table {
    conns: BTreeMap<ConnId, Entry>,
    settle: Settle,
}

impl Table {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn settle_mut(&mut self) -> &mut Settle {
        &mut self.settle
    }
    /// Moves every Settling connection whose time has passed to Gone.
    pub(crate) fn advance(&mut self, now: u64) {
        self.conns
            .retain(|_, entry| entry.phase != Phase::Settling || entry.until > now);
    }
    /// Applies `event` to `conn` at `now`. An unknown connection or a
    /// transition the table does not have changes nothing and returns `None`.
    pub(crate) fn apply(&mut self, conn: ConnId, event: ConnEvent, now: u64) -> Option<Change> {
        self.advance(now);
        let from = self.conns.get(&conn).map(|entry| entry.phase);
        let to = match (from, event) {
            (None, ConnEvent::Started { clocked }) => {
                let entry = Entry {
                    phase: Phase::SettingUp,
                    until: 0,
                    clocked,
                };
                self.conns.insert(conn, entry);
                return Some(Change {
                    conn,
                    from,
                    to: Phase::SettingUp,
                });
            }
            (Some(Phase::SettingUp), ConnEvent::Connected) => Phase::Connected,
            (Some(Phase::SettingUp), ConnEvent::SetupEnded) => Phase::Gone,
            (Some(Phase::SettingUp), ConnEvent::Retired)
            | (Some(Phase::Closing), ConnEvent::Disposed) => Phase::Settling,
            (Some(Phase::Connected), ConnEvent::Closing) => Phase::Closing,
            (Some(Phase::Connected | Phase::Closing), ConnEvent::ServerClosed) => Phase::Gone,
            _ => return None,
        };
        if to == Phase::Gone {
            self.conns.remove(&conn);
        } else if let Some(entry) = self.conns.get_mut(&conn) {
            entry.phase = to;
            entry.until = now.saturating_add(self.settle.ts());
        }
        Some(Change { conn, from, to })
    }
    pub(crate) fn phase(&self, conn: ConnId) -> Option<Phase> {
        self.conns.get(&conn).map(|entry| entry.phase)
    }
    fn count(&self, wanted: impl Fn(&Entry) -> bool) -> usize {
        self.conns.values().filter(|entry| wanted(entry)).count()
    }
    fn ids(&self, wanted: impl Fn(&Entry) -> bool) -> BTreeSet<ConnId> {
        let wanted = &wanted;
        self.conns
            .iter()
            .filter_map(|(conn, entry)| wanted(entry).then_some(*conn))
            .collect()
    }
    /// The number Connected.
    pub(crate) fn connected(&self) -> usize {
        self.count(|entry| entry.phase == Phase::Connected)
    }
    /// Setting up, Connected, Closing or Settling.
    pub(crate) fn possible(&self) -> usize {
        self.conns.len()
    }
    /// Setting up, Connected or Closing: SATURATED's admission count (§4.5).
    pub(crate) fn held(&self) -> usize {
        self.count(|entry| entry.phase != Phase::Settling)
    }
    /// §4.5's quiet: no clocked setup in flight, nothing Closing, nothing
    /// Settling.
    pub(crate) fn is_quiet(&self) -> bool {
        self.count(|entry| match entry.phase {
            Phase::SettingUp => entry.clocked,
            phase => matches!(phase, Phase::Closing | Phase::Settling),
        }) == 0
    }
    pub(crate) fn next_settle_deadline(&self) -> Option<u64> {
        self.conns
            .values()
            .filter(|entry| entry.phase == Phase::Settling)
            .map(|entry| entry.until)
            .min()
    }
    pub(crate) fn connected_ids(&self) -> BTreeSet<ConnId> {
        self.ids(|entry| entry.phase == Phase::Connected)
    }
    pub(crate) fn possible_ids(&self) -> BTreeSet<ConnId> {
        self.conns.keys().copied().collect()
    }
    /// Closing or Settling.
    pub(crate) fn winding_down_ids(&self) -> BTreeSet<ConnId> {
        self.ids(|entry| matches!(entry.phase, Phase::Closing | Phase::Settling))
    }
    /// Whether any of `ids` is still Closing or Settling.
    pub(crate) fn any_winding_down(&self, ids: &BTreeSet<ConnId>) -> bool {
        ids.iter()
            .any(|conn| matches!(self.phase(*conn), Some(Phase::Closing | Phase::Settling)))
    }
}
