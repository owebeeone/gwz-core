//! The HTTPS endpoint's TLS client stack: one per platform behind one surface.
//!
//! macOS and Windows use the platform's own (`native-tls`: Security.framework,
//! Schannel). Everywhere else it is OpenSSL, built here directly on the
//! `openssl` crate rather than through `native-tls`: the backend supplies the
//! trust roots itself there (`verify_paths`), and `native-tls` would parse the
//! system bundle twice, 24 ms of a Raspberry Pi's CPU on the critical path of
//! every command, to fill a store it then replaces (gwz-core-evidence,
//! `2026-10-13-https-fixed-cost-pi`).
//!
//! The surface, the same in both:
//! - [`Certificate`], a root, from its DER;
//! - [`Connector`], an endpoint's TLS configuration (`https_tls` builds it
//!   once and shares it), whose `connect` runs the client handshake over any
//!   async stream and yields a [`Stream`], or [`Refused`];
//! - [`tls_server_end_point`], the channel binding of a verified [`Stream`].
use std::sync::Arc;

cfg_if::cfg_if! {
    if #[cfg(any(windows, target_vendor = "apple"))] {
        mod native_stack;
        pub(crate) use native_stack::{Certificate, Connector, Stream, tls_server_end_point};
    } else {
        mod openssl_stack;
        pub(crate) use openssl_stack::{Certificate, Connector, Stream, tls_server_end_point};
    }
}

/// A handshake that did not complete; the stack's own words are not kept.
#[derive(Debug)]
pub(crate) struct Refused;

/// The platform's roots, where they are not the TLS backend's own.
pub(crate) type RootsLoader = Arc<dyn Fn() -> Vec<Certificate> + Send + Sync>;
