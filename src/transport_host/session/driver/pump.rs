use super::*;

impl Session {
    /// One pass. `waker` wakes the thread that runs passes; the result says
    /// whether the pass moved a message, so that another pass runs at once.
    pub(in crate::transport_host::session) fn drive(&self, waker: &Waker) -> bool {
        let mut moved = false;
        let mut reports: Vec<Box<dyn FnOnce() -> bool + Send>> = Vec::new();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64;
        let mut cx = Context::from_waker(waker);
        if let Some(owner) = &state.owner {
            owner.advance(now);
        }
        if state
            .owner
            .as_ref()
            .is_some_and(|o| o.phase() == Phase::Closed)
            && !state.closed
        {
            Self::close_state(&mut state);
        }
        if let Some(engine) = &mut state.engine {
            if engine.step(now, &mut cx).is_err() {
                Self::close_state(&mut state);
            }
        }
        if let Some(engine) = &mut state.https {
            if engine.step(now, &mut cx).is_err() {
                Self::close_state(&mut state);
            }
        }
        if !state.closed {
            if let Some(owner) = state.owner.clone() {
                let held = self.test_hooks.should_hold_pump();
                for _ in 0..64 {
                    if held {
                        break;
                    }
                    let item = if let Some(item) = state.incoming.take() {
                        item
                    } else {
                        match pin!(owner.next_action()).poll(&mut cx) {
                            Poll::Ready(Ok(Some(item))) => item,
                            Poll::Ready(Ok(None)) | Poll::Pending => break,
                            Poll::Ready(Err(_)) => {
                                Self::close_state(&mut state);
                                break;
                            }
                        }
                    };
                    let (request, message) = item;
                    let https = message
                        .open
                        .as_ref()
                        .is_some_and(|o| o.destination.scheme == Scheme::Https)
                        || state
                            .https
                            .as_ref()
                            .is_some_and(|e| e.owns(&request, message.stream_id));
                    if state.endpoint_config.is_some() {
                        let result = if https {
                            state
                                .https
                                .as_mut()
                                .ok_or(EndpointError::InvalidRequest)
                                .and_then(|engine| engine.accept(request.clone(), message.clone()))
                        } else {
                            state
                                .engine
                                .as_mut()
                                .ok_or(EndpointError::InvalidRequest)
                                .and_then(|engine| engine.accept(request.clone(), message.clone()))
                        };
                        match result {
                            Ok(()) => moved = true,
                            Err(EndpointError::WouldBlock) => {
                                state.incoming = Some((request, message));
                                break;
                            }
                            Err(_) => {
                                Self::close_state(&mut state);
                                break;
                            }
                        }
                        continue;
                    }
                    moved = true;
                    if let Some(check) = state.checks.remove(&message.stream_id) {
                        let result = match message.kind {
                            MessageKind::IdentityChecked => Ok(()),
                            MessageKind::IdentityCheckFailed => Err(message
                                .identity_check_failed
                                .expect("validated check failure")),
                            _ => Err(protocol_failure(
                                gwz_transport::protocol::ErrorCode::Cancelled,
                            )),
                        };
                        check.result.complete(result);
                        continue;
                    }
                    if let Some(entry) = state.streams.get_mut(&message.stream_id) {
                        if let Some(opened) = message.opened.clone() {
                            entry.opened = true;
                            let observe = entry.observe.clone();
                            let value = opened.clone();
                            let reply = entry.reply.clone();
                            let stream = entry.stream.clone();
                            reports.push(Box::new(move || {
                                reply.complete_with(
                                    Ok((BlockingStream::new(stream), opened)),
                                    || {
                                        observe(message.stream_id, &value);
                                    },
                                );
                                true
                            }));
                        } else if let Some(failure) = message.open_failed.clone() {
                            let report = entry.facts.clone();
                            let report_open_failure = entry.report_open_failure;
                            let reply = entry.reply.clone();
                            let peer = entry.peer.clone();
                            reports.push(Box::new(move || {
                                if report_open_failure {
                                    if let Some(facts) = &failure.facts {
                                        report(facts);
                                    }
                                }
                                reply.complete(Err(failure));
                                peer.disconnect();
                                true
                            }));
                        } else {
                            let facts = message
                                .failed
                                .as_ref()
                                .and_then(|f| f.facts.clone())
                                .or_else(|| message.closed.as_ref().map(|c| c.facts.clone()));
                            let report = entry.facts.clone();
                            let peer = entry.peer.clone();
                            reports.push(Box::new(move || {
                                if let Some(facts) = facts {
                                    report(&facts);
                                }
                                peer.deliver(message).is_ok()
                            }));
                        }
                    }
                }
                // Hand the mux every message the endpoints have ready, up to a
                // bound per pass. Alternate schemes; a busy SSH stream cannot
                // starve HTTPS.
                for _ in 0..MAX_HANDOFFS {
                    if state.pending.is_none() {
                        let first = if state.prefer_https {
                            state.https.as_mut().and_then(|e| e.take_outbound(&mut cx))
                        } else {
                            state.engine.as_mut().and_then(|e| e.take_outbound())
                        };
                        let item = first.or_else(|| {
                            if state.prefer_https {
                                state.engine.as_mut().and_then(|e| e.take_outbound())
                            } else {
                                state.https.as_mut().and_then(|e| e.take_outbound(&mut cx))
                            }
                        });
                        state.pending = item.map(|out| (out.request, out.envelope));
                        state.prefer_https = !state.prefer_https;
                    }
                    let Some(mut item) = state.pending.take() else {
                        break;
                    };
                    // Local seal makes the mux own the cancellation terminal. Late
                    // physical completion is cleanup only and cannot replace it.
                    let sealed = state
                        .registrations
                        .get(&item.0)
                        .is_some_and(|r| r.sealed.is_some());
                    if sealed {
                        moved = true;
                        continue;
                    }
                    if let Some(engine) = &mut state.https {
                        engine.before_handoff(&item.0, &mut item.1);
                    }
                    match owner.send(&item.0, &item.1) {
                        Ok(()) => {
                            moved = true;
                            if let Some(engine) = &mut state.https {
                                engine.handed_off(&item.0, &item.1);
                            }
                        }
                        Err(mux::Error::WouldBlock) => {
                            state.pending = Some(item);
                            break;
                        }
                        Err(mux::Error::InvalidRequest)
                            if matches!(
                                item.1.kind,
                                MessageKind::OpenFailed
                                    | MessageKind::IdentityCheckFailed
                                    | MessageKind::Failed
                                    | MessageKind::Closed
                            ) && state.session.as_deref() == Some(&item.1.session_id) =>
                        {
                            // Mux deadline can win against the endpoint worker's
                            // terminal completion for this same admitted stream.
                            moved = true;
                        }
                        Err(_) => {
                            Self::close_state(&mut state);
                            break;
                        }
                    }
                }
                let sealed: BTreeSet<String> = state
                    .registrations
                    .iter()
                    .filter(|(_, r)| r.sealed.is_some())
                    .map(|(id, _)| id.clone())
                    .collect();
                let session = state.session.clone();
                let mut failed = false;
                'streams: for entry in state.streams.values_mut().filter(|_| !held) {
                    entry.peer.advance(now);
                    if !entry.opened {
                        if entry.deadline.is_some_and(|at| Instant::now() >= at) {
                            entry.reply.complete(Err(protocol_failure(
                                gwz_transport::protocol::ErrorCode::Timeout,
                            )));
                            let _ = owner.cancel(&entry.request);
                            entry.peer.disconnect();
                        }
                        continue;
                    }
                    // Forward what the stream has ready, up to a bound per pass.
                    // A wait for its next message stays pending, so the
                    // client's next read, write or close wakes this thread.
                    for _ in 0..MAX_STREAM_MESSAGES {
                        if entry.pending.is_none() {
                            let next = entry.next.get_or_insert_with(|| next_message(&entry.peer));
                            if let Poll::Ready(result) = next.as_mut().poll(&mut cx) {
                                entry.next = None;
                                entry.pending = result.ok().flatten();
                            }
                        }
                        let Some(message) = entry.pending.take() else {
                            break;
                        };
                        // As above: the seal made the mux own this request's
                        // cancellation, so a late client message for one of its
                        // streams is stale. The mux may already have retired
                        // the route; forwarding it would fail the whole session.
                        if sealed.contains(&entry.request) {
                            moved = true;
                            continue;
                        }
                        match owner.send(&entry.request, &message) {
                            Ok(()) => moved = true,
                            Err(mux::Error::WouldBlock) => {
                                entry.pending = Some(message);
                                break;
                            }
                            Err(mux::Error::InvalidRequest)
                                if session.as_deref() == Some(&message.session_id)
                                    && message.version == 2 =>
                            {
                                // The mux retires a stream's route the moment it
                                // admits that stream's terminal (the endpoint's
                                // Closed or Failed, or its own deadline's) and
                                // queues the terminal for this pump, which hands it
                                // to the stream only after this pass. A client
                                // message taken meanwhile is stale: drop it. `send`
                                // answers InvalidRequest for a missing route, or for
                                // a route, session or version that doesn't match;
                                // the guard rules out the last two, and the route
                                // the mux gave this stream belongs to this entry's
                                // request, so here it means the route is gone.
                                moved = true;
                            }
                            Err(_) => {
                                failed = true;
                                break 'streams;
                            }
                        }
                    }
                }
                if failed {
                    Self::close_state(&mut state);
                }
                state.streams.retain(|_, entry| {
                    held || !entry.peer.stats().terminal || entry.pending.is_some()
                });
                if !held {
                    self.test_hooks.pumped();
                }
            }
        }
        let sealed: Vec<_> = state
            .registrations
            .iter()
            .filter_map(|(id, r)| r.sealed.map(|at| (id.clone(), at)))
            .collect();
        for (id, at) in sealed {
            let pending = state
                .engine
                .as_ref()
                .is_some_and(|e| e.pending_request(&id))
                || state
                    .https
                    .as_ref()
                    .is_some_and(|e| e.pending_request_count(&id) > 0)
                || state.streams.values().any(|e| e.request == id)
                || state.checks.values().any(|e| e.request == id)
                || state.pending.as_ref().is_some_and(|p| p.0 == id)
                || state.incoming.as_ref().is_some_and(|p| p.0 == id);
            let retired = state.registrations[&id].mux_retired
                || state
                    .owner
                    .as_ref()
                    .is_none_or(|o| o.finish(&id).is_ok() || o.phase() == Phase::Closed);
            if retired {
                // finish removes the mux request. Repeating it would return
                // InvalidRequest and erase proof that retirement completed.
                state.registrations.get_mut(&id).unwrap().mux_retired = true;
            }
            if (!pending && retired) || at.elapsed() >= CLEANUP || state.closed {
                if at.elapsed() >= CLEANUP && !retired {
                    Self::close_state(&mut state);
                }
                let count = state
                    .engine
                    .as_ref()
                    .map_or(0, |e| e.pending_request_count(&id))
                    + state
                        .https
                        .as_ref()
                        .map_or(0, |e| e.pending_request_count(&id));
                if let Some(record) = state.registrations.get_mut(&id) {
                    record.result.get_or_insert(CleanupReport {
                        pending_local_work: count,
                        peer_cleanup_confirmed: false,
                    });
                }
            }
        }
        drop(state);
        moved |= !reports.is_empty();
        for report in reports {
            if !report() {
                self.close();
            }
        }
        moved
    }
}
