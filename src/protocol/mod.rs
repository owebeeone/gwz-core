pub mod convert;

#[allow(clippy::redundant_closure)]
// The 0.8.0 emitter wraps fallible decode arms as `Ok(...?)`.
#[allow(clippy::needless_question_mark)]
#[rustfmt::skip]
#[path = "generated.rs"]
pub mod generated;
