//! One operation's handle on a governor (adaptive concurrency design §4.5,
//! §4.9): what its endpoint asks and what it reports.
use super::{
    control::{Outcome, Ruling},
    filter::Signal,
    governor::{Admission, Conn, Scoped, View},
    notes::Note,
    states::ConnEvent,
    windows::{AttemptId, AttemptKind, Own},
};
use gwz_transport::pool::{Key, Site};

impl Scoped {
    /// Whether new connections may start on `key`'s site, and how many the
    /// operation may have there.
    pub(crate) fn admission(&self, key: &Key, now: u64) -> Admission {
        let mut book = self.governor.book();
        let index = book.slot(&self.operation, &key.site(), now);
        self.governor.sync(&mut book, &self.operation, index, now);
        let limit = &mut book.scope(&self.operation).slots[index].limit;
        Admission {
            gate_open: limit.gate_open(now),
            target: limit.pool_limit(),
            room: limit.has_room(),
            connected: limit.table().connected(),
        }
    }

    /// Whether an open may begin its first exchange (a discovery) on a leased
    /// connection: not during a hold (§4.5).
    pub(crate) fn exchange_may_begin(&self, key: &Key, now: u64) -> bool {
        let mut book = self.governor.book();
        let index = book.slot(&self.operation, &key.site(), now);
        self.governor.sync(&mut book, &self.operation, index, now);
        book.scope(&self.operation).slots[index]
            .limit
            .admits_first_exchange(now)
    }

    /// An exchange begins on the leased connection `connection`: its window
    /// runs from here. False, and nothing begun, during a hold.
    pub(crate) fn exchange_begins(&self, key: &Key, connection: Conn, now: u64) -> bool {
        let mut book = self.governor.book();
        let index = book.slot(&self.operation, &key.site(), now);
        self.governor.sync(&mut book, &self.operation, index, now);
        let slot = &mut book.scope(&self.operation).slots[index];
        if !slot.limit.admits_first_exchange(now) {
            return false;
        }
        // The connection's own setup attempt, if no exchange answered it, is
        // over: it went back to the pool unanswered.
        slot.limit.result(connection.attempt(), Outcome::Ended, now);
        let target = slot.limit.pool_limit();
        let began = slot.limit.begin(
            connection.attempt(),
            AttemptKind::Ordinary,
            target,
            Own::Leased(connection.id()),
            true,
            now,
        );
        if began {
            slot.leased.insert(connection.0);
        }
        began
    }

    /// The attempt on `connection` succeeded: a new connection's first
    /// exchange was answered (for HTTPS, that is when it is Connected), or a
    /// leased exchange was.
    pub(crate) fn answered(&self, key: &Key, connection: Conn, now: u64) {
        let mut book = self.governor.book();
        let site = key.site();
        let index = book.slot(&self.operation, &site, now);
        // Any answered exchange sets up a connection no discovery answered
        // (a POST-only use, or a lease taken after an unanswered one), and the
        // server holds it for every operation's table.
        let operations: Vec<String> = book.scopes.keys().cloned().collect();
        for operation in &operations {
            let other = book.slot(operation, &site, now);
            book.scope(operation).slots[other].limit.conn(
                connection.id(),
                ConnEvent::Connected,
                now,
            );
        }
        let slot = &mut book.scope(&self.operation).slots[index];
        let fresh = !slot.leased.remove(&connection.0);
        slot.limit
            .result(connection.attempt(), Outcome::Succeeded { fresh }, now);
        for operation in &operations {
            let other = book.slot(operation, &site, now);
            self.governor.sync(&mut book, operation, other, now);
        }
    }

    /// The attempt on `connection` was refused with `signal`, with a
    /// `Retry-After` of `retry_after_ms` if the server gave one, on a POST
    /// when `post`. `None` when no attempt is in flight on it. The server's
    /// `Retry-After` holds the site for every live operation.
    pub(crate) fn refused(
        &self,
        key: &Key,
        connection: Conn,
        signal: Signal,
        retry_after_ms: Option<u64>,
        post: bool,
        now: u64,
    ) -> Option<Ruling> {
        let mut book = self.governor.book();
        let site = key.site();
        let index = book.slot(&self.operation, &site, now);
        let slot = &mut book.scope(&self.operation).slots[index];
        slot.leased.remove(&connection.0);
        let ruling = slot.limit.result(
            connection.attempt(),
            Outcome::Refused {
                signal,
                retry_after_ms,
                post,
            },
            now,
        );
        if let Some(wait) = retry_after_ms {
            let operations: Vec<String> = book.scopes.keys().cloned().collect();
            for operation in operations {
                // The asking operation's own hold is set by its judgement, or
                // here when no attempt was in flight to judge.
                if operation == self.operation && ruling.is_some() {
                    continue;
                }
                let other = book.slot(&operation, &site, now);
                book.scope(&operation).slots[other]
                    .limit
                    .set_hold(wait, post, now);
            }
        }
        let operations: Vec<String> = book.scopes.keys().cloned().collect();
        for operation in operations {
            let other = book.slot(&operation, &site, now);
            self.governor.sync(&mut book, &operation, other, now);
        }
        ruling
    }

