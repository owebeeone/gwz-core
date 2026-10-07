use super::*;
cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[path = "permit_tests.rs"]
        mod permit_tests;

        /// A clock that moves only when a test advances it.
        #[derive(Clone)]
        pub(crate) struct ManualClock {
            now: Arc<Mutex<Instant>>,
        }
        impl ManualClock {
            pub(crate) fn new() -> Self {
                Self {
                    now: Arc::new(Mutex::new(Instant::now())),
                }
            }
            pub(crate) fn now(&self) -> Instant {
                *self.now.lock().unwrap_or_else(|e| e.into_inner())
            }
            pub(crate) fn advance(&self, by: Duration) {
                let mut now = self.now.lock().unwrap_or_else(|e| e.into_inner());
                *now += by;
            }
            pub(crate) fn clock(&self) -> Arc<dyn Fn() -> Instant + Send + Sync> {
                let now = self.now.clone();
                Arc::new(move || *now.lock().unwrap_or_else(|e| e.into_inner()))
            }
        }
        impl Control {
            /// A control on a test's clock, outside any job.
            pub(crate) fn scripted(
                aggregate: Option<Instant>,
                stall: Duration,
                cleanup: Duration,
                clock: Arc<dyn Fn() -> Instant + Send + Sync>,
            ) -> Arc<Self> {
                Arc::new(Self::new(aggregate, stall, cleanup, clock))
            }
            pub(crate) fn disposal_due(&self) -> bool {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.cancelled_at.is_some_and(|at| self.cleanup_due(at))
            }
        }
        impl<T: Send + 'static> Job<T> {
            /// Starts a job whose threads `spawn` creates, so a test can make
            /// thread creation fail deterministically.
            pub(crate) fn start_with(
                supervisor: &Supervisor,
                deadline: Option<Instant>,
                cleanup: Duration,
                work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
                spawn: impl FnMut(&str, Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>,
            ) -> io::Result<Self> {
                Self::start_inner(Place::Take(supervisor), deadline, Duration::ZERO, cleanup, wall_clock(), work, spawn)
            }
            /// Starts a job on a job budget of its own, for a test that is not
            /// about the budget.
            pub(crate) fn start_isolated(
                deadline: Option<Instant>,
                cleanup: Duration,
                work: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
            ) -> io::Result<Self> {
                Self::start(&Supervisor::new(), deadline, cleanup, work)
            }
        }

        fn scripted(stall: Duration, aggregate: Duration) -> (ManualClock, Arc<Control>) {
            let clock = ManualClock::new();
            let aggregate = (aggregate > Duration::ZERO).then(|| clock.now() + aggregate);
            let control = Control::scripted(aggregate, stall, Duration::from_secs(5), clock.clock());
            (clock, control)
        }

        #[test]
        fn progressing_waits_outlive_one_stall_allowance() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            for _ in 0..4 {
                control.begin_slice().unwrap();
                clock.advance(Duration::from_millis(600));
                control.end_slice(true).unwrap();
            }
            control.check().unwrap();
        }

        #[test]
        fn idle_slices_expire_as_stall_while_aggregate_remains() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            control.begin_wait().unwrap();
            let _ = control.quantum().unwrap();
            let _ = control.quantum().unwrap();
            clock.advance(Duration::from_millis(1_000));
            let error = control.check().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
        }

        #[test]
        fn completed_native_attempt_after_stall_is_rejected() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            control.begin_wait().unwrap();
            clock.advance(Duration::from_millis(1_001));
            let error = control.complete_wait().unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
        }

        #[test]
        fn terminal_stall_wins_over_a_poll_error() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            let error = control
                .wait_step(|_| {
                    clock.advance(Duration::from_millis(1_000));
                    Err(io::ErrorKind::ConnectionRefused.into())
                })
                .unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
        }

        #[test]
        fn handshake_completions_reset_the_stall() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(10_000));
            for _ in 0..3 {
                control.begin_slice().unwrap();
                clock.advance(Duration::from_millis(700));
                control.end_slice(true).unwrap();
            }
            control.check().unwrap();
        }

        #[test]
        fn disabled_stall_does_not_invent_a_deadline() {
            let (clock, control) = scripted(Duration::ZERO, Duration::from_secs(10));
            control.begin_wait().unwrap();
            clock.advance(Duration::from_secs(3));
            control.check().unwrap();
            clock.advance(Duration::from_millis(6_999));
            control.check().unwrap();
            clock.advance(Duration::from_millis(1));
            let error = control.check().unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Aggregate));
        }

        #[test]
        fn zero_aggregate_has_no_network_deadline() {
            let clock = ManualClock::new();
            let control = Control::scripted(None, Duration::ZERO, Duration::from_secs(5), clock.clock());
            control.begin_wait().unwrap();
            clock.advance(Duration::from_secs(30));
            control.complete_wait().unwrap();
            control.check().unwrap();
            control.cancel();
            assert!(!control.disposal_due());
            clock.advance(Duration::from_secs(5));
            assert!(control.disposal_due());
        }

        #[test]
        fn short_waits_fail_when_their_sum_passes_the_aggregate() {
            let (clock, control) = scripted(Duration::from_millis(1_000), Duration::from_millis(2_500));
            for _ in 0..2 {
                control.begin_slice().unwrap();
                clock.advance(Duration::from_millis(800));
                control.end_slice(true).unwrap();
            }
            control.begin_slice().unwrap();
            clock.advance(Duration::from_millis(900));
            let error = control.end_slice(true).unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Aggregate));
        }

        #[test]
        fn cancel_drops_a_success_inside_the_aggregate() {
            let (clock, control) = scripted(Duration::from_secs(3), Duration::from_secs(10));
            for _ in 0..4 {
                control.begin_slice().unwrap();
                clock.advance(Duration::from_millis(1_250));
                control.end_slice(true).unwrap();
            }
            control.cancel();
            let error = control.check().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
            assert!(timeout_reason(&error).is_none());
        }
    }
}
