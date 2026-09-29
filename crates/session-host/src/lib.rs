//! The core session host's machinery, generic over core's per-session data.
//!
//! Authority: gwz-dev `dev-docs/GwzCoreSessionCrateMap.md` §2 (the
//! `gwz-session-host` row and its notes) and §6 step 4, over the core session
//! contract, `GwzCoreSessionDesign.md` at revision 5 as the reuse and server
//! designs amend it. Step 4 moves three pieces of gwz-core's `session_host`
//! here, with crate-local error types:
//!
//! | Piece | What it guarantees | Clause |
//! | --- | --- | --- |
//! | The operation gate ([`CallControls`], [`OperationGate`], [`GateView`], [`GateScope`], [`CancellationToken`], [`HandlerContext`]) | Controls and gate created at receipt; one holder for cancelling and revoking and one for crossing; live, cancelled and revoked states; nesting stopped by capabilities, with a per-gate thread record as the backstop | O7, O8, §5.3, §5.6, §8 step 5; map §2 "Nesting without a thread-local" |
//! | The session limits ([`Limits`], [`MAX_READ_WAIT`], [`validate_limits`]) | The §1 defaults and `open`'s validation, and the channel's two limits for gwz-session-channel's `pair` | §1, §3, §9 |
//! | The supervisor ([`Supervisor`], [`SupervisedJob`]) | One thread per host context; quarantine; `shutdown(bound)` | §5.6; reuse design §7 |
//!
//! Core's per-session data is the type parameter `S`: a gate holds it
//! weakly and a crossing hands the closure a reference, and nothing here
//! looks inside it. The environment snapshot, the host context and `open`
//! therefore stay in gwz-core, which re-exports [`Limits`] and
//! [`MAX_READ_WAIT`] at their frozen `gwz_core::session_host` paths. Phase 2
//! of the session plan adds the host itself here: `serve` and the
//! `HostPorts` that core implements.
//!
//! Bytes only, and no GWZ protocol: nothing here encodes or decodes a
//! message, and no error here is core's. gwz-core maps [`Refused`] to
//! `cancelled` and [`InvalidLimits`] to `invalid_request`. The crate keeps no
//! global state: no static and no thread-local.
//!
//! The `test-support` feature adds [`test_support`], a view of the
//! supervisor's thread for gwz-core's tests; only core's dev-dependency turns
//! it on.
#![forbid(unsafe_code)]

use std::sync::{Mutex, MutexGuard, PoisonError};

mod gate;
mod limits;
mod supervisor;
mod token;

pub use gate::{CallControls, GateScope, GateState, GateView, HandlerContext, OperationGate};
pub use limits::{InvalidLimits, Limits, MAX_READ_WAIT, validate_limits};
pub use supervisor::{SuperviseError, SupervisedJob, Supervisor};
pub use token::{CancelRegistration, CancellationToken, Refused};

/// Every lock of the gate and the token guards state that a panic cannot
/// leave half-written, so a poisoned lock is recovered.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    //! A view of a supervisor's thread, for tests: whether it runs, and a
    //! bounded wait for its end. The crate's own tests use it, and gwz-core's
    //! tests of its host context reach it through the `test-support` feature.

    use std::sync::{Arc, PoisonError};
    use std::time::Duration;

    use crate::Supervisor;
    use crate::supervisor::{Shared, State};

    /// A test's view of a supervisor's thread. It can outlive the supervisor,
    /// and it keeps none of its jobs.
    pub struct SupervisorWatch {
        shared: Arc<Shared>,
    }

    /// A view of `supervisor`'s thread.
    pub fn watch(supervisor: &Supervisor) -> SupervisorWatch {
        SupervisorWatch {
            shared: Arc::clone(&supervisor.shared),
        }
    }

    impl SupervisorWatch {
        /// Whether the thread runs.
        pub fn running(&self) -> bool {
            self.shared.lock().running
        }

        /// Waits at most `timeout` until the supervisor has been released, by
        /// its drop or its shutdown, and its thread, if it ever started, has
        /// stopped. Returns whether it has.
        pub fn wait_ended(&self, timeout: Duration) -> bool {
            let ended = |state: &State| state.released && !state.running;
            let state = self.shared.lock();
            let (state, _) = self
                .shared
                .changed
                .wait_timeout_while(state, timeout, |state| !ended(state))
                .unwrap_or_else(PoisonError::into_inner);
            ended(&state)
        }
    }
}
