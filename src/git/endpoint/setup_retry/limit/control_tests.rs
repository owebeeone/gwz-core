//! One key end to end (§4.5 to §4.8), on a scripted clock: the §10.2 cases
//! that need no pool, endpoint or server.
use super::{
    control::*,
    filter::Signal::{self, Suspect, Throttle},
    fsm::State,
    notes::Note,
    states::{ConnEvent, ConnId},
    timer::Spread,
    windows::{AttemptId, AttemptKind, Own},
};

pub(super) type Flight = (AttemptId, ConnId);
const CLOCKED: ConnEvent = ConnEvent::Started { clocked: true };

pub(super) struct Rig {
    pub(super) limit: Limit,
    pub(super) now: u64,
    attempts: u64,
    conns: u64,
    pub(super) target: usize,
}

impl Rig {
    pub(super) fn with(ceiling: usize, adaptive: bool, permille: u64) -> Self {
        let limit = Limit::new(ceiling, adaptive, Spread::fixed(permille));
        Self {
            limit,
            now: 0,
            attempts: 0,
            conns: 0,
            target: 0,
        }
    }
    pub(super) fn new(ceiling: usize) -> Self {
        Self::with(ceiling, true, 1_000)
    }
    /// A key at `N = n` in STABLE, found by one conclusive Throttle.
    pub(super) fn stable(ceiling: usize, n: usize) -> Self {
        let mut rig = Self::new(ceiling);
        rig.connect(n);
        let a = rig.go(Start::Ordinary);
        assert_eq!(rig.refuse(a, Throttle), Ruling::Overload { n });
        assert_eq!(rig.limit.state(), State::Stable);
        rig
    }
    pub(super) fn at(&mut self, now: u64) -> Option<Action> {
        self.now = now;
        self.limit.tick(now)
    }
    pub(super) fn connect(&mut self, count: usize) {
        for _ in 0..count {
            self.conns += 1;
            self.limit.conn(ConnId(self.conns), CLOCKED, self.now);
            self.limit
                .conn(ConnId(self.conns), ConnEvent::Connected, self.now);
        }
    }
    pub(super) fn conn(&mut self, id: u64, event: ConnEvent) {
        self.limit.conn(ConnId(id), event, self.now);
    }
    pub(super) fn start(&mut self, start: Start) -> Option<Flight> {
        self.attempts += 1;
        self.conns += 1;
        let flight = (AttemptId(self.attempts), ConnId(self.conns));
        let target = self
            .limit
            .admit(flight.0, start, Own::New(flight.1), true, self.now)?;
        self.target = target;
        Some(flight)
    }
    pub(super) fn go(&mut self, start: Start) -> Flight {
        self.start(start).expect("admitted")
    }
    pub(super) fn ok(&mut self, (attempt, conn): Flight) -> Ruling {
        self.limit.conn(conn, ConnEvent::Connected, self.now);
        self.finish(attempt, Outcome::Succeeded { fresh: true })
    }
    pub(super) fn refuse(&mut self, flight: Flight, signal: Signal) -> Ruling {
        self.refuse_with(flight, signal, None, false)
    }
    pub(super) fn refuse_with(
        &mut self,
        (attempt, conn): Flight,
        signal: Signal,
        ra: Option<u64>,
        post: bool,
    ) -> Ruling {
        self.limit.conn(conn, ConnEvent::SetupEnded, self.now);
        self.finish(
            attempt,
            Outcome::Refused {
                signal,
                retry_after_ms: ra,
                post,
            },
        )
    }
    pub(super) fn finish(&mut self, attempt: AttemptId, outcome: Outcome) -> Ruling {
        self.limit
            .result(attempt, outcome, self.now)
            .expect("in flight")
    }
    pub(super) fn demand(&mut self, needing_new: usize, non_final: usize) {
        self.limit.set_demand(needing_new, non_final);
    }
    pub(super) fn plan(&mut self) -> Option<TestPlan> {
        self.limit.test_due(self.now)
    }
}

#[test]
fn a_new_key_is_saturated_and_admits_to_the_ceiling() {
    let mut rig = Rig::new(4);
    assert_eq!((rig.limit.state(), rig.limit.n()), (State::Saturated, 4));
    for _ in 0..4 {
        rig.go(Start::Ordinary);
    }
    assert_eq!(rig.target, 4);
    assert!(rig.start(Start::Ordinary).is_none());
}

#[test]
fn a_leased_exchange_is_not_a_new_connection_and_its_429_excludes_its_own_connection() {
    // R10: eight Connected, the exchange on connection 3 is refused.
    let mut rig = Rig::new(32);
    rig.connect(8);
    rig.limit
        .admit(
            AttemptId(1),
            Start::Ordinary,
            Own::Leased(ConnId(3)),
            true,
            0,
        )
        .unwrap();
    let ruling = rig.limit.result(
        AttemptId(1),
        Outcome::Refused {
            signal: Throttle,
            retry_after_ms: None,
            post: false,
        },
        0,
    );
    assert_eq!(
        ruling,
        Some(Ruling::Overload { n: 7 }),
        "min(Connected 8, hi 7)"
    );
}

