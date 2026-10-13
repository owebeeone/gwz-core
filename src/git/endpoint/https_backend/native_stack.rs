//! The platform's TLS client (Security.framework on macOS, Schannel on
//! Windows) through `native-tls`.
use super::{Refused, RootsLoader};
use std::io;
use tokio::io::{AsyncRead, AsyncWrite};

pub(crate) use native_tls::Certificate;

/// A verified TLS stream over `S`.
pub(crate) type Stream<S> = tokio_native_tls::TlsStream<S>;

/// One endpoint's TLS configuration.
#[derive(Clone)]
pub(crate) struct Connector(tokio_native_tls::TlsConnector);

impl Connector {
    /// The configuration that trusts the platform's own roots (`platform` is
    /// `None`) or only `platform`'s, and `roots` besides.
    pub(crate) fn build(
        platform: Option<&RootsLoader>,
        roots: &[Certificate],
    ) -> Result<Self, io::ErrorKind> {
        let mut builder = native_tls::TlsConnector::builder();
        if let Some(platform) = platform {
            builder.disable_built_in_roots(true);
            for root in platform() {
                builder.add_root_certificate(root);
            }
        }
        for root in roots {
            builder.add_root_certificate(root.clone());
        }
        builder
            .build()
            .map(|connector| Self(connector.into()))
            .map_err(|_| io::ErrorKind::InvalidInput)
    }

    /// The client handshake for `host` over `io`.
    pub(crate) async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        host: &str,
        io: S,
    ) -> Result<Stream<S>, Refused> {
        self.0.connect(host, io).await.map_err(|_| Refused)
    }
}

/// The `tls-server-end-point` channel binding (RFC 5929) of the server's
/// certificate, if the platform gives one.
pub(crate) fn tls_server_end_point<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &Stream<S>,
) -> Option<Vec<u8>> {
    stream.get_ref().tls_server_end_point().ok().flatten()
}
