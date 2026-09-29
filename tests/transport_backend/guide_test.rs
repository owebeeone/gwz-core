//! Compiles the example in `docs/TransportPlacement.md` ("Example: configure,
//! fetch once, remove") as an external consumer would: `gwz_core` is an extern
//! crate here, so the example reaches only its public API. `prepare.py` adds
//! this target to the candidate manifest; ordinary builds never compile it.
//! `guide_example.rs` stays byte-equal to the document's block
//! (`test_prepare.py`), so this wrapper holds the lint allowance: the example
//! defines a function that nothing calls.
#![allow(dead_code)]

#[path = "guide_example.rs"]
mod guide_example;
