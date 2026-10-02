//! The HTTPS endpoint's setup retries: the retry plan's §4 and §5, with
//! amendment 2's §3.20 (OD18) cold start, for the connect before an open's
//! first request byte (gwz-core dev-docs/GwzRemoteTransportRetryPlan.md).
//!
//! An open asks its key's machine before each attempt starts, and the
//! machine learns how the attempt's first connect ended. A retried open waits
//! in the endpoint, not in the pool, so a wake with no open left starts
//! nothing, and an open the per-host limit holds back starts no connect while
//! its key waits.
use super::*;
use crate::git::endpoint::setup_retry::{
    self, AllocationClock, Decision, Jitter, Operations, Outcome, Phase,
};

/// The endpoint's setup retry state: each operation's machines, by HTTPS
/// pool key, whose members are the opens' stream keys, and the jitter that
/// each wait draws.
pub(super) struct Retries {
    machines: Operations<pool::Key, Key>,
    jitter: Jitter,
}

/// An open the endpoint holds until an attempt may start.
pub(super) struct Held {
    /// What its first attempt continues: the anonymous attempt's budget and
    /// connection that a `gh` Open carries on. A later attempt starts afresh.
    carry: Option<Retry>,
    /// Its allocation clock, as the SSH endpoint's queue keeps it: it runs
    /// while the per-host limit holds the open and stops while its key does.
    allocation: AllocationClock,
}
impl Held {
    pub(super) fn new(carry: Option<Retry>, now: u64, allocation_ms: i64) -> Self {
        Self {
            carry,
            allocation: AllocationClock::new(now, allocation_ms.max(0) as u64),
        }
    }
    /// Drops a carried challenge connection once it lapses, as `step` does
    /// for one that an operation keeps, so a waiting open holds none.
    fn lapse(&mut self) {
        if let Some(carry) = self.carry.as_mut()
            && carry
                .challenge
                .as_ref()
                .is_some_and(ChallengeLease::expired)
        {
            carry.challenge = None;
        }
    }
}

impl Retries {
    pub(super) fn new() -> Self {
        Self {
            machines: Operations::new(),
            jitter: Jitter::random(),
        }
    }
    pub(super) fn set_max_retries(&mut self, request: &str, max_retries: u32) {
        self.machines.set_max_retries(request, max_retries);
    }
    /// Forgets a finished or cancelled request: its next operation starts
    /// its keys Cold.
    pub(super) fn remove(&mut self, request: &str) {
        self.machines.remove(request);
    }
    /// The open `member` ended its attempt with no verdict: it was
    /// cancelled, which frees its key's probe.
    pub(super) fn abandoned(&mut self, member: &Key, pool_key: &pool::Key) {
        self.machines.machine(&member.0, pool_key).abandoned(member);
    }

    /// Tells `entry`'s key how its open's attempt ended, and returns what the
    /// open publishes, or `None` when it waits for another attempt.
    pub(super) fn settle(
        &mut self,
        now: u64,
        member: Key,
        entry: &mut Entry,
        result: Result<Prepared, Failure>,
        connect: FirstConnect,
        carry: &mut Retry,
    ) -> Option<Result<Prepared, Failure>> {
        let machine = self.machines.machine(&member.0, &entry.pool_key);
        // A fresh connect is the setup the key counts, also when the request
        // after it failed: the setup ended before that request's first byte.
        let admitted = match (connect, &result) {
            (FirstConnect::Connected, _) => machine.succeeded(&member, true),
            (FirstConnect::None, Ok(_)) => machine.succeeded(&member, false),
            _ => true,
        };
        let failure = match result {
            Ok(mut prepared) => {
                if !admitted {
                    prepared.discard_after_use();
                }
                let facts = std::mem::take(&mut prepared.opened.facts);
                entry.facts = setup_retry::merged_facts(entry.facts.take(), Some(facts));
                prepared.opened.facts = entry.facts.clone().unwrap_or_default();
                return Some(Ok(prepared));
            }
            Err(failure) => failure,
        };
        if !admitted {
            // Nor does a `gh` Open carry on a connection the key did not admit.
            carry.challenge = None;
        }
        let phase = if connect == FirstConnect::Failed {
            Phase::Setup
        } else {
            Phase::Other
        };
        let verdict = setup_retry::classify(&failure, phase);
        let jitter = self.jitter.draw();
        let failure = match machine.failed(&member, verdict, failure.clone(), now, jitter) {
            Outcome::Retry => {
                entry.facts = setup_retry::merged_facts(entry.facts.take(), failure.facts);
                let allocation = entry
                    .envelope
                    .open
                    .as_ref()
                    .map_or(0, |open| open.deadlines.allocation_ms);
                entry.held = Some(Held::new(None, now, allocation));
                return None;
            }
            // Its own attempts' facts, also when the key finishes it with a
            // failure another open's setup recorded.
            Outcome::Finish(last) => Failure {
                facts: failure.facts,
                ..last.wire_failure()
            },
            Outcome::Return => failure,
        };
        entry.facts = setup_retry::merged_facts(entry.facts.take(), failure.facts.clone());
        Some(Err(Failure {
            facts: entry.facts.clone(),
            ..failure
        }))
    }
}

