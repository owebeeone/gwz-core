//! The TLS handshake of an HTTPS connect, and what its failure was. The peer
//! ending or breaking the connection under it is `Io`, which a setup retries
//! (gwz-core dev-docs/GwzRemoteTransportRetryPlan.md §4), as a server that
//! drops a handshake it has no room for does; every other failure, a
//! certificate or the protocol, is `Trust`, which closes the key.
use super::https_connection::failure;
use gwz_transport::protocol::{ErrorCode, Failure};
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_native_tls::{TlsConnector, TlsStream};

/// Runs the handshake for `host` over `io`.
pub(crate) async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    connector: &TlsConnector,
    host: &str,
    io: S,
) -> Result<TlsStream<Watched<S>>, Failure> {
    let broken = Arc::new(AtomicBool::new(false));
    let watched = Watched {
        io,
        broken: broken.clone(),
    };
    connector.connect(host, watched).await.map_err(|_| {
        failure(if broken.load(Ordering::Acquire) {
            ErrorCode::Io
        } else {
            ErrorCode::Trust
        })
    })
}

/// The connection under a handshake, which records whether the peer ended
/// it or it broke: a read at its end, or a read or a write that failed.
pub(crate) struct Watched<S> {
    io: S,
    broken: Arc<AtomicBool>,
}
impl<S> Watched<S> {
    fn mark(&self) {
        self.broken.store(true, Ordering::Release);
    }
}
impl<S: AsyncRead + Unpin> AsyncRead for Watched<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buffer.filled().len();
        let wanted = buffer.remaining() > 0;
        let result = Pin::new(&mut self.io).poll_read(cx, buffer);
        let ended =
            matches!(result, Poll::Ready(Ok(()))) && wanted && buffer.filled().len() == before;
        if ended || matches!(result, Poll::Ready(Err(_))) {
            self.mark();
        }
        result
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for Watched<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.io).poll_write(cx, bytes);
        let refused = matches!(result, Poll::Ready(Ok(0))) && !bytes.is_empty();
        if refused || matches!(result, Poll::Ready(Err(_))) {
            self.mark();
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(&mut self.io).poll_flush(cx);
        if matches!(result, Poll::Ready(Err(_))) {
            self.mark();
        }
        result
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}