#[test]
fn an_attempt_that_is_not_in_flight_has_no_result() {
    let mut rig = Rig::new(4);
    assert_eq!(rig.limit.result(AttemptId(9), Outcome::Ended, 0), None);
}

#[test]
fn a_throttled_wave_lowers_n_once_then_its_successes_raise_it_to_what_the_server_held() {
    // Cases 1, 15, 24 and R3: C = 32, the server holds 8.
    let mut rig = Rig::new(32);
    let wave: Vec<_> = (0..32).map(|_| rig.go(Start::Ordinary)).collect();
    for flight in &wave[..3] {
        assert_eq!(rig.ok(*flight), Ruling::Succeeded);
    }
    assert_eq!(rig.refuse(wave[3], Throttle), Ruling::Overload { n: 3 });
    for (i, flight) in wave[4..9].iter().enumerate() {
        rig.ok(*flight);
        assert_eq!(rig.limit.n(), 4 + i, "each success raises N to Connected");
    }
    for flight in &wave[9..] {
        assert_eq!(rig.refuse(*flight, Throttle), Ruling::Inconclusive);
    }
    assert_eq!((rig.limit.n(), rig.limit.state()), (8, State::Stable));
    assert!(
        rig.start(Start::Ordinary).is_none(),
        "no start beyond Possible < N"
    );
    assert_eq!(
        rig.limit.drain_notes(),
        [Note::Overload { n: 8, ceiling: 32 }]
    );
}

#[test]
fn a_suspect_wave_opens_one_confirmation_and_tests_at_connected_plus_one() {
    // Case 2: the server holds 8 and resets the rest.
    let mut rig = Rig::new(32);
    let wave: Vec<_> = (0..32).map(|_| rig.go(Start::Ordinary)).collect();
    for flight in &wave[..8] {
        rig.ok(*flight);
    }
    assert_eq!(rig.refuse(wave[8], Suspect), Ruling::Confirmation);
    assert!(rig.limit.confirmation_open());
    rig.demand(24, 24);
    assert!(
        !rig.limit.admits_ordinary(0),
        "no fill while a confirmation is open"
    );
    let plan = rig.plan().unwrap();
    assert_eq!(
        (plan.kind, plan.ready),
        (AttemptKind::Confirming, false),
        "the wave has not resolved"
    );
    for flight in &wave[9..] {
        assert_eq!(
            rig.refuse(*flight, Suspect),
            Ruling::Inconclusive,
            "the rest of the wave"
        );
    }
    let plan = rig.plan().unwrap();
    assert_eq!((plan.target, plan.ready), (9, true));
    let test = rig.go(Start::Confirming);
    assert_eq!(rig.target, 9);
    assert!(rig.plan().is_none(), "one test at a time");
    assert_eq!(rig.refuse(test, Suspect), Ruling::Overload { n: 8 });
    assert!(!rig.limit.confirmation_open());
    assert_eq!((rig.limit.n(), rig.limit.state()), (8, State::Stable));
}

#[test]
fn a_refuted_suspect_changes_nothing_and_prints_nothing() {
    // Case 6a: one reset at concurrency 9 against an unlimited server.
    let mut rig = Rig::new(32);
    rig.connect(8);
    let a = rig.go(Start::Ordinary);
    assert_eq!(rig.refuse(a, Suspect), Ruling::Confirmation);
    rig.demand(1, 1);
    let test = rig.go(Start::Confirming);
    assert_eq!(rig.target, 9);
    assert_eq!(rig.ok(test), Ruling::Refuted);
    assert!(!rig.limit.confirmation_open());
    assert_eq!((rig.limit.n(), rig.limit.state()), (32, State::Saturated));
    assert_eq!(rig.limit.drain_notes(), []);
}

#[test]
fn an_unfair_confirming_test_is_rearmed_and_k_is_taken_again() {
    let mut rig = Rig::new(32);
    rig.connect(8);
    let a = rig.go(Start::Ordinary);
    rig.refuse(a, Suspect);
    rig.demand(1, 1);
    let test = rig.go(Start::Confirming);
    rig.conn(3, ConnEvent::Closing); // found dead during the window
    assert_eq!(rig.refuse(test, Suspect), Ruling::Unfair);
    assert!(rig.limit.confirmation_open(), "still open");
    assert_eq!(
        rig.plan().map(|p| p.ready),
        Some(false),
        "closing is not quiet"
    );
    rig.conn(3, ConnEvent::Disposed);
    rig.at(250);
    let plan = rig.plan().unwrap();
    assert_eq!((plan.target, plan.ready), (8, true), "k = 7 + 1");
}