impl HttpsEndpoint {
    /// Offers each held open an attempt.
    pub(super) fn start_held(&mut self) {
        let held: Vec<Key> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.held.is_some())
            .map(|(key, _)| key.clone())
            .collect();
        for key in held {
            self.admit(&key);
        }
    }

    /// Starts an attempt of `key`'s open when its key's machine and the
    /// per-host limit allow one, finishes the open when its key is closed
    /// for the operation or its allocation ran out, and otherwise keeps
    /// holding it.
    pub(super) fn admit(&mut self, key: &Key) {
        let now = self.now_ms;
        let Some(pool_key) = self
            .entries
            .get(key)
            .filter(|entry| entry.held.is_some())
            .map(|entry| entry.pool_key.clone())
        else {
            return;
        };
        let decision = self.retries.machines.machine(&key.0, &pool_key).decide(now);
        let room = self.admits_attempt(&pool_key);
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        match decision {
            // The key's recorded failure, without the facts of the setup
            // that recorded it: this open set nothing up in this pass.
            Decision::Finish(last) => {
                entry.held = None;
                fail(
                    entry,
                    Failure {
                        facts: None,
                        ..last.wire_failure()
                    },
                );
            }
            Decision::Wait => {
                if let Some(held) = entry.held.as_mut() {
                    held.allocation.stop(now);
                    held.lapse();
                }
            }
            Decision::Start => {
                let Some(mut held) = entry.held.take() else {
                    return;
                };
                held.allocation.run(now);
                let left = held.allocation.left(now);
                if left == 0 {
                    fail(entry, setup_retry::allocation_timeout());
                } else if room {
                    self.start_attempt(key, held.carry, left);
                } else {
                    held.lapse();
                    entry.held = Some(held);
                }
            }
        }
    }

    /// An attempt starts while the attempts in flight on its host stay within
    /// the operation's per-host limits, which the transport host installs in
    /// the pool: a key's first wave is at most that many setups (amendment
    /// 2's §3.20), as on SSH.
    fn admits_attempt(&self, pool_key: &pool::Key) -> bool {
        let capacity = self.client.pool().capacity();
        let in_flight = self
            .entries
            .values()
            .filter(|entry| entry.preparing.is_some() && entry.pool_key.host == pool_key.host)
            .count();
        in_flight < capacity.per_host.min(capacity.per_user_host)
    }

    /// One attempt of `key`'s open, with its own budget, afresh unless it
    /// continues the anonymous attempt `carry`, and the allocation its hold
    /// `left`.
    fn start_attempt(&mut self, key: &Key, carry: Option<Retry>, left: u64) {
        let Some(name) = self
            .operations
            .get(&key.0)
            .map(|operation| operation.name.clone())
        else {
            return;
        };
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        let open = entry.envelope.open.as_ref().expect("admitted Open");
        let input = input(open, &entry.envelope.session_id, &name);
        let mut deadlines = open.deadlines.clone();
        deadlines.allocation_ms = left.min(i64::MAX as u64) as i64;
        let mut carry = carry.unwrap_or_else(|| Retry {
            budget: self.client.budget_for_open(&open.deadlines),
            challenge: None,
        });
        carry
            .budget
            .shorten(self.client.budget_for_open(&deadlines));
        self.retries
            .machines
            .machine(&key.0, &entry.pool_key)
            .start(key.clone());
        let cancelled = entry.cancel.clone();
        let client = self.client.clone();
        entry.preparing = Some(self.runtime.spawn(async move {
            let (result, connect) = client
                .prepare_attempt(input, &cancelled, &mut carry.budget, &mut carry.challenge)
                .await;
            (result, connect, carry)
        }));
    }
}

/// Ends `entry`'s open with `failure`, which then carries every fact of the
/// open's attempts.
fn fail(entry: &mut Entry, failure: Failure) {
    entry.facts = setup_retry::merged_facts(entry.facts.take(), failure.facts.clone());
    entry.output = Some(Envelope {
        version: entry.envelope.version,
        session_id: entry.envelope.session_id.clone(),
        stream_id: entry.envelope.stream_id,
        kind: MessageKind::OpenFailed,
        open_failed: Some(Failure {
            facts: entry.facts.clone(),
            ..failure
        }),
        ..Default::default()
    });
}

/// The worker's request for `open`, within the endpoint's operation `name`.
fn input(open: &Open, session: &str, name: &str) -> Input {
    let host = if open.destination.host.contains(':') {
        format!("[{}]", open.destination.host)
    } else {
        open.destination.host.clone()
    };
    let selector = open.destination.https_username.as_deref()
        .map_or_else(String::new, |value| format!("{value}@"));
    Input {
        destination: format!(
            "https://{selector}{host}:{}{}",
            open.destination.port, open.destination.path
        ),
        service: open.service,
        policy: open.policy,
        session: session.into(),
        operation: name.into(),
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl Retries {
            pub(super) fn set_jitter(&mut self, jitter: Jitter) {
                self.jitter = jitter;
            }
        }
    }
}
