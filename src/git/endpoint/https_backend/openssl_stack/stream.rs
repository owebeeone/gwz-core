//! An async TLS stream over OpenSSL, without a pointer to the task's context.
//!
//! OpenSSL reads and writes through a [`Wire`], two buffers in memory: what
//! the socket has delivered and OpenSSL has not read, and what OpenSSL has
//! written and the socket has not taken. The stream moves bytes between them
//! and the socket, when OpenSSL asks to read (`WouldBlock` from an empty
//! `Wire`) and before it is asked for more to write. Nothing of the task's
//! context is stored, and nothing is unsafe.
use super::*;
use openssl::ssl::{ErrorCode, SslStream};
use std::{
    io::{Read, Write},
    pin::Pin,
    task::{Context, Poll, ready},
};
use tokio::io::ReadBuf;

/// The most ciphertext taken from the socket at a time (one record and its
/// framing).
const CHUNK: usize = 16 * 1024 + 512;

#[derive(Default)]
struct Wire {
    /// What the socket has delivered, from `taken` on.
    inbound: Vec<u8>,
    taken: usize,
    /// The socket has ended.
    ended: bool,
    /// What OpenSSL has written and the socket has not taken yet.
    outbound: Vec<u8>,
}

impl Read for Wire {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let available = &self.inbound[self.taken..];
        if available.is_empty() {
            self.inbound.clear();
            self.taken = 0;
            return if self.ended {
                Ok(0)
            } else {
                Err(io::ErrorKind::WouldBlock.into())
            };
        }
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        self.taken += count;
        Ok(count)
    }
}

impl Write for Wire {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.outbound.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A verified TLS stream over `S`.
pub(crate) struct Stream<S> {
    tls: SslStream<Wire>,
    io: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Stream<S> {
    /// Runs the client handshake of `ssl` over `io`.
    pub(super) async fn connect(ssl: Ssl, io: S) -> Result<Self, Refused> {
        let tls = SslStream::new(ssl, Wire::default()).map_err(|_| Refused)?;
        let mut stream = Self { tls, io };
        std::future::poll_fn(|cx| stream.poll_handshake(cx)).await?;
        Ok(stream)
    }

    fn poll_handshake(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Refused>> {
        loop {
            match self.tls.connect() {
                Ok(()) => {
                    return self.poll_send(cx).map(|sent| sent.map_err(|_| Refused));
                }
                Err(error)
                    if matches!(error.code(), ErrorCode::WANT_READ | ErrorCode::WANT_WRITE) =>
                {
                    ready!(self.poll_send(cx)).map_err(|_| Refused)?;
                    ready!(self.poll_fill(cx)).map_err(|_| Refused)?;
                }
                Err(_) => {
                    // The alert OpenSSL queued goes out if the socket takes it now.
                    let _ = self.poll_send(cx);
                    return Poll::Ready(Err(Refused));
                }
            }
        }
    }

    /// Hands the socket what OpenSSL has written.
    fn poll_send(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let Self { tls, io } = self;
        let wire = tls.get_mut();
        while !wire.outbound.is_empty() {
            let sent = ready!(Pin::new(&mut *io).poll_write(cx, &wire.outbound))?;
            if sent == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            wire.outbound.drain(..sent);
        }
        Poll::Ready(Ok(()))
    }

    /// Takes the next ciphertext the socket delivers, or its end.
    fn poll_fill(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let Self { tls, io } = self;
        let mut chunk = [0u8; CHUNK];
        let mut buffer = ReadBuf::new(&mut chunk);
        ready!(Pin::new(&mut *io).poll_read(cx, &mut buffer))?;
        let wire = tls.get_mut();
        if buffer.filled().is_empty() {
            wire.ended = true;
        } else {
            wire.inbound.extend_from_slice(buffer.filled());
        }
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for Stream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            match self.tls.read(buffer.initialize_unfilled()) {
                Ok(count) => {
                    buffer.advance(count);
                    return Poll::Ready(Ok(()));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    ready!(self.poll_send(cx))?;
                    ready!(self.poll_fill(cx))?;
                }
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for Stream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            // What OpenSSL has written goes first, so that it never holds more
            // than one record.
            ready!(self.poll_send(cx))?;
            match self.tls.write(bytes) {
                Ok(count) => {
                    // Sent now if the socket takes it; else by the next call.
                    if let Poll::Ready(Err(error)) = self.poll_send(cx) {
                        return Poll::Ready(Err(error));
                    }
                    return Poll::Ready(Ok(count));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    ready!(self.poll_fill(cx))?;
                }
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_send(cx))?;
        Pin::new(&mut self.io).poll_flush(cx)
    }

    /// Sends the close notification and nothing else, as `tokio-native-tls`
    /// does: the socket is closed when the stream is dropped.
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.tls.shutdown() {
            Ok(_) => {}
            Err(error) if error.code() == ErrorCode::ZERO_RETURN => {}
            Err(error) => {
                let error = error.into_io_error().unwrap_or_else(io::Error::other);
                if error.kind() != io::ErrorKind::WouldBlock {
                    return Poll::Ready(Err(error));
                }
            }
        }
        self.poll_send(cx)
    }
}

/// The `tls-server-end-point` channel binding (RFC 5929) of the server's
/// certificate: its hash by the digest of its signature, SHA-256 where that is
/// MD5 or SHA-1, as `native-tls` computes it.
pub(crate) fn tls_server_end_point<S>(stream: &Stream<S>) -> Option<Vec<u8>> {
    use openssl::{hash::MessageDigest, nid::Nid};
    let certificate = stream.tls.ssl().peer_certificate()?;
    let algorithm = certificate.signature_algorithm().object().nid();
    let digest = match algorithm.signature_algorithms()?.digest {
        Nid::MD5 | Nid::SHA1 => MessageDigest::sha256(),
        nid => MessageDigest::from_nid(nid)?,
    };
    certificate.digest(digest).ok().map(|bytes| bytes.to_vec())
}

cfg_if::cfg_if! { if #[cfg(test)] {
    mod tests {
        use super::*;

        #[test]
        fn the_wire_blocks_until_the_socket_delivers_and_ends_when_it_has_ended() {
            let mut wire = Wire::default();
            let mut buffer = [0u8; 4];
            assert_eq!(wire.read(&mut buffer).unwrap_err().kind(), io::ErrorKind::WouldBlock);
            wire.inbound.extend_from_slice(b"abcdef");
            assert_eq!(wire.read(&mut buffer).unwrap(), 4);
            assert_eq!(&buffer, b"abcd");
            assert_eq!(wire.read(&mut buffer).unwrap(), 2);
            assert_eq!(
                wire.read(&mut buffer).unwrap_err().kind(),
                io::ErrorKind::WouldBlock,
                "emptied, it waits again"
            );
            wire.ended = true;
            assert_eq!(wire.read(&mut buffer).unwrap(), 0);
            assert_eq!(wire.write(b"xy").unwrap(), 2);
            assert_eq!(wire.outbound, b"xy");
        }
    }
} }
