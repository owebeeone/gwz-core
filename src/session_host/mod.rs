//! The core session host of the core session contract (gwz-dev
//! `dev-docs/GwzCoreSessionDesign.md`, accepted at revision 5, as the connection
//! reuse and server designs amend it), built in the steps of the session plan
//! (`dev-docs/GwzCoreSessionPlan.md`).
//!
//! Phase 1 freezes these interfaces, and nothing calls them yet:
//! - CS1.4: the host context and the session context (§5.6), the operation gate
//!   and its cancellation token (O7, O8), the handler's context (§5.2, §16), the
//!   limits of §1 and `open` (§9), in `context`, `gate` and `limits`;
//! - CS1.5: the endpoint environment snapshot (§5.6, O9), in `environment`.
//! - CS1.9: the host context's bounded `shutdown` and its `ShutdownReport`,
//!   `open`'s `transport_off` and the snapshot's zeroization (§5.6 as amended).
//!
//! The channel's frames and queues are CS1.2's. `ClientChannel` is the seam it
//! fills.

pub(crate) mod context;
pub(crate) mod environment;
// The gate's callers are CS1.6's dispatch and Phase 2's host (CS2.2, CS2.4,
// CS2.5, CS2.8, CS2.11, CS2.12); until they land, nothing in production reaches it.
#[allow(
    dead_code,
    reason = "frozen by CS1.4; CS1.6 and Phase 2 are its first callers"
)]
pub(crate) mod gate;
pub(crate) mod limits;

pub use context::{ClientChannel, HostContext, SessionOptions, ShutdownReport, open};
pub use environment::EnvironmentSnapshot;
pub use limits::{Limits, MAX_FRAME_BYTES, MAX_READ_WAIT};
