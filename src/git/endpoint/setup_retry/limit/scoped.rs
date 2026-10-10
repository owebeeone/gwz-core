//! One operation's handle on a governor (adaptive concurrency design §4.5,
//! §4.9): what its endpoint asks and what it reports. An operation that is
//! not live (never begun, or ended) is answered as if no limit applied, and
//! its reports change nothing: only `begin_operation` makes a machine.
use super::{
    control::{Outcome, Ruling},
    filter::Signal,
    governor::{Admission, Book, Conn, Scoped, Stage, TestToken, View, label},
    notes::Note,
    states::ConnEvent,
    timer::T0_MS,
    windows::{AttemptId, AttemptKind, Own},
};
use gwz_transport::pool::{Key, Site};

impl Scoped {
    /// The tag a request of this operation's member `member` carries.
    pub(crate) fn tag(&self, member: &str) -> String {
        label(&self.operation, member)
    }

    /// Runs `f` on this operation's slot for `key`'s site, after bringing it
    /// up to `now`, and syncs every machine on the site afterwards. `None`
    /// when the operation is not live.
    fn with<R>(
        &self,
        key: &Key,
        now: u64,
        f: impl FnOnce(&mut super::governor::Slot, &mut Book) -> R,
    ) -> Option<R> {
        let mut book = self.governor.book();
        let site = key.site();
        let index = book.slot(&self.operation, &site, now)?;
        self.governor.sync(&mut book, &self.operation, index, now);
        let mut slot = book
            .scopes
            .get_mut(&self.operation)?
            .slots
            .swap_remove(index);
        let result = f(&mut slot, &mut book);
        let scope = book.scopes.get_mut(&self.operation)?;
        scope.slots.push(slot);
        let last = scope.slots.len() - 1;
        scope.slots.swap(index, last);
        self.governor.sync_site(&mut book, &site, now);
        Some(result)
    }

    /// Whether new connections may start on `key`'s site, and how many the
    /// operation may have there.
    pub(crate) fn admission(&self, key: &Key, now: u64) -> Admission {
        self.with(key, now, |slot, _| Admission {
            gate_open: slot.limit.gate_open(now),
            target: slot.limit.pool_limit(),
            room: slot.limit.has_room(),
            connected: slot.limit.table().connected(),
        })
        .unwrap_or_else(|| Admission {
            gate_open: true,
            target: self.governor.book().default_ceiling(),
            room: true,
            connected: 0,
        })
    }

    /// Whether an open may begin its first exchange (a discovery) on a leased
    /// connection: not during a hold (§4.5).
    pub(crate) fn exchange_may_begin(&self, key: &Key, now: u64) -> bool {
        self.with(key, now, |slot, _| slot.limit.admits_first_exchange(now))
            .unwrap_or(true)
    }

    /// An exchange begins on the leased connection `connection`: its window
    /// runs from here. False, and nothing begun, during a hold.
    pub(crate) fn exchange_begins(&self, key: &Key, connection: Conn, now: u64) -> bool {
        self.with(key, now, |slot, _| {
            if !slot.limit.admits_first_exchange(now) {
                return false;
            }
            // The connection's own setup attempt, if no exchange answered it,
            // is over: it went back to the pool unanswered.
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
        })
        .unwrap_or(true)
    }

    /// The attempt on `connection` succeeded: a new connection's first
    /// exchange was answered (for HTTPS, that is when it is Connected), or a
    /// leased exchange was. The server holds it for every operation's table,
    /// and a fresh one is every operation's success (§4.2), as SSH's
    /// authentication is.
    pub(crate) fn answered(&self, key: &Key, connection: Conn, now: u64) {
        let mut book = self.governor.book();
        let site = key.site();
        let state = book.site_index(&site);
        let table = &mut book.sites[state];
        table
            .table
            .apply(connection.id(), ConnEvent::Connected, now);
        let mut connect_ms = None;
        if let Some(info) = table.conns.get_mut(&connection.0) {
            if info.stage == Stage::Live {
                connect_ms = Some(now.saturating_sub(info.started));
            }
            info.stage = Stage::Up;
        }
        if let Some(ms) = connect_ms {
            table.table.settle_mut().observe_connect(ms);
        }
        for (operation, index) in book.slots_on(&site) {
            let mine = operation == self.operation;
            let slot = &mut book.scopes.get_mut(&operation).expect("scope").slots[index];
            if let Some(ms) = connect_ms {
                slot.limit.connect_time(ms);
            }
            slot.limit.conn(connection.id(), ConnEvent::Connected, now);
            if mine {
                let fresh = !slot.leased.remove(&connection.0);
                slot.limit
                    .result(connection.attempt(), Outcome::Succeeded { fresh }, now);
            } else if connect_ms.is_some() {
                slot.limit.observed_success(now);
            }
        }
        self.governor.sync_site(&mut book, &site, now);
    }

