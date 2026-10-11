//! The connection's socket, with a switch that stops it delivering what the client sends. Writes always pass.
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};

pub(super) struct Holdable {
    socket: TcpStream,
    hold: Arc<AtomicBool>,
}

impl Holdable {
    pub(super) fn new(socket: TcpStream, hold: Arc<AtomicBool>) -> Self {
        Self { socket, hold }
    }
}

impl AsyncRead for Holdable {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // The switch is set from the session loop between two reads, so no read is in flight when it flips; a
        // held read stays pending for good, and the session task polls it again whenever anything else wakes it.
        if self.hold.load(Ordering::Acquire) {
            return Poll::Pending;
        }
        Pin::new(&mut self.socket).poll_read(cx, buf)
    }
}

impl AsyncWrite for Holdable {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.socket).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}
