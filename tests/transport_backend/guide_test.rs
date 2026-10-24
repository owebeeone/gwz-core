//! Compiles the example in `docs/TransportPlacement.md` ("Example: configure,
//! fetch once, remove") as an external consumer would: `gwz_core` is an extern
//! crate here, so the example reaches only its public API. `prepare.py` adds
//! this target to the candidate manifest; ordinary builds never compile it.
//! The example documents the Cli placement through the transport build's
//! `transport_host` facade, so it compiles with the transport switch only; in
//! a build without it this target is empty.
//! `guide_example.rs` stays byte-equal to the document's block
//! (`test_prepare.py`), so this wrapper holds the lint allowance: the example
//! defines a function that nothing calls.
#![allow(dead_code)]

cfg_if::cfg_if! {
    if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
        #[path = "guide_example.rs"]
        mod guide_example;
    }
}
