//! `gwz server`, core side (gwz-dev `dev-docs/GwzServerCore-Step2.3-Plan.md`; `GwzSplitCommandChannel.md` §4.3): the
//! composition that serves the workspace over local carriers. Behind the transport candidate switch, as the carrier
//! is.
//!
//! | Module | Step | Holds |
//! | --- | --- | --- |
//! | [`session`] | 2.3g1 | the policy each served connection gets (limits, stall bound, handshake bounds) and the call that serves one |
pub(crate) mod session;

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
