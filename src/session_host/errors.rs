//! How gwz-core reports the session crates' errors as its own. The crates
//! return crate-local errors and core keeps its model errors (gwz-dev
//! `dev-docs/GwzCoreSessionCrateMap.md` §3, §6 step 4), so each maps here,
//! with the crate's words.

use gwz_session_host::{InvalidLimits, Refused};

use crate::model::{ErrorCode, ModelError};

/// `open` refuses limits a session cannot have with `invalid_request`,
/// before any effect (§1, §9).
impl From<InvalidLimits> for ModelError {
    fn from(error: InvalidLimits) -> Self {
        ModelError::new(ErrorCode::InvalidRequest, error.to_string())
    }
}

/// A gate refuses a crossing, and a token a registration, with `cancelled`
/// (§4.2, §5.3), as CS1.4's gate did: "the operation's token is cancelled"
/// or "the operation's gate is revoked".
impl From<Refused> for ModelError {
    fn from(refused: Refused) -> Self {
        ModelError::new(ErrorCode::Cancelled, refused.to_string())
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
