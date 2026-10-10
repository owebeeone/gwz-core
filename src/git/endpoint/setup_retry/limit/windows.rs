//! The window of an attempt (§4.3): the two sets that say what the server
//! could have held when it decided the attempt.
use super::states::{Change, ConnId, Phase, Table};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AttemptId(pub(crate) u64);

/// What an attempt is (§4.4's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttemptKind {
    Ordinary,
    Probe,
    Confirming,
    Restore,
}

impl AttemptKind {
    /// A test uses the key's one test slot and must be a new connection.
    pub(crate) fn is_test(self) -> bool {
        matches!(self, Self::Probe | Self::Confirming)
    }
}

/// The connection an attempt is its own: excluded from both sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Own {
    /// A new connection, whose socket connect begins with the window.
    New(ConnId),
    /// An exchange on a leased connection: the window runs from the lease.
    Leased(ConnId),
}

struct Window {
    kind: AttemptKind,
    own: ConnId,
    lo: BTreeSet<ConnId>,
    hi: BTreeSet<ConnId>,
    winding_down: BTreeSet<ConnId>,
    /// The attempt's connection ended: the sets stop where they are, though
    /// the verdict on it is still to come.
    frozen: bool,
}

/// A window at its attempt's result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Closed {
    pub(crate) kind: AttemptKind,
    pub(crate) lo: usize,
    pub(crate) hi: usize,
    /// Every connection that was Closing or Settling at any time in the
    /// window: the next start waits until they have settled (§4.4).
    pub(crate) winding_down: BTreeSet<ConnId>,
}

/// The windows of the attempts in flight on one key.
#[derive(Default)]
pub(crate) struct Windows {
    open: BTreeMap<AttemptId, Window>,
}

impl Own {
    fn conn(self) -> ConnId {
        match self {
            Self::New(conn) | Self::Leased(conn) => conn,
        }
    }
}

impl Windows {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    /// Opens `attempt`'s window on `table`'s current state. Returns false,
    /// changing nothing, if the attempt already has one.
    pub(crate) fn open(
        &mut self,
        attempt: AttemptId,
        kind: AttemptKind,
        own: Own,
        table: &Table,
    ) -> bool {
        if self.open.contains_key(&attempt) {
            return false;
        }
        let own = own.conn();
        let others = |mut set: BTreeSet<ConnId>| {
            set.remove(&own);
            set
        };
        self.open.insert(
            attempt,
            Window {
                kind,
                own,
                lo: others(table.connected_ids()),
                hi: others(table.possible_ids()),
                winding_down: others(table.winding_down_ids()),
                frozen: false,
            },
        );
        true
    }
    /// Reports a table transition to every open window.
    pub(crate) fn observe(&mut self, change: &Change) {
        for window in self
            .open
            .values_mut()
            .filter(|w| w.own != change.conn && !w.frozen)
        {
            if change.from == Some(Phase::Connected) {
                window.lo.remove(&change.conn);
            }
            match change.to {
                Phase::Gone => {}
                Phase::Closing | Phase::Settling => {
                    window.hi.insert(change.conn);
                    window.winding_down.insert(change.conn);
                }
                Phase::SettingUp | Phase::Connected => {
                    window.hi.insert(change.conn);
                }
            }
        }
    }
    /// Ends the window's view of the key at `now` without closing it: the
    /// attempt's connection has ended and its judgement is yet to be asked.
    pub(crate) fn freeze(&mut self, attempt: AttemptId) {
        if let Some(window) = self.open.get_mut(&attempt) {
            window.frozen = true;
        }
    }
    /// Closes the window at the attempt's result or abandonment.
    pub(crate) fn close(&mut self, attempt: AttemptId) -> Option<Closed> {
        let window = self.open.remove(&attempt)?;
        Some(Closed {
            kind: window.kind,
            lo: window.lo.len(),
            hi: window.hi.len(),
            winding_down: window.winding_down,
        })
    }
}
