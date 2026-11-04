//! An endpoint adapter that counts the connections the host has read as ended for an idle loss or a failed proof, for
//! the rows that must show a connection was never retired (finding F19). Everything else goes to the adapter it wraps.
use gwz_connection::{
    AuthContext, Completion, ConnectCommand, EndpointAdapter, ExchangeIo, ExchangeSpec, ProofId,
    ReuseCommand,
};
use gwz_transport::pool::ConnectionId;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

/// `inner`, counting each `IdleClosed` the host reads from it into `idle_closed`.
pub(super) struct Counting<A> {
    pub(super) inner: A,
    pub(super) idle_closed: Arc<AtomicUsize>,
}
impl<A: EndpointAdapter> EndpointAdapter for Counting<A> {
    fn connect(&mut self, command: ConnectCommand) {
        self.inner.connect(command);
    }
    fn cancel_connect(&mut self, connection: ConnectionId) {
        self.inner.cancel_connect(connection);
    }
    fn close(&mut self, connection: ConnectionId) {
        self.inner.close(connection);
    }
    fn start_exchange(
        &mut self,
        connection: ConnectionId,
        spec: &ExchangeSpec,
        auth: &AuthContext,
    ) -> io::Result<Box<dyn ExchangeIo>> {
        self.inner.start_exchange(connection, spec, auth)
    }
    fn authorize_reuse(&mut self, command: ReuseCommand) -> bool {
        self.inner.authorize_reuse(command)
    }
    fn cancel_reuse(&mut self, proof: ProofId) {
        self.inner.cancel_reuse(proof);
    }
    fn poll_completion(&mut self, cx: &mut Context<'_>) -> Poll<Completion> {
        let polled = self.inner.poll_completion(cx);
        if matches!(polled, Poll::Ready(Completion::IdleClosed { .. })) {
            self.idle_closed.fetch_add(1, Ordering::SeqCst);
        }
        polled
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.inner.poll_dispose(cx)
    }
}
