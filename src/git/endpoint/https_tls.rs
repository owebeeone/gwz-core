//! One endpoint's TLS configuration, built once for all of its connections.
//!
//! Building a connector parses the CA bundle twice (the platform's default
//! paths, then the bundle `openssl-probe` finds), about 23 ms of CPU for the
//! 150 certificates of a Linux system bundle, and every connection built its
//! own: 17 builds at 16 members, most of a command's CPU. A connection's trust
//! is its endpoint's configuration and nothing else (the endpoint's CA file's
//! certificates beside the platform's roots), so the configuration an
//! endpoint's connector holds is the same for every connection it makes.
//!
//! The platform's roots are the TLS backend's own, unless the endpoint names
//! a loader for them (`RootsLoader`), which replaces them: the backend's are
//! left out and the loader's are the only built-in roots. Only OpenSSL builds
//! have one (`verify_paths`).
//! Another endpoint, with another CA file, has a configuration of its own.
use std::{
    io,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

/// The platform's roots, where they are not the TLS backend's own.
pub(crate) type RootsLoader = Arc<dyn Fn() -> Vec<native_tls::Certificate> + Send + Sync>;

#[derive(Clone)]
pub(crate) struct SharedTls(Arc<Inner>);
struct Inner {
    /// The CA file's certificates, each a root beside the platform's own.
    roots: Vec<native_tls::Certificate>,
    /// The platform's roots, in place of the backend's, if the endpoint says.
    platform: Option<RootsLoader>,
    /// The first build's outcome. A failed build is the failure of every
    /// connection, as a build per connection would have failed each alike.
    built: OnceLock<Result<native_tls::TlsConnector, io::ErrorKind>>,
    builds: AtomicUsize,
}
impl SharedTls {
    pub(crate) fn new(roots: Vec<native_tls::Certificate>, platform: Option<RootsLoader>) -> Self {
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
    pub(crate) fn connector(&self) -> io::Result<native_tls::TlsConnector> {
        let inner = &self.0;
        inner
            .built
            .get_or_init(|| {
                inner.builds.fetch_add(1, Ordering::SeqCst);
                let mut builder = native_tls::TlsConnector::builder();
                if let Some(platform) = &inner.platform {
                    builder.disable_built_in_roots(true);
                    for root in platform() {
                        builder.add_root_certificate(root);
                    }
                }
                for root in &inner.roots {
                    builder.add_root_certificate(root.clone());
                }
                builder.build().map_err(|_| io::ErrorKind::InvalidInput)
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
    cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
        /// How many times this configuration has been built.
        pub(crate) fn builds(&self) -> usize {
            self.0.builds.load(Ordering::SeqCst)
        }
    } }
}
