//! Compiles the example in `docs/TransportPlacement.md` ("Example: configure,
//! fetch once, remove") as an external consumer would: `gwz_core` is an extern
//! crate here, so the example reaches only its public API. `prepare.py` adds
//! this target to the candidate manifest; ordinary builds never compile it.
//! The example documents the Cli placement, the 1.2.0 session host's transport
//! entry (S7.1 (1.1.0) step C3a), so it compiles in the session candidate only:
//! in the 1.1.0 transport build this target is empty.
//! `guide_example.rs` stays byte-equal to the document's block
//! (`test_prepare.py`), so this wrapper holds the lint allowance: the example
//! defines a function that nothing calls.
#![allow(dead_code)]

cfg_if::cfg_if! {
    if #[cfg(gwz_session_candidate)] {
        #[path = "guide_example.rs"]
        mod guide_example;
    }
}
