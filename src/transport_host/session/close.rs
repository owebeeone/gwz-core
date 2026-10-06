use super::*;

impl Session {
    pub(in crate::transport_host) fn is_closed(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed
            || state
                .owner
                .as_ref()
                .is_some_and(|o| o.phase() == Phase::Closed)
    }

    pub(in crate::transport_host) fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::close_state(&mut state);
        drop(state);
        self.event.signal();
        // The placement pass that retires what the close left must not wait
        // out its park.
        self.wake();
    }
    pub(super) fn close_state(state: &mut State) {
        state.closed = true;
        if let Some(port) = &state.port {
            port.disconnect();
        }
        if let Some(engine) = &mut state.engine {
            engine.shutdown();
        }
        if let Some(engine) = &mut state.https {
            engine.shutdown();
        }
        for (_, entry) in std::mem::take(&mut state.streams) {
            entry.peer.disconnect();
            entry.reply.complete(Err(protocol_failure(
                gwz_transport::protocol::ErrorCode::CarrierLost,
            )));
        }
        for (_, check) in std::mem::take(&mut state.checks) {
            check.result.complete(Err(protocol_failure(
                gwz_transport::protocol::ErrorCode::CarrierLost,
            )));
        }
        state.pending = None;
        state.incoming = None;
    }
    pub(super) fn report(&self) -> CleanupReport {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        CleanupReport {
            pending_local_work: state.streams.len()
                + state.checks.len()
                + state.engine.as_ref().map_or(0, |e| e.pending())
                + state.https.as_ref().map_or(0, |e| e.pending()),
            peer_cleanup_confirmed: false,
        }
    }
    pub(in crate::transport_host) async fn cleanup(&self) -> CleanupReport {
        let deadline = Instant::now() + CLEANUP;
        let listener = self.listener();
        poll_fn(|cx| {
            let armed = listener.arm(cx);
            let report = self.report();
            if !armed || report.pending_local_work == 0 || Instant::now() >= deadline {
                Poll::Ready(report)
            } else {
                Poll::Pending
            }
        })
        .await
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        Self::close_state(self.state.get_mut().unwrap_or_else(|e| e.into_inner()));
    }
}
