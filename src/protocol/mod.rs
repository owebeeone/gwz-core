pub mod convert;
pub mod transport_capabilities;

cfg_if::cfg_if! {
    // The 1.2.0 session projection (S7.1 (1.1.0) step C3a): the candidate schema adds the Cli
    // placement, the embedded message and the capability extras to the ordinary one, which
    // already carries everything 1.1.0 needs (C2.1). Only the session candidate selects it.
    if #[cfg(gwz_session_candidate)] {
        #[allow(clippy::redundant_closure)]
        #[allow(clippy::needless_question_mark)]
        #[rustfmt::skip]
        #[path = "candidate_generated.rs"]
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
