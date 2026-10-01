//! The core session host of the core session contract (gwz-dev
//! `dev-docs/GwzCoreSessionDesign.md`, accepted at revision 5, as the connection
//! reuse and server designs amend it), built in the steps of the session plan
//! (`dev-docs/GwzCoreSessionPlan.md`).
//!
//! Phase 1 freezes these interfaces, and nothing calls them yet:
//! - CS1.4: the host context and the session context (§5.6) and `open` (§9),
//!   in `context`;
//! - CS1.5: the endpoint environment snapshot (§5.6, O9), in `environment`;
//! - CS1.9: the host context's bounded `shutdown` and its `ShutdownReport`,
//!   `open`'s `transport_off` and the snapshot's zeroization (§5.6 as amended);
//! - CS1.2's core side: `open` returns the client end of gwz-session-channel's
//!   in-process pair, which carries gwz-session-contract's frames.
//!
//! The core session crate map (gwz-dev `dev-docs/GwzCoreSessionCrateMap.md`)
//! moved the rest into crates, which this module composes and re-exports at
//! its frozen paths:
//! - gwz-session-host: CS1.4's operation gate, cancellation token and handler
//!   context (O7, O8, §5.2), with the map's nesting design in place of a
//!   thread-local; the limits of §1 and their validation; and the host
//!   context's supervisor (§6 step 4). Core's `SessionContext` is the gates'
//!   per-session data, and `errors` maps the crate's errors to core's.
//! - gwz-session-contract and gwz-session-channel: the frames, lanes and
//!   channel of §3 (§6 step 3).

pub(crate) mod context;
pub(crate) mod environment;
mod errors;

pub use context::{
    CLEANUP_BOUND, ClientChannel, HostContext, SessionOptions, ShutdownReport, open,
};
pub use environment::EnvironmentSnapshot;
pub use gwz_session_contract::MAX_FRAME_BYTES;
// What a `ClientChannel`'s send and recv take and return (CS1.2's core side).
pub use gwz_session_contract::{
    Closed, Frame, FrameError, FrameSink, FrameSource, Lane, SendError, Tag,
};
pub use gwz_session_host::{Limits, MAX_READ_WAIT};
