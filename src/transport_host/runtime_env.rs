//! The environment values the local carrier's per-user directory depends on, captured in one place.
//!
//! The Linux directory order of `GwzSplitCommandChannel.md` §1 starts from `$XDG_RUNTIME_DIR` and `$TMPDIR`. The carrier
//! (`carrier/`) reads no environment at all, so that no variable can move what its peer rule accepts (the source check
//! `scripts/checks/check_carrier_peer_rule.py` fails the build on a read there), and so its directory resolver stays a
//! pure function of values passed in. This file is the one named place that reads them, at the process boundary: the
//! server command (plan step 2.3) calls [`capture`] once and hands the result to the carrier. macOS ignores both
//! values.
use super::carrier::local::RuntimeEnv;

/// The two variables, as the process has them now.
pub(crate) fn capture() -> RuntimeEnv {
    RuntimeEnv {
        xdg_runtime_dir: std::env::var_os("XDG_RUNTIME_DIR"),
        tmpdir: std::env::var_os("TMPDIR"),
    }
}
