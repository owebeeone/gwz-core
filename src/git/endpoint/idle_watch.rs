//! Readiness for idle SSH sessions (dev-docs/GwzTransportIdleLossDesign.md
//! §5.1). libssh2 reads a session's socket only during an exchange, so an idle
//! session's EOF or reset reaches nobody. A reactor thread owned by the
//! connector watches a duplicate of each idle socket and wakes the host.
use std::{
    io,
    mem::MaybeUninit,
    net::TcpStream,
    sync::Arc,
    task::{Context, Poll},
    thread::{self, JoinHandle},
};
use tokio::{io::ReadBuf, runtime::Handle, sync::oneshot};

/// A thread driving only a current-thread runtime's I/O driver, until the last
/// owner drops it. The connector and every watch it made own it, so it
/// outlives each registration.
pub(crate) struct IdleReactor {
    handle: Handle,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl IdleReactor {
    pub(crate) fn start() -> io::Result<Arc<Self>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()?;
        let handle = runtime.handle().clone();
        let (stop, stopped) = oneshot::channel::<()>();
        let thread = thread::Builder::new()
            .name("gwz-ssh-idle-watch".into())
            .spawn(move || {
                // Either a stop or the sender's drop ends the wait.
                let _ = runtime.block_on(stopped);
            })?;
        Ok(Arc::new(Self {
            handle,
            stop: Some(stop),
            thread: Some(thread),
        }))
    }
}

impl Drop for IdleReactor {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One idle socket's watch: a duplicate of the session's socket, registered
/// with the reactor. Dropping it deregisters and closes only the duplicate.
pub(crate) struct IdleSocket {
    // Deregistered before the reactor can be dropped.
    stream: tokio::net::TcpStream,
    _reactor: Arc<IdleReactor>,
}

impl IdleSocket {
    pub(crate) fn watch(reactor: &Arc<IdleReactor>, socket: TcpStream) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        let _entered = reactor.handle.enter();
        let stream = tokio::net::TcpStream::from_std(socket)?;
        Ok(Self {
            stream,
            _reactor: reactor.clone(),
        })
    }

    /// Ready once the peer has closed or reset the connection, or has sent
    /// bytes, which no one reads while the session is idle. Pending registers
    /// `cx` with the reactor. A peek: nothing is consumed.
    pub(crate) fn poll_lost(&self, cx: &mut Context<'_>) -> Poll<()> {
        let mut byte = [MaybeUninit::uninit()];
        let mut buffer = ReadBuf::uninit(&mut byte);
        match self.stream.poll_peek(cx, &mut buffer) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(_) => Poll::Ready(()),
        }
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
