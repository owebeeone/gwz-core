//! Carrying the session channel's frames: the two adapters of the core
//! session contract's §3. The in-process pair implements gwz-session-contract's
//! `FrameSink` and `FrameSource`; a byte-stream end implements `FrameSink` only
//! and delivers what it reads to a core-supplied `FrameRouter` (operator
//! decision D-N).
//!
//! Authority: gwz-dev `dev-docs/GwzCoreSessionDesign.md` §3 (revision 5, as
//! `GwzCoreServerDesign.md` §3 and §8 and decision D-N amend it) and
//! `GwzCoreSessionCrateMap.md` §2; the steps are session plan CS1.2 and CS1.3,
//! and convergence plan 2.1.
//!
//! | Adapter | Guarantees | Clause |
//! | --- | --- | --- |
//! | [`pair`] | Two bounded queues, each holding the outstanding-call limit on the call lane plus the control reserve on the control lane; a `send` that never waits, ignores its key and refuses a full lane without dropping the frame; a `recv` that waits for a frame or the end; closure reported to both ends; tags 1 to 3, 16 and 17 | §3; CS1.2; §15.10 |
//! | [`byte_stream`] | Each frame prefixed by its size as a little-endian `u32`, as taut-shape's interop tool frames it; a prefix over 64 MiB ends the session before anything is allocated; a reader thread that hands every frame to the router and never waits for a consumer; end of stream is closure; closing ends the peer's stream; tags 1 to 8, 16 and 17; a writer that puts the oldest control frame ahead of ordinary frames of other targets and never overtakes a `SessionCall` (D-N, [`WriteQueue`]) | §3; server design §3; CS1.3; §15.9; D-N |
//!
//! Both end the session on a frame their carrier does not carry, and both run
//! the contract's conformance suite. Bytes only: nothing here decodes a body,
//! and the crate keeps no global state.
#![forbid(unsafe_code)]

mod byte_stream;
mod in_process;
mod write_queue;

pub use byte_stream::{ByteStreamEnd, byte_stream, peer_side_failure, write_frame};
pub use in_process::{InProcessEnd, pair};
pub use write_queue::WriteQueue;

mod test_router;
mod tests;
mod write_queue_tests;
