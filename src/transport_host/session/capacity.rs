use super::*;

fn has_non_idle_lease(state: &State) -> bool {
    let non_idle = |counts: pool::Counts| counts.total() != counts.idle;
    state
        .engine
        .as_ref()
        .is_some_and(|engine| non_idle(engine.pool().counts()))
        || state
            .https
            .as_ref()
            .is_some_and(|endpoint| non_idle(endpoint.pool().counts()))
}
struct CapacityMutation<'a> {
    session: &'a Session,
    armed: bool,
}
impl Drop for CapacityMutation<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.session.close();
        }
    }
}
struct AdmissionLeader<'a>(&'a Session);
impl Drop for AdmissionLeader<'_> {
    fn drop(&mut self) {
        self.0.admission_gate.store(false, Ordering::Release);
        self.0.event.signal();
    }
}
struct CapacityLeader<'a>(&'a Session);
impl Drop for CapacityLeader<'_> {
    fn drop(&mut self) {
        self.0.capacity_gate.store(false, Ordering::Release);
        self.0.event.signal();
    }
}
impl Session {
    pub(in crate::transport_host) async fn admit_client_request(
        self: &Arc<Self>,
        request: &str,
        capacity: pool::Capacity,
    ) -> ModelResult<ClientRequest> {
        // A differing physical policy is refused before consuming the mux's
        // lifetime request ID. Serialize that check with local admissions.
        let deadline = Instant::now() + CLEANUP;
        let listener = self.listener();
        let _admission = poll_fn(|cx| {
            if !listener.arm(cx) {
                return Poll::Ready(Err(unavailable("transport capacity wait unavailable")));
            }
            if Instant::now() >= deadline {
                return Poll::Ready(Err(unavailable("transport capacity wait timed out")));
            }
            if self
                .admission_gate
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                Poll::Ready(Ok(AdmissionLeader(self)))
            } else {
                Poll::Pending
            }
        })
        .await?;
        {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(unavailable("transport session is closed"));
            }
            if !request::identifier(request)
                || state.used.contains(request)
                || state.used.len() >= 256
            {
                return Err(invalid("invalid or exhausted request registration"));
            }
            if state.installed_capacity != Some(capacity)
                && (state
                    .registrations
                    .values()
                    .any(|record| record.result.is_none())
                    || state.engine.as_ref().is_some_and(|engine| {
                        engine.pending().saturating_sub(engine.pool().counts().idle) != 0
                    })
                    || state
                        .https
                        .as_ref()
                        .is_some_and(|endpoint| endpoint.pending() != 0)
                    || has_non_idle_lease(&state))
            {
                return Err(ModelError::new(
                    crate::model::ErrorCode::TransportCapacityConflict,
                    "transport physical capacity conflicts with live work",
                ));
            }
        }
        self.install_capacity(capacity, deadline).await?;
        if Instant::now() >= deadline {
            return Err(unavailable("transport capacity wait timed out"));
        }
        ClientRequest::new(self.clone(), request)
    }

    pub(in crate::transport_host) async fn install_capacity(
        &self,
        capacity: pool::Capacity,
        deadline: Instant,
    ) -> ModelResult<()> {
        // One physical-policy leader owns both pools and the shared authority
        // through retirement. Followers wait without holding the state mutex.
        let gate_listener = self.listener();
        let _leader = poll_fn(|cx| {
            if !gate_listener.arm(cx) {
                return Poll::Ready(Err(unavailable("transport capacity wait unavailable")));
            }
            if Instant::now() >= deadline {
                return Poll::Ready(Err(unavailable("transport capacity wait timed out")));
            }
            if self
                .capacity_gate
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Poll::Ready(Ok(CapacityLeader(self)));
            }
            Poll::Pending
        })
        .await?;
        // A finished request can still have bounded physical cleanup behind
        // its terminal reply. Wait for that owner to retire before installing
        // the next operation's limits; live overlapping requests still fail.
        let listener = self.listener();
        let reused = poll_fn(|cx| {
            if !listener.arm(cx) {
                return Poll::Ready(Err(unavailable("transport capacity wait unavailable")));
            }
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Poll::Ready(Err(unavailable("transport operation is active")));
            }
            // Identical physical policy shares the installed pools. In
            // particular, a live lease must not trigger a reinstall.
            if state.installed_capacity == Some(capacity) {
                return Poll::Ready(Ok(true));
            }
            if state
                .registrations
                .values()
                .any(|record| record.result.is_none())
            {
                return Poll::Ready(Err(unavailable("transport operation is active")));
            }
            // The SSH worker's pending count includes healthy idle sockets.
            // Those sockets are the resource this operation is meant to reuse;
            // waiting for them to disappear makes a sequential request time out.
            let pending = state.engine.as_ref().is_some_and(|engine| {
                engine.pending().saturating_sub(engine.pool().counts().idle) != 0
            }) || state
                .https
                .as_ref()
                .is_some_and(|endpoint| endpoint.pending() != 0);
            if !pending {
                return Poll::Ready(Ok(false));
            }
            if Instant::now() >= deadline {
                return Poll::Ready(Err(unavailable("prior transport cleanup incomplete")));
            }
            Poll::Pending
        })
        .await?;
        if reused {
            return Ok(());
        }
        let mut mutation = CapacityMutation {
            session: self,
            armed: false,
        };
        let (ssh, https, authority) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.installed_capacity == Some(capacity) {
                return Ok(());
            }
            if state.closed
                || state.engine.as_ref().is_some_and(|engine| {
                    engine.pending().saturating_sub(engine.pool().counts().idle) != 0
                })
                || state
                    .https
                    .as_ref()
                    .is_some_and(|endpoint| endpoint.pending() != 0)
            {
                return Err(unavailable("transport operation is active"));
            }
            if state
                .registrations
                .values()
                .any(|record| record.result.is_none())
            {
                return Err(unavailable("transport operation is active"));
            }
            if Instant::now() >= deadline {
                return Err(unavailable("transport capacity wait timed out"));
            }
            if has_non_idle_lease(&state) {
                return Err(unavailable("transport capacity is active"));
            }
            let ssh = state
                .engine
                .as_ref()
                .ok_or_else(|| unavailable("SSH endpoint unavailable"))?;
            let https = state.https.as_ref().map(|endpoint| endpoint.pool().clone());
            mutation.armed = true;
            if let Some(https) = &https {
                ssh.pool()
                    .install_capacity_pair(https, capacity)
                    .map_err(|_| unavailable("transport capacity is active"))?;
            } else {
                ssh.pool()
                    .install_capacity(capacity)
                    .map_err(|_| unavailable("transport capacity is active"))?;
            }
            ssh.set_request_capacity(capacity.max_requests);
            let ssh_pool = ssh.pool().clone();
            state.installed_capacity = None;
            (
                ssh_pool,
                https,
                state
                    .authority
                    .clone()
                    .ok_or_else(|| unavailable("shared capacity unavailable"))?,
            )
        };
        let listener = self.listener();
        let retired = poll_fn(|cx| {
            if !listener.arm(cx) {
                return Poll::Ready(Err(unavailable("transport capacity wait unavailable")));
            }
            if self.test_hooks.should_hold_retirement() {
                return Poll::Pending;
            }
            if ssh.counts().closing == 0
                && https.as_ref().is_none_or(|pool| pool.counts().closing == 0)
            {
                return Poll::Ready(Ok(()));
            }
            if Instant::now() >= deadline || self.is_closed() {
                return Poll::Ready(Err(unavailable("transport capacity retirement incomplete")));
            }
            Poll::Pending
        })
        .await;
        if let Err(error) = retired {
            // The pools have already accepted the new policy. A failure to
            // finish retirement cannot leave an unpaired usable endpoint.
            self.close();
            return Err(error);
        }
        if !authority.install_capacity(capacity.total, capacity.per_host) {
            self.close();
            return Err(unavailable("shared capacity remains occupied"));
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .installed_capacity = Some(capacity);
        mutation.armed = false;
        Ok(())
    }
}