    /// The attempt on `connection` was refused with `signal`, with a
    /// `Retry-After` of `retry_after_ms` if the server gave one, on a POST
    /// when `post`. `None` when no attempt is in flight on it. The server's
    /// `Retry-After` holds the site for every live operation. A throttle that
    /// teaches the machine nothing (nothing else was counted) and carries no
    /// `Retry-After` still holds the site for `T0`: a host already
    /// throttling is not asked again at once (§4.8).
    pub(crate) fn refused(
        &self,
        key: &Key,
        connection: Conn,
        signal: Signal,
        retry_after_ms: Option<u64>,
        post: bool,
        now: u64,
    ) -> Option<Ruling> {
        let site = key.site();
        let ruling = self
            .with(key, now, |slot, _| {
                slot.leased.remove(&connection.0);
                slot.limit.result(
                    connection.attempt(),
                    Outcome::Refused {
                        signal,
                        retry_after_ms,
                        post,
                    },
                    now,
                )
            })
            .flatten();
        let mut book = self.governor.book();
        let wait = match (retry_after_ms, signal, ruling) {
            (Some(wait), _, _) => Some(wait),
            (None, Signal::Throttle, None | Some(Ruling::RetryMachine)) => Some(T0_MS),
            _ => None,
        };
        if let Some(wait) = wait {
            for (operation, index) in book.slots_on(&site) {
                // The asking operation's own hold with a `Retry-After` is set
                // by its judgement.
                if operation == self.operation && retry_after_ms.is_some() && ruling.is_some() {
                    continue;
                }
                if operation != self.operation && retry_after_ms.is_none() {
                    continue;
                }
                book.scopes.get_mut(&operation).expect("scope").slots[index]
                    .limit
                    .set_hold(wait, post, now);
            }
            self.governor.sync_site(&mut book, &site, now);
        }
        ruling
    }

    /// The setup of this operation's member `member` failed with a failure of
    /// the class `signal` (§4.8): judged on that connection's own window. The
    /// host may have seen it end already, or not yet (a connect that timed
    /// out fails its request first). `None` when the member has no setup the
    /// machine began, or it was judged or expired already, which the endpoint
    /// counts as the retry machine's, as it did before the machine existed.
    pub(crate) fn setup_failed(
        &self,
        key: &Key,
        member: &str,
        signal: Signal,
        now: u64,
    ) -> Option<Ruling> {
        let tag = self.tag(member);
        let site = key.site();
        self.with(key, now, |slot, book| {
            let state = book.site_index(&site);
            let (conn, info) = book.sites[state].conns.iter_mut().rev().find(|(_, info)| {
                info.tag.as_deref() == Some(tag.as_str())
                    && matches!(info.stage, Stage::Live | Stage::Ended(_))
            })?;
            let conn = *conn;
            let live = info.stage == Stage::Live;
            if live {
                info.stage = Stage::Judged;
                slot.limit.freeze(AttemptId(conn));
            } else {
                book.sites[state].conns.remove(&conn);
            }
            slot.limit.result(
                AttemptId(conn),
                Outcome::Refused {
                    signal,
                    retry_after_ms: None,
                    post: false,
                },
                now,
            )
        })
        .flatten()
    }

    /// The members of this operation queued for a new connection on `key`'s
    /// site, and how many of them are not on their final attempt (a probe's
    /// carriers, §4.7): reported by the endpoint as it holds them.
    pub(crate) fn set_demand(&self, key: &Key, needing_new: usize, non_final: usize, now: u64) {
        let mut book = self.governor.book();
        if let Some(index) = book.slot(&self.operation, &key.site(), now) {
            book.scopes.get_mut(&self.operation).expect("scope").slots[index]
                .limit
                .set_demand(needing_new, non_final);
        }
    }

    /// Starts the test that is due on `key`'s site for a carrier, if one is
    /// ready: returns the admission target, which the pool's limit has
    /// already been raised to under this lock, so the carrier's request is
    /// served by a new connection, and the token the carrier holds until it
    /// has a connection of its own or ends.
    pub(crate) fn start_test(
        &self,
        key: &Key,
        carrier_final: bool,
        now: u64,
    ) -> Option<(usize, TestToken)> {
        let (target, arm) = self
            .with(key, now, |slot, _| slot.limit.arm_test(carrier_final, now))
            .flatten()?;
        Some((
            target,
            TestToken::new(&self.governor, &self.operation, key.site(), arm),
        ))
    }

    /// `key`'s site as this operation's machine has it, if it has touched it.
    pub(crate) fn view(&self, key: &Key, now: u64) -> Option<View> {
        let site = key.site();
        {
            let book = self.governor.book();
            let scope = book.scopes.get(&self.operation)?;
            scope.slots.iter().position(|slot| slot.site == site)?;
        }
        self.with(key, now, |slot, book| {
            let ceiling = book.scopes.get(&self.operation).map_or(0, |s| s.ceiling);
            let table = slot.limit.table();
            View {
                state: slot.limit.state(),
                n: slot.limit.n(),
                ceiling,
                connected: table.connected(),
                possible: table.possible(),
                pool_limit: slot.limit.pool_limit(),
                settle_ms: slot.limit.settle_ms(),
                confirmation: slot.limit.confirmation_open(),
            }
        })
    }

    /// The notes this operation's machines have made since the last call
    /// (§9).
    pub(crate) fn drain_notes(&self) -> Vec<(Site, Note)> {
        let mut book = self.governor.book();
        book.scopes
            .get_mut(&self.operation)
            .map(|scope| std::mem::take(&mut scope.notes))
            .unwrap_or_default()
    }
}
