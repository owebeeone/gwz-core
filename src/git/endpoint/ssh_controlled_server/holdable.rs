//! The connection's socket, with a switch that stops it delivering what the client sends, and a gate that lets it start
//! again. Writes always pass.
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
    sync::watch,
};

pub(super) struct Holdable {
    socket: TcpStream,
    hold: Arc<AtomicBool>,
    /// When there is one, a hold lasts only until this gate opens. It never closes again, so a hold set after it
    /// opened is no hold, and nothing races the two.
    release: Option<watch::Receiver<bool>>,
}

impl Holdable {
    pub(super) fn new(
        socket: TcpStream,
        hold: Arc<AtomicBool>,
        release: Option<watch::Receiver<bool>>,
    ) -> Self {
        Self {
            socket,
            hold,
            release,
        }
    }

    fn held(&self) -> bool {
        self.hold.load(Ordering::Acquire)
            && !self.release.as_ref().is_some_and(|gate| *gate.borrow())
    }
}

impl AsyncRead for Holdable {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // The switch is set from the session loop between two reads, so no read is in flight when it flips; a
        // held read stays pending until the gate opens (for good without one), and the session task polls it again
        // whenever anything else wakes it: the task that opens the gate's channel does, by what it then sends.
        if self.held() {
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
