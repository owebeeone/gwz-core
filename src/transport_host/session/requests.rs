use super::*;

impl Session {
    pub(in crate::transport_host) fn begin(&self, request: &str) -> ModelResult<()> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .owner
            .as_ref()
            .ok_or_else(|| unavailable("no initiator"))?
            .begin(request)
            .map_err(mux_error)
    }
    /// The request's `--max-retries`: its driver's opens wait out that many
    /// retried setups, and its endpoint's retry machines allow them.
    pub(in crate::transport_host) fn set_max_retries(&self, request: &str, max_retries: u32) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.registrations.get_mut(request) {
            record.max_retries = max_retries;
        }
        if let Some(engine) = &mut state.engine {
            engine.set_max_retries(request, max_retries);
        }
        if let Some(endpoint) = &mut state.https {
            endpoint.set_max_retries(request, max_retries);
        }
    }
    /// The request's `--max-retries`, as its admission installed it.
    pub(in crate::transport_host) fn max_retries(&self, request: &str) -> u32 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .registrations
            .get(request)
            .map_or(setup_retry::DEFAULT_MAX_RETRIES, |record| {
                record.max_retries
            })
    }
    pub(in crate::transport_host) async fn ready(&self) -> ModelResult<()> {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .owner
            .clone()
            .ok_or_else(|| unavailable("no binding"))?;
        owner.ready().await.map_err(mux_error)
    }
    pub(in crate::transport_host) fn cancel(&self, request: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(owner) = &state.owner {
            let _ = owner.cancel(request);
        }
        if let Some(record) = state.registrations.get_mut(request) {
            record.sealed.get_or_insert(Instant::now());
        }
        if let Some(engine) = &mut state.engine {
            engine.cancel_request(request);
        }
        if let Some(engine) = &mut state.https {
            engine.cancel_request(request);
        }
        if state
            .incoming
            .as_ref()
            .is_some_and(|item| item.0 == request)
        {
            state.incoming = None;
        }
        for entry in state.streams.values().filter(|e| e.request == request) {
            // A not-yet-open stream may be retired before its endpoint reply
            // arrives. Complete its independent blocking waiter before retiring
            // the stream so cancellation never relies on a peer acknowledgment.
            entry.reply.complete(Err(Failure {
                setup_cause: None,
                code: gwz_transport::protocol::ErrorCode::Cancelled,
                effect: entry.opening_cancel_effect,
                facts: None,
            }));
            entry.stream.cancel();
        }
        for check in state.checks.values().filter(|c| c.request == request) {
            check.result.complete(Err(protocol_failure(
                gwz_transport::protocol::ErrorCode::Cancelled,
            )));
        }
        drop(state);
        self.event.signal();
    }
    pub(in crate::transport_host) fn seal(&self, request: &str) {
        self.cancel(request);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let closed = state.closed
            || state
                .owner
                .as_ref()
                .is_some_and(|o| o.phase() == Phase::Closed);
        let pending = state
            .engine
            .as_ref()
            .map_or(0, |e| e.pending_request_count(request))
            + state
                .https
                .as_ref()
                .map_or(0, |e| e.pending_request_count(request));
        if let Some(record) = state.registrations.get_mut(request) {
            record.sealed.get_or_insert(Instant::now());
            if closed {
                record.result.get_or_insert(CleanupReport {
                    pending_local_work: pending,
                    peer_cleanup_confirmed: false,
                });
            }
        }
    }
    pub(in crate::transport_host) async fn finish(&self, request: &str) -> CleanupReport {
        self.seal(request);
        let listener = self.listener();
        poll_fn(|cx| {
            if !listener.arm(cx) {
                self.close();
                return Poll::Ready(self.report());
            }
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            match state.registrations.get(request) {
                Some(r) if r.result.is_some() => Poll::Ready(r.result.clone().unwrap()),
                None => Poll::Ready(CleanupReport::default()),
                _ => Poll::Pending,
            }
        })
        .await
    }
}
