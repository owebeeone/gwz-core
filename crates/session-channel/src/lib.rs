//! Carrying the session channel's frames: the two adapters of the core
//! session contract's §3, each implementing gwz-session-contract's
//! `FrameSink` and `FrameSource`.
//!
//! Authority: gwz-dev `dev-docs/GwzCoreSessionDesign.md` §3 (revision 5, as
//! `GwzCoreServerDesign.md` §3 and §8 amend it) and
//! `GwzCoreSessionCrateMap.md` §2; the steps are session plan CS1.2 and CS1.3.
//!
//! | Adapter | Guarantees | Clause |
//! | --- | --- | --- |
//! | [`pair`] | Two bounded queues, each holding the outstanding-call limit on the call lane plus the control reserve on the control lane; a `send` that never waits and refuses a full lane without dropping the frame; a `recv` that waits for a frame or the end; closure reported to both ends; tags 1 to 3 | §3; CS1.2; §15.10 |
//! | [`byte_stream`] | Each frame prefixed by its size as a little-endian `u32`, as taut-shape's interop tool frames it; a prefix over 64 MiB ends the session before anything is allocated; end of stream is closure; closing ends the peer's stream; tags 1 to 8 | §3; server design §3; CS1.3; §15.9 |
//!
//! Both end the session on a frame their carrier does not carry, and both run
//! the contract's conformance suite. Bytes only: nothing here decodes a body,
//! and the crate keeps no global state.
#![forbid(unsafe_code)]

mod byte_stream;
mod in_process;

pub use byte_stream::{ByteStreamEnd, byte_stream};
pub use in_process::{InProcessEnd, pair};

mod tests;
