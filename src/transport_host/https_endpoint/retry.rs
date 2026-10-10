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
    self, Admission, AllocationClock, Decision, Jitter, Operations, Outcome, Phase, Scoped,
};

/// How an attempt ended, and what the limit machine can say of it.
pub(super) struct Settling {
    pub(super) result: Result<Prepared, Failure>,
    pub(super) connect: FirstConnect,
    /// A refusal the machine judged on its response (§5.1).
    pub(super) rejection: Option<Rejection>,
    pub(super) governor: Scoped,
    pub(super) pool_now: u64,
}

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
    /// The request's `--max-retries`.
    pub(super) fn max_retries(&self, request: &str) -> u32 {
        self.machines.max_retries(request)
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
        settling: Settling,
        carry: &mut Retry,
    ) -> Option<Result<Prepared, Failure>> {
        let Settling {
            result,
            connect,
            rejection,
            governor,
            pool_now,
        } = settling;
        let allowed = self.machines.max_retries(&member.0).saturating_add(1);
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
        // A refusal that looks like the site's limit is judged by its machine
        // (§4.8): a response's was judged when it came, and a connect
        // failure's is asked for now. When it is evidence, the open is
        // requeued and the key is not made to count it (§5.1).
        let rejection = rejection.or_else(|| {
            let signal = setup_retry::suspect(&failure, phase, false)?;
            let ruling = governor.setup_failed(&entry.pool_key, signal, pool_now);
            Some(Rejection {
                throttled: false,
                ruling,
            })
        });
        if let Some(rejection) = rejection
            && setup_retry::requeues(rejection.ruling, rejection.throttled)
        {
            machine.abandoned(&member);
            if entry.attempts >= allowed {
                let facts = failure.facts.clone();
                let spent =
                    setup_retry::spent(failure, rejection.throttled, entry.attempts, allowed);
                entry.facts = setup_retry::merged_facts(entry.facts.take(), facts);
                return Some(Err(Failure {
                    facts: entry.facts.clone(),
                    ..spent
                }));
            }
            entry.facts = setup_retry::merged_facts(entry.facts.take(), failure.facts);
            let allocation = entry
                .envelope
                .open
                .as_ref()
                .map_or(0, |open| open.deadlines.allocation_ms);
            entry.held = Some(Held::new(None, now, allocation));
            return None;
        }
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
    /// Whether a connection of the site may be idle: more are set up than
    /// carry a stream. An overestimate costs a wait in the pool, as at the
    /// ceiling; an underestimate would hold an open that could lease.
    fn idle_may_exist(&self, pool_key: &pool::Key, admission: &Admission) -> bool {
        let streaming = self
            .entries
            .values()
            .filter(|entry| {
                (entry.prepared.is_some() || entry.serving.is_some())
                    && entry.pool_key.site() == pool_key.site()
            })
            .count();
        admission.connected > streaming
    }

    /// Tells each operation's limit machines how many held opens want a new
    /// connection on a site, and how many could carry a probe (not on their
    /// final attempt, §4.7). A site with none held is told zero.
    fn report_demand(&self) {
        let pool_now = self.client.pool_now();
        let mut demand: Vec<(String, pool::Key, usize, usize)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.preparing.is_some())
            .map(|((request, _), entry)| (request.clone(), entry.pool_key.clone(), 0, 0))
            .collect();
        for ((request, _), entry) in self.entries.iter().filter(|(_, e)| e.held.is_some()) {
            let spare = entry.attempts < self.retries.machines.max_retries(request);
            match demand
                .iter_mut()
                .find(|(op, key, ..)| op == request && *key == entry.pool_key)
            {
                Some(found) => {
                    found.2 += 1;
                    found.3 += usize::from(spare);
                }
                None => demand.push((
                    request.clone(),
                    entry.pool_key.clone(),
                    1,
                    usize::from(spare),
                )),
            }
        }
        for (request, key, needing, spare) in demand {
            if let Some(operation) = self.operations.get(&request) {
                self.client
                    .governor()
                    .scoped(&operation.name)
                    .set_demand(&key, needing, spare, pool_now);
            }
        }
    }

    /// Offers each held open an attempt.
    pub(super) fn start_held(&mut self) {
        self.report_demand();
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
        let name = self
            .operations
            .get(&key.0)
            .map(|operation| operation.name.clone())
            .unwrap_or_default();
        let governor = self.client.governor().scoped(&name);
        let pool_now = self.client.pool_now();
        let mut admission = governor.admission(&pool_key, pool_now);
        // A due test of the site's limit starts through this open when it
        // cannot start otherwise, the gate shut or the site full (§4.5, §4.7):
        // never a probe on its final attempt.
        let mut carries_test = false;
        let idle = self.idle_may_exist(&pool_key, &admission);
        if matches!(decision, Decision::Start)
            && !(admission.gate_open && (admission.room || idle))
            && let Some(attempts) = self.entries.get(key).map(|entry| entry.attempts)
            && let Some(target) = governor.start_test(
                &pool_key,
                attempts >= self.retries.machines.max_retries(&key.0),
                pool_now,
            )
        {
            admission = Admission {
                gate_open: true,
                target,
                ..admission
            };
            carries_test = true;
        }
        // Below the ceiling an open that cannot lease an idle connection
        // waits for room here, where its allocation clock stops (§5.2).
        let waits_for_room = !carries_test && !admission.room && !idle;
        let room = self.admits_attempt(&pool_key, admission.target);
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
                if !admission.gate_open || waits_for_room {
                    // Behind a hold of its site, or a full one (§5.2): the
                    // wait is the server's or the limit's, and the allocation
                    // clock stops, as it does while the key's retry machine
                    // holds the open.
                    held.allocation.stop(now);
                    held.lapse();
                    entry.held = Some(held);
                    return;
                }
                held.allocation.run(now);
                let left = held.allocation.left(now);
                if left == 0 {
                    fail(entry, setup_retry::allocation_timeout());
                } else if room {
                    self.start_attempt(key, held.carry, left, carries_test);
                } else {
                    held.lapse();
                    entry.held = Some(held);
                    if carries_test {
                        governor.test_unused(&pool_key, pool_now);
                    }
                }
            }
        }
    }

    /// An attempt starts while the attempts in flight on its host stay within
    /// the operation's per-host limits, which the transport host installs in
    /// the pool: a key's first wave is at most that many setups (amendment
    /// 2's §3.20), as on SSH. The attempts in flight on its site also stay
    /// within `target`, the believed limit its limit machine sets (the ceiling
    /// until a limit is found).
    fn admits_attempt(&self, pool_key: &pool::Key, target: usize) -> bool {
        let capacity = self.client.pool().capacity();
        let on_host = self
            .entries
            .values()
            .filter(|entry| entry.preparing.is_some() && entry.pool_key.host == pool_key.host);
        on_host.clone().count() < capacity.per_host.min(capacity.per_user_host)
            && on_host
                .filter(|entry| entry.pool_key.port == pool_key.port)
                .count()
                < target
    }

    /// One attempt of `key`'s open, with its own budget, afresh unless it
    /// continues the anonymous attempt `carry`, and the allocation its hold
    /// `left`.
    fn start_attempt(&mut self, key: &Key, carry: Option<Retry>, left: u64, carries_test: bool) {
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
        entry.attempts += 1;
        entry.carries_test = carries_test;
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
            let (result, connect, rejection) = client
                .prepare_attempt_judged(input, &cancelled, &mut carry.budget, &mut carry.challenge)
                .await;
            (result, connect, carry, rejection)
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
    let selector = open
        .destination
        .https_username
        .as_deref()
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
