//! The session channel's contract: frames, the tag registry, lanes, the
//! channel's limits, and the [`FrameSink`] and [`FrameSource`] traits that
//! every channel adapter implements.
//!
//! Authority: gwz-dev `dev-docs/GwzCoreSessionDesign.md`, the core session
//! contract at revision 5, §1 and §3, with §3 as `GwzCoreServerDesign.md`
//! §3 and §8 amend it (tags 4 to 8, for byte streams only), and
//! `GwzCoreSessionCrateMap.md` §2 and §7, under which `send` takes a lane
//! and core classifies each frame.
//!
//! | Guarantee | Clause |
//! | --- | --- |
//! | A frame is a tag byte and a body; the body is bytes, never decoded here | §3, §4.1; map §1 |
//! | Tags 1 to 3 on both adapters, 4 to 8 on byte streams only; 16 to 31 reserved; any other tag is a protocol error | §3; server design §3, §8 |
//! | No frame exceeds 64 MiB, its tag byte and body together | §1, §3 |
//! | A protocol error ends the session, and the frame comes back to its sender | §3 |
//! | Each queue holds the outstanding-call limit on the call lane plus the control reserve on the control lane; the outstanding-call limit never refuses a control frame | §3 |
//! | A full lane refuses the frame and gives it back; the channel stays open | §3, §9 |
//! | Frames arrive in the order sent, whatever their lanes, each once and whole | §3 |
//! | Closure is reported to both ends; the frames sent before it still arrive | §3, §8 |
//!
//! Bytes only: this crate holds no GWZ or Taut protocol type and decodes no
//! CBOR. Encoding and decoding bodies, reporting a body that does not decode,
//! checking call-ID order and classifying each frame's lane are core's (crate
//! map §3). It keeps no global state.
//!
//! The `contract-tests` feature adds [`contract_tests`], the shared
//! conformance suite that every adapter runs against itself (LBT-009).
#![forbid(unsafe_code)]

mod channel;
mod frame;
mod limits;

pub use channel::{Closed, FrameSink, FrameSource, Lane, SendError};
pub use frame::{Carrier, Frame, FrameError, MAX_FRAME_BYTES, RESERVED_TAGS, Tag};
pub use limits::{Limits, LimitsError};

#[cfg(feature = "contract-tests")]
pub mod contract_tests {
    //! The shared conformance suite for channel adapters (LBT-009), enabled by
    //! the `contract-tests` feature, which only dev-dependencies turn on.
    //!
    //! An adapter runs [`run_all`] against itself through a
    //! [`ChannelFixture`] that makes fresh connected pairs of its ends. The
    //! cases check what every adapter guarantees: order on both lanes and in
    //! both directions, whole frames under concurrent senders, closure
    //! reported to both ends after the frames sent before it, a closed end's
    //! refusals, and protocol errors ending the session. They check nothing
    //! an adapter may choose: whether `send` waits, how full a lane may get,
    //! and which of the two allowed reasons a protocol error's peer reports.

    mod suite;

    pub use suite::{
        ChannelFixture, a_frame_at_the_size_limit_arrives_whole,
        a_frame_over_the_size_limit_ends_the_session,
        a_tag_the_carrier_does_not_carry_ends_the_session,
        a_waiting_receiver_returns_when_the_peer_closes,
        closing_delivers_what_was_sent_then_the_closure, concurrent_senders_keep_each_frame_whole,
        dropping_an_end_closes_the_channel, frames_arrive_in_order_on_both_lanes, run_all,
    };
}

mod tests;
