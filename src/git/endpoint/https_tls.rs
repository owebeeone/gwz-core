//! One endpoint's TLS configuration, built once for all of its connections.
//!
//! Building a connector costs CPU, and every connection built its own: 17
//! builds at 16 members, most of a command's CPU. On the OpenSSL build the cost
//! is the decode of the platform's roots, about 10 ms for the 150 certificates
//! of a Linux system bundle (`verify_paths`); the other builds decode only the
//! endpoint's CA file. A connection's trust
//! is its endpoint's configuration and nothing else (the endpoint's CA file's
//! certificates beside the platform's roots), so the configuration an
//! endpoint's connector holds is the same for every connection it makes.
//!
//! The platform's roots are the TLS backend's own, unless the endpoint names
//! a loader for them (`RootsLoader`), which replaces them: the backend's are
//! left out and the loader's are the only built-in roots. Only OpenSSL builds
//! have one (`verify_paths`), and there the backend (`https_backend`) is built
//! with an empty store, so the loader's roots are the only ones read, and the
//! OpenSSL stack has no fallback: with no loader it trusts only the CA file's.
//! Another endpoint, with another CA file, has a configuration of its own.
use super::https_backend::{Certificate, Connector};

/// The platform's roots, where they are not the TLS backend's own.
pub(crate) use super::https_backend::RootsLoader;
use std::{
    io,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
pub(crate) struct SharedTls(Arc<Inner>);
struct Inner {
    /// The CA file's certificates, each a root beside the platform's own.
    roots: Vec<Certificate>,
    /// The platform's roots, in place of the backend's, if the endpoint says.
    platform: Option<RootsLoader>,
    /// The first build's outcome. A failed build is the failure of every
    /// connection, as a build per connection would have failed each alike.
    built: OnceLock<Result<Connector, io::ErrorKind>>,
    builds: AtomicUsize,
}
impl SharedTls {
    pub(crate) fn new(roots: Vec<Certificate>, platform: Option<RootsLoader>) -> Self {
        Self(Arc::new(Inner {
            roots,
            platform,
            built: OnceLock::new(),
            builds: AtomicUsize::new(0),
        }))
    }
    /// The configuration, built by whoever asks first while the others wait for
    /// that one build. A blocking call: the connection's setup job is the one
    /// that makes it, on its own thread.
    pub(crate) fn connector(&self) -> io::Result<Connector> {
        let inner = &self.0;
        inner
            .built
            .get_or_init(|| {
                inner.builds.fetch_add(1, Ordering::SeqCst);
                Connector::build(inner.platform.as_ref(), &inner.roots)
            })
            .clone()
            .map_err(io::Error::from)
    }
    /// Starts the build on a blocking thread of `runtime`, so that it overlaps
    /// whatever comes before the first connection needs it (an open's
    /// preparation, the connection's name resolution) instead of following it.
    /// The connection that needs it waits for it, never builds a second.
    pub(crate) fn prebuild_on(&self, runtime: &tokio::runtime::Handle) {
        if self.0.built.get().is_some() {
            return;
        }
        let tls = self.clone();
        drop(runtime.spawn_blocking(move || {
            let _ = tls.connector();
        }));
    }
    /// `prebuild_on` the current runtime, if there is one; without one the
    /// first connection builds it.
    pub(crate) fn prebuild(&self) {
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            self.prebuild_on(&runtime);
        }
    }
    cfg_if::cfg_if! { if #[cfg(test)] {
        /// How many times this configuration has been built.
        pub(crate) fn builds(&self) -> usize {
            self.0.builds.load(Ordering::SeqCst)
        }
    } }
}