#[test]
fn an_overload_while_a_confirmation_is_open_closes_it() {
    // Case 40: a reset opens it; an earlier co-attempt's 429 is an Overload.
    let mut rig = Rig::new(32);
    rig.connect(8);
    let early = rig.go(Start::Ordinary);
    let reset = rig.go(Start::Ordinary);
    assert_eq!(rig.refuse(reset, Suspect), Ruling::Confirmation);
    assert_eq!(rig.refuse(early, Throttle), Ruling::Overload { n: 8 });
    assert!(!rig.limit.confirmation_open());
    assert_eq!(rig.limit.state(), State::Stable);
}

#[test]
fn a_confirming_test_may_be_carried_by_a_final_attempt_but_a_probe_may_not() {
    // Case 45 and §4.7.
    let mut rig = Rig::new(32);
    rig.connect(8);
    let a = rig.go(Start::Ordinary);
    rig.refuse(a, Suspect);
    rig.demand(3, 0);
    assert_eq!(rig.plan().map(|p| p.kind), Some(AttemptKind::Confirming));
    rig.demand(0, 0);
    assert_eq!(
        rig.plan(),
        None,
        "the confirmation stays open for want of a carrier"
    );
    assert!(rig.limit.confirmation_open());
    assert!(
        !rig.limit.admits_ordinary(0),
        "and never re-offers the refused width"
    );
    let mut rig = Rig::stable(32, 8);
    rig.at(500);
    rig.demand(3, 0);
    assert_eq!(rig.plan(), None, "a probe is never given a final attempt");
    rig.demand(3, 1);
    assert_eq!(rig.plan().map(|p| p.kind), Some(AttemptKind::Probe));
}

#[test]
fn a_probe_fills_to_n_waits_for_quiet_then_tests_n_plus_one() {
    // Cases 35 and 41's machine side.
    let mut rig = Rig::stable(32, 8);
    rig.conn(1, ConnEvent::ServerClosed);
    rig.conn(2, ConnEvent::ServerClosed);
    rig.demand(10, 10);
    rig.at(499);
    assert_eq!(rig.plan(), None, "the timer has not expired");
    rig.at(500);
    assert_eq!(rig.plan().map(|p| p.ready), Some(false), "only 6 Connected");
    assert!(rig.limit.admits_ordinary(500), "ordinary starts fill to N");
    let fill = [rig.go(Start::Ordinary), rig.go(Start::Ordinary)];
    assert!(!rig.limit.admits_ordinary(500));
    assert_eq!(
        rig.plan().map(|p| p.ready),
        Some(false),
        "a setup is in flight"
    );
    assert!(
        rig.limit.closes_suppressed(500),
        "nothing is closed or evicted"
    );
    for flight in fill {
        rig.ok(flight);
    }
    let plan = rig.plan().unwrap();
    assert_eq!(
        (plan.kind, plan.target, plan.ready),
        (AttemptKind::Probe, 9, true)
    );
    assert!(!rig.limit.closes_suppressed(500));
    let probe = rig.go(Start::Probe);
    assert_eq!(rig.target, 9);
    assert_eq!(rig.limit.state(), State::Probing);
    assert!(rig.limit.is_test_carrier(probe.0), "never deferred");
    assert!(!rig.limit.admits_ordinary(500) && rig.plan().is_none());
    assert_eq!(rig.refuse(probe, Throttle), Ruling::RefusedTest);
    assert!(!rig.limit.is_test_carrier(probe.0));
    assert_eq!((rig.limit.n(), rig.limit.state()), (8, State::Stable));
    assert_eq!(
        rig.limit.next_deadline(500),
        Some(1_500),
        "T doubled to 1 s"
    );
}

#[test]
fn closes_are_not_suppressed_when_the_base_fails_and_the_test_stays_due() {
    let mut rig = Rig::stable(32, 8);
    rig.conn(1, ConnEvent::ServerClosed);
    rig.demand(1, 1);
    rig.at(500);
    assert_eq!(rig.plan().map(|p| p.ready), Some(false));
    assert!(
        !rig.limit.closes_suppressed(500),
        "quiet with Connected 7 < N: admission resumes"
    );
    assert!(rig.limit.admits_ordinary(500));
}

#[test]
fn no_connection_is_opened_only_to_test() {
    let mut rig = Rig::stable(32, 8);
    rig.at(60_000);
    assert_eq!(rig.plan(), None);
    assert!(rig.start(Start::Probe).is_none());
    rig.demand(1, 1);
    assert_eq!(
        rig.plan().map(|p| p.ready),
        Some(true),
        "the next carrier takes it"
    );
}

