pub mod convert;
pub mod transport_capabilities;

cfg_if::cfg_if! {
    if #[cfg(gwz_transport_candidate)] {
        #[allow(clippy::redundant_closure)]
        #[allow(clippy::needless_question_mark)]
        #[rustfmt::skip]
        #[path = "../../tests/transport_consumer/candidate/candidate_generated.rs"]
        pub mod generated;
    } else {
        #[allow(clippy::redundant_closure)]
        // The 0.8.0 emitter wraps fallible decode arms as `Ok(...?)`.
        #[allow(clippy::needless_question_mark)]
        #[rustfmt::skip]
        #[path = "generated.rs"]
        pub mod generated;
    }
}
