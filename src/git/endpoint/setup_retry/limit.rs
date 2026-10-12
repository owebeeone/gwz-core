//! Limit discovery: the adaptive concurrency design's §4 (gwz-core
//! dev-docs/GwzTransportAdaptiveConcurrencyDesign.md, revision 11), as pure
//! state with an injected clock and no I/O, threads or globals. It decides
//! one number per pool key, the believed limit `N`; it does not decide whether
//! a connection is up (§4).
//!
//! Cut 1: [`states`] (§4.1's connection-state table and the
//! settle time), [`windows`] (§4.3's window of an attempt) and [`filter`]
//! (§4.4's judgement and §4.5's evidence filter). Cut 2: [`fsm`] (the machine, §4.6),
//! [`timer`] (§4.7), [`hold`] (a `Retry-After` hold), [`notes`] (§9, as data)
//! and [`control`] (one key: the events, the queries, the confirmation, the
//! test slot and the admission rules of §4.5).
//!
//! **Connected includes a background close.** A connection whose close runs
//! in the background (SSH background close design, revision 3, §7) is
//! Connected until it is discarded or returned to the pool. Its owner reports
//! no state change when the background close begins, and `Closing` only if the
//! close ends in a discard. Reporting `Closing` at a fetch's completion would
//! make every key non-quiet and every test window unfair.
//!
//! Cut 3: [`governor`], one pool's machines behind one lock, per operation
//! (§4.1), which the pool host reports to and the endpoints consult through
//! [`Scoped`]. The endpoints use it for the gate and the admission target,
//! for holds and the idle discard, for the judgement of a refused setup, and
//! to start the tests of the limit with a queued member as carrier (§4.7).
//! Cut 4: RESTORING (§5.5), in [`control`] and [`fsm`]: after an outage that
//! lowered `N`, judged doubling steps back up to `N_good`, which the endpoints
//! start when the retry machine's health changes ([`Scoped::health`]).
//! Cut 5: SSH's setup limit `Ns` (§4.10), in `control::setup_limit`: a second
//! machine and timer over the setups in flight, fed by [`Signal::SetupDrop`]
//! and never by `N`'s evidence, with its own window ([`windows`]'s `lo_s` and
//! `hi_s`) and its test (`AttemptKind::SetupProbe`). The POST-level holds are
//! the HTTPS worker's `Gate` and [`Scoped::continuing_may_begin`].
//! Cut 6: the notes drain (§9). The machines make [`notes::Note`]s as data;
//! [`Governor::drain_all_notes`] hands over every operation's, those of ended
//! operations too, and a [`NoteSink`] the caller injects turns each into its
//! `note: ` line (host and port from the site) and gives it to the reader. Each
//! endpoint gives its governor's notes to the sink once per pass; there is no
//! global, and a command with no reader (machine-readable output) has none.
//!
//! Cut 7: [`capacity`], the words of a member's final `Capacity` failure
//! (§5.3), one text per cause, and the `capacity` detail that carries a report
//! from the endpoint that ends the member to the code that words the error.
//!
//! The module still allows dead code, for what only the machine's own tests
//! drive and no endpoint calls: `Limit::admit` and `admits_ordinary` with
//! `Start` (the whole admission rule driven by hand), `setup_state`, and the
//! timer's `fixed`. The capacity report is no longer one of them: the endpoints
//! build it and the code that words an error reads it.
#![allow(
    dead_code,
    reason = "the machine's tests drive admit, Start, setup_state and the timer's fixed; no endpoint does"
)]

mod capacity;
mod control;
mod filter;
mod fsm;
mod governor;
mod hold;
mod note_sink;
mod notes;
mod scoped;
mod states;
mod timer;
mod windows;

pub(crate) use capacity::{CapacityReport, LastSignal, ServerCapacity};
pub(crate) use control::Ruling;
pub(crate) use filter::Signal;
pub(crate) use fsm::State;
pub(crate) use governor::{Admission, Conn, Governor, Scoped, TestToken, View, label};
pub use note_sink::NoteSink;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod control_hold_tests;
        mod control_pool_tests;
        mod control_restore_tests;
        mod control_setup_tests;
        mod control_tests;
        mod filter_tests;
        mod fsm_tests;
        mod governor_tests;
        mod hold_notes_tests;
        mod states_tests;
        mod timer_tests;
        mod windows_tests;
    }
}