#[test]
fn a_successful_probe_climbs_one_connection_per_test_to_the_ceiling() {
    // Cases 5b, 30 and 37: DISCOVERING needs no timer, SATURATED tests nothing.
    let mut rig = Rig::stable(32, 8);
    rig.demand(30, 30);
    rig.at(500);
    for n in 8..32 {
        let plan = rig.plan().expect("a test is due");
        assert_eq!(
            (plan.kind, plan.target, plan.ready),
            (AttemptKind::Probe, n + 1, true)
        );
        let probe = rig.go(Start::Probe);
        assert_eq!(rig.ok(probe), Ruling::Succeeded);
        assert_eq!(rig.limit.n(), n + 1);
        let want = if n + 1 == 32 {
            State::Saturated
        } else {
            State::Discovering
        };
        assert_eq!(rig.limit.state(), want);
    }
    assert_eq!(rig.plan(), None, "nothing is tested above the ceiling");
    assert_eq!(
        rig.limit.drain_notes(),
        [
            Note::Overload { n: 8, ceiling: 32 },
            Note::BackAtCeiling { ceiling: 32 }
        ]
    );
}

#[test]
fn a_refused_test_in_discovering_resets_the_timer_to_t0() {
    let mut rig = Rig::stable(32, 8);
    rig.demand(30, 30);
    rig.at(500);
    let probe = rig.go(Start::Probe);
    rig.ok(probe);
    let probe = rig.go(Start::Probe);
    assert_eq!(rig.refuse(probe, Throttle), Ruling::RefusedTest);
    assert_eq!((rig.limit.n(), rig.limit.state()), (9, State::Stable));
    assert_eq!(rig.limit.next_deadline(500), Some(1_000), "T0, not 2 T0");
}

#[test]
fn a_probe_that_succeeds_unfair_raises_nothing_and_runs_again_when_quiet() {
    // Case 23, R2.
    let mut rig = Rig::stable(32, 8);
    rig.demand(5, 5);
    rig.at(500);
    let probe = rig.go(Start::Probe);
    rig.conn(3, ConnEvent::Closing);
    assert_eq!(rig.ok(probe), Ruling::Unfair);
    assert_eq!((rig.limit.n(), rig.limit.state()), (8, State::Stable));
    assert_eq!(
        rig.plan().map(|p| p.ready),
        Some(false),
        "due again, key not quiet"
    );
    rig.conn(3, ConnEvent::Disposed);
    rig.at(750);
    assert_eq!(rig.plan().map(|p| (p.target, p.ready)), Some((9, true)));
}

#[test]
fn a_test_served_by_reuse_is_unfair_and_never_a_success() {
    // Case 48.
    let mut rig = Rig::stable(32, 8);
    rig.demand(5, 5);
    rig.at(500);
    let (attempt, conn) = rig.go(Start::Probe);
    rig.conn(conn.0, ConnEvent::SetupEnded);
    let ruling = rig
        .limit
        .result(attempt, Outcome::Succeeded { fresh: false }, 500);
    assert_eq!(ruling, Some(Ruling::Unfair));
    assert_eq!((rig.limit.n(), rig.limit.state()), (8, State::Stable));
}

#[test]
fn a_test_that_ended_with_no_verdict_is_due_again() {
    let mut rig = Rig::stable(32, 8);
    rig.demand(5, 5);
    rig.at(500);
    let (attempt, conn) = rig.go(Start::Probe);
    rig.conn(conn.0, ConnEvent::SetupEnded);
    assert_eq!(
        rig.limit.result(attempt, Outcome::Ended, 500),
        Some(Ruling::Ended)
    );
    assert_eq!(rig.limit.state(), State::Stable);
    assert_eq!(rig.plan().map(|p| p.ready), Some(true));
}

#[test]
fn refused_tests_at_a_steady_limit_stay_within_the_bound_with_one_test_at_a_time() {
    // Case 4 at jitter 0.8: tests at 0.4, 1.2, 2.8, 6.0, 12.4, 25.2 s, then
    // every 24 s.
    let mut rig = Rig::with(32, true, 800);
    rig.connect(8);
    let a = rig.go(Start::Ordinary);
    rig.refuse(a, Throttle);
    rig.demand(200, 200);
    let mut at = Vec::new();
    while at.len() < 8 {
        let deadline = rig.limit.next_deadline(rig.now).expect("a timer");
        rig.at(deadline);
        let plan = rig.plan().expect("due at its deadline");
        assert!(plan.ready && plan.target == 9);
        let probe = rig.go(Start::Probe);
        assert!(rig.plan().is_none() && !rig.limit.admits_ordinary(rig.now));
        assert_eq!(rig.refuse(probe, Throttle), Ruling::RefusedTest);
        at.push(deadline);
    }
    assert_eq!(
        at,
        [400, 1_200, 2_800, 6_000, 12_400, 25_200, 49_200, 73_200]
    );
    assert_eq!(rig.limit.n(), 8);
}
