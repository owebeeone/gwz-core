//! Limit discovery: the adaptive concurrency design's §4 (gwz-core
//! dev-docs/GwzTransportAdaptiveConcurrencyDesign.md, revision 11), as pure
//! state with an injected clock and no I/O, threads or globals. It decides
//! one number per pool key, the believed limit `N`; it does not decide whether
//! a connection is up (§4).
//!
//! Cut 1 of the step: [`states`] (§4.1's connection-state table and the
//! settle time), [`windows`] (§4.3's window of an attempt) and [`filter`]
//! (§4.4's judgement and §4.5's evidence filter). Cut 2 adds the machine
//! (§4.6) and the probe timer (§4.7).
//!
//! **Connected includes a background close.** A connection whose close runs
//! in the background (SSH background close design, revision 3, §7) is
//! Connected until it is discarded or returned to the pool. Its owner reports
//! no state change when the background close begins, and `Closing` only if the
//! close ends in a discard. Reporting `Closing` at a fetch's completion would
//! make every key non-quiet and every test window unfair.
//!
//! The endpoints do not use these types yet, so the module allows dead code
//! until the integration step wires them.
#![allow(dead_code, reason = "wired by the integration step")]

mod filter;
mod states;
mod windows;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod filter_tests;
        mod states_tests;
        mod windows_tests;
    }
}