    /// The attempt that ended in a setup failure of the class `signal`: the
    /// oldest setup the host saw end on the site, which the endpoint reports
    /// in the order the host saw them. `None` when none was waiting (it
    /// expired, or the machine did not begin it), which the endpoint counts
    /// as the retry machine's, as it did before the machine existed.
    pub(crate) fn setup_failed(&self, key: &Key, signal: Signal, now: u64) -> Option<Ruling> {
        let mut book = self.governor.book();
        let index = book.slot(&self.operation, &key.site(), now);
        self.governor.sync(&mut book, &self.operation, index, now);
        let slot = &mut book.scope(&self.operation).slots[index];
        let (conn, _) = slot.ended.pop_front()?;
        let ruling = slot.limit.result(
            AttemptId(conn),
            Outcome::Refused {
                signal,
                retry_after_ms: None,
                post: false,
            },
            now,
        );
        self.governor.sync(&mut book, &self.operation, index, now);
        ruling
    }

    /// The members of this operation queued for a new connection on `key`'s
    /// site, and how many of them are not on their final attempt (a probe's
    /// carriers, §4.7): reported by the endpoint as it holds them.
    pub(crate) fn set_demand(&self, key: &Key, needing_new: usize, non_final: usize, now: u64) {
        let mut book = self.governor.book();
        let index = book.slot(&self.operation, &key.site(), now);
        book.scope(&self.operation).slots[index]
            .limit
            .set_demand(needing_new, non_final);
    }

    /// Starts the test that is due on `key`'s site for a carrier, if one is
    /// ready: returns the admission target, which the pool's limit has
    /// already been raised to under this lock, so the carrier's request is
    /// served by a new connection.
    pub(crate) fn start_test(&self, key: &Key, carrier_final: bool, now: u64) -> Option<usize> {
        let mut book = self.governor.book();
        let index = book.slot(&self.operation, &key.site(), now);
        let target = book.scope(&self.operation).slots[index]
            .limit
            .arm_test(carrier_final, now)?;
        self.governor.sync(&mut book, &self.operation, index, now);
        Some(target)
    }

    /// The carrier of a test ended without a connection of its own.
    pub(crate) fn test_unused(&self, key: &Key, now: u64) {
        let mut book = self.governor.book();
        let index = book.slot(&self.operation, &key.site(), now);
        book.scope(&self.operation).slots[index].limit.disarm(now);
        self.governor.sync(&mut book, &self.operation, index, now);
    }

    /// `key`'s site as this operation's machine has it, if it has touched it.
    pub(crate) fn view(&self, key: &Key, now: u64) -> Option<View> {
        let mut book = self.governor.book();
        let site = key.site();
        let scope = book.scopes.get(&self.operation)?;
        let index = scope.slots.iter().position(|slot| slot.site == site)?;
        self.governor.sync(&mut book, &self.operation, index, now);
        let scope = book.scope(&self.operation);
        let ceiling = scope.ceiling;
        let slot = &mut scope.slots[index];
        let table = slot.limit.table();
        Some(View {
            state: slot.limit.state(),
            n: slot.limit.n(),
            ceiling,
            connected: table.connected(),
            possible: table.possible(),
            pool_limit: slot.limit.pool_limit(),
            settle_ms: slot.limit.settle_ms(),
            confirmation: slot.limit.confirmation_open(),
        })
    }

    /// The notes this operation's machines have made since the last call
    /// (§9).
    pub(crate) fn drain_notes(&self) -> Vec<(Site, Note)> {
        let mut book = self.governor.book();
        std::mem::take(&mut book.scope(&self.operation).notes)
    }
}
