//! The session's table: records at receipt, admission, the classes and the
//! queue, cancellation, results and release (§4.2, §5.1, §5.3, §5.4).

mod admit;
mod cancel;
mod receive;
mod results;
mod settle;
mod state;

pub(crate) use admit::{admission_loop, schedule};
pub(crate) use cancel::cancelled_before_start;
pub(crate) use receive::{close_report, receive};
pub(crate) use settle::settle;
pub(crate) use state::{Phase, State, is_live, mark_delivered};
