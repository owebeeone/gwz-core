pub mod convert;
pub mod transport_capabilities;

// Generated from `protocol/gwz.taut.py` by `protocol/regen.py`. The schema composes the transport owner's
// export, so the owner's declarations (the embedded `Envelope`, `Scheme`, `AuthPolicy`, `Limits` and the
// rest) are re-exported from `gwz_transport::protocol`, never defined here a second time.
#[allow(clippy::redundant_closure)]
// The 0.8.0 emitter wraps fallible decode arms as `Ok(...?)`.
#[allow(clippy::needless_question_mark)]
#[rustfmt::skip]
#[path = "generated.rs"]
pub mod generated;
