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
//! to start the tests of the limit with a queued member as carrier (§4.7). The
//! restore steps (RESTORING) and the setup limit `Ns` are the next step, so
//! the module still allows dead code.
#![allow(dead_code, reason = "restore steps and the setup limit come later")]

mod control;
mod filter;
mod fsm;
mod governor;
mod hold;
mod notes;
mod scoped;
mod states;
mod timer;
mod windows;

pub(crate) use control::Ruling;
pub(crate) use filter::Signal;
pub(crate) use fsm::State;
pub(crate) use governor::{Admission, Conn, Governor, Scoped, TestToken, View, label};

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod control_hold_tests;
        mod control_pool_tests;
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
