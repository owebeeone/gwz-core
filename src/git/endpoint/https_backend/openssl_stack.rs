//! The OpenSSL TLS client, built on the `openssl` crate directly.
//!
//! What `native-tls` 0.2.18 built for gwz is rebuilt here without the part
//! that cost: its connector reads OpenSSL's default verify paths (a parse of
//! the system bundle), then the `openssl-probe` file and directories (another
//! parse), and only then replaces the store the endpoint's own roots go into.
//! This context is made with an empty store and takes the endpoint's roots
//! and nothing else, so the bundle is parsed once, by whoever decodes the
//! roots (`verify_paths`).
//!
//! Nothing else differs from what `native-tls` made: the context starts
//! from the options, mode, cipher list and verify mode of the `openssl` crate's
//! `SslConnector` (which `native-tls` builds on), the protocol floor is TLS
//! 1.2 and there is no ceiling (`native-tls`'s `TlsConnectorBuilder` defaults,
//! applied over whatever a host's `openssl.cnf` says), a session verifies the
//! peer's chain against the store and its host name or address as
//! `SslConnector` does, and sends the server name unless the host is an
//! address. `tests` checks that against a context made by `SslConnector`
//! itself, a server capped at TLS 1.1 against the floor, and the bytes of a
//! ClientHello and the channel binding against `native-tls`.
//!
//! There is no fallback to OpenSSL's default verify paths: a connector with no
//! roots loader trusts only the endpoint's own roots. The endpoint always
//! supplies the loader on this stack (`endpoint_environment`), which reads the
//! paths from the command's environment snapshot and not the process's.
use super::{Refused, RootsLoader};
use openssl::{
    error::ErrorStack,
    ssl::{
        Ssl, SslContext, SslContextBuilder, SslMethod, SslMode, SslOptions, SslVerifyMode,
        SslVersion,
    },
    x509::{X509, store::X509StoreBuilder, verify::X509CheckFlags},
};
use std::{io, net::IpAddr};
use tokio::io::{AsyncRead, AsyncWrite};

mod stream;
pub(crate) use stream::{Stream, tls_server_end_point};

/// A trusted root.
#[derive(Clone)]
pub(crate) struct Certificate(X509);

impl Certificate {
    /// The certificate in `der`.
    pub(crate) fn from_der(der: &[u8]) -> Result<Self, ErrorStack> {
        X509::from_der(der).map(Self)
    }

    /// The certificate as DER. The profile's platform-trust digest
    /// (`transport_host::endpoint_environment`) hashes these bytes in every
    /// build, so this is not a test-only accessor.
    pub(crate) fn to_der(&self) -> Result<Vec<u8>, ErrorStack> {
        self.0.to_der()
    }
}

/// The ciphers `SslConnector` offers.
const CIPHERS: &str = "DEFAULT:!aNULL:!eNULL:!MD5:!3DES:!DES:!RC4:!IDEA:!SEED:!aDSS:!SRP:!PSK";

/// One endpoint's TLS configuration.
#[derive(Clone)]
pub(crate) struct Connector {
    context: SslContext,
}

impl Connector {
    /// The configuration that trusts `platform`'s roots and `roots` besides.
    /// With no loader (`platform` is `None`) it trusts `roots` alone: the
    /// store is empty, never OpenSSL's default verify paths.
    pub(crate) fn build(
        platform: Option<&RootsLoader>,
        roots: &[Certificate],
    ) -> Result<Self, io::ErrorKind> {
        Self::context(platform, roots)
            .map(|context| Self { context })
            .map_err(|_| io::ErrorKind::InvalidInput)
    }

    fn context(
        platform: Option<&RootsLoader>,
        roots: &[Certificate],
    ) -> Result<SslContext, ErrorStack> {
        let mut context = base()?;
        let mut store = X509StoreBuilder::new()?;
        fill(&mut store, platform, roots);
        context.set_cert_store(store.build());
        Ok(context.build())
    }

    /// The client handshake for `host` over `io`.
    pub(crate) async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        host: &str,
        io: S,
    ) -> Result<Stream<S>, Refused> {
        let ssl = self.session(host).map_err(|_| Refused)?;
        Stream::connect(ssl, io).await
    }

    /// A session for `host`: its server name unless it is an address, and the
    /// name or address its certificate must be for.
    fn session(&self, host: &str) -> Result<Ssl, ErrorStack> {
        let mut ssl = Ssl::new(&self.context)?;
        let address = host.parse::<IpAddr>();
        if address.is_err() {
            ssl.set_hostname(host)?;
        }
        let parameters = ssl.param_mut();
        parameters.set_hostflags(X509CheckFlags::NO_PARTIAL_WILDCARDS);
        match address {
            Ok(address) => parameters.set_ip(address),
            Err(_) => parameters.set_host(host),
        }?;
        Ok(ssl)
    }
}

/// A context as the `openssl` crate's `SslConnector` makes it, bar its trust
/// roots: its private `ctx`, and the rest of `SslConnector::builder`, without
/// the `set_default_verify_paths` that builder ends its roots with, and with
/// the protocol bounds `native-tls` 0.2.18 applies by default: a floor of TLS
/// 1.2 (`TlsConnectorBuilder`'s `min_protocol`) and no ceiling. The floor is
/// set, not left to the host: an explicit minimum overrides a distribution's
/// `MinProtocol`, so leaving it unset would let a policy lower than 1.2 apply,
/// and clearing it would lower it to the library's.
fn base() -> Result<SslContextBuilder, ErrorStack> {
    let mut context = SslContextBuilder::new(SslMethod::tls())?;
    let mut options = SslOptions::ALL
        | SslOptions::NO_COMPRESSION
        | SslOptions::NO_SSLV2
        | SslOptions::NO_SSLV3
        | SslOptions::SINGLE_DH_USE
        | SslOptions::SINGLE_ECDH_USE;
    options &= !SslOptions::DONT_INSERT_EMPTY_FRAGMENTS;
    context.set_options(options);
    context.set_mode(
        SslMode::AUTO_RETRY
            | SslMode::ACCEPT_MOVING_WRITE_BUFFER
            | SslMode::ENABLE_PARTIAL_WRITE
            | SslMode::RELEASE_BUFFERS,
    );
    context.set_cipher_list(CIPHERS)?;
    context.set_verify(SslVerifyMode::PEER);
    context.set_min_proto_version(Some(SslVersion::TLS1_2))?;
    context.set_max_proto_version(None)?;
    Ok(context)
}

/// What a context's trust roots go into. The tests stand a recorder in place
/// of the store, to show what was loaded.
trait Store {
    /// Adds `root`. A root the store already holds is not an error.
    fn add(&mut self, root: Certificate);
}
impl Store for X509StoreBuilder {
    fn add(&mut self, root: Certificate) {
        let _ = self.add_cert(root.0);
    }
}

/// Puts into `store` the trust of a connector: `platform`'s roots, loaded once
/// (none without a loader), and `roots` besides. OpenSSL's default verify paths
/// are never read.
fn fill(store: &mut impl Store, platform: Option<&RootsLoader>, roots: &[Certificate]) {
    for root in platform.map(|platform| platform()).unwrap_or_default() {
        store.add(root);
    }
    for root in roots {
        store.add(root.clone());
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
