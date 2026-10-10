use super::*;

impl HttpsEndpoint {
    pub(in crate::transport_host) fn step(
        &mut self,
        now: u64,
        cx: &mut Context<'_>,
    ) -> Result<(), EndpointError> {
        self.now_ms = self.now_ms.max(now);
        self.watch.register(cx.waker());
        for operation in self.operations.values_mut() {
            for retry in operation.retries.values_mut() {
                if retry
                    .challenge
                    .as_ref()
                    .is_some_and(ChallengeLease::expired)
                {
                    retry.challenge = None;
                }
            }
        }
        for ((request, _), entry) in &mut self.entries {
            if let Some(task) = entry.preparing.as_mut() {
                if let Poll::Ready(result) = Pin::new(task).poll(cx) {
                    entry.preparing = None;
                    let (mut result, connect, mut retry) =
                        result.map_err(|_| EndpointError::Protocol)?;
                    entry.publication_deadline = retry.budget.publication_deadline();
                    if let Ok(prepared) = &result {
                        let code = if entry.cancel.is_cancelled() {
                            Some(ErrorCode::Cancelled)
                        } else if publication_expired(
                            entry.publication_deadline,
                            tokio::time::Instant::now(),
                        ) {
                            Some(ErrorCode::Timeout)
                        } else {
                            None
                        };
                        if let Some(code) = code {
                            let facts = Some(prepared.opened.facts.clone());
                            prepared.revoke_native_route();
                            result = Err(Failure {
                                detail: None,
                                setup_cause: None,
                                code,
                                effect: Effect::None,
                                facts,
                            });
                        }
                    }
                    if self.shutting_down || entry.retired {
                        drop(result);
                        continue;
                    }
                    // The key learns how the attempt's setup went; a retried
                    // open waits for its next attempt (retry.rs). A cancelled
                    // one told it at its cancellation.
                    if !entry.cancel.is_cancelled() {
                        let member = (request.clone(), entry.envelope.stream_id);
                        let settled = self.retries.settle(
                            self.now_ms,
                            member,
                            entry,
                            result,
                            connect,
                            &mut retry,
                        );
                        let Some(settled) = settled else {
                            continue;
                        };
                        result = settled;
                    }
                    let mut receipt = Envelope {
                        version: entry.envelope.version,
                        session_id: entry.envelope.session_id.clone(),
                        stream_id: entry.envelope.stream_id,
                        ..Default::default()
                    };
                    match result {
                        Ok(mut prepared) => {
                            entry.publication_route = prepared.native_publication_route();
                            prepared.opened.endpoint_id = self.endpoint.clone();
                            prepared.opened.trust_owner = self.trust_owner.clone();
                            let limits = entry
                                .envelope
                                .open
                                .as_ref()
                                .expect("Open")
                                .receive_limits
                                .clone();
                            prepared.opened.receive_limits = limits.clone();
                            receipt.kind = MessageKind::Opened;
                            receipt.opened = Some(prepared.opened.clone());
                            let mut config = stream::Config::new(
                                &receipt.session_id,
                                receipt.stream_id,
                                stream::Side::Endpoint,
                            );
                            config.profile_version = 2;
                            config.io_timeout_ms = prepared.io_timeout_ms();
                            config.receive_window = limits.receive_window as usize;
                            config.peer_receive_window = limits.receive_window as usize;
                            config.max_payload =
                                config.max_payload.min(limits.data_payload as usize);
                            config.receive_limits = limits.clone();
                            config.peer_limits = limits;
                            let (stream, peer) =
                                Stream::new(config).map_err(|_| EndpointError::Protocol)?;
                            let peer = Arc::new(peer);
                            peer.advance(now);
                            if entry.publication_deadline.is_none()
                                && https_policy::advertisement(
                                    entry.envelope.open.as_ref().expect("Open").service,
                                )
                            {
                                entry.serving = Some(self.runtime.spawn(prepared.serve(
                                    stream.clone(),
                                    peer.clone(),
                                    entry.cancel.clone(),
                                    entry.wake.clone(),
                                )));
                            } else {
                                entry.prepared = Some(prepared);
                            }
                            entry.stream = Some(stream);
                            entry.peer = Some(peer);
                        }
                        Err(failure) => {
                            let open = entry.envelope.open.as_ref().expect("Open");
                            if !entry.cancel.is_cancelled()
                                && open.policy == AuthPolicy::Anonymous
                                && https_policy::advertisement(open.service)
                                && matches!(
                                    failure.code,
                                    ErrorCode::Authentication | ErrorCode::RepositoryRefused
                                )
                                && failure
                                    .facts
                                    .as_ref()
                                    .is_some_and(|f| matches!(f.http_status, Some(401 | 404)))
                            {
                                if let Some(operation) = self.operations.get_mut(request) {
                                    if operation.retries.len() < 64 {
                                        operation.retries.insert(retry_key(open), retry);
                                    }
                                }
                            }
                            receipt.kind = MessageKind::OpenFailed;
                            receipt.open_failed = Some(failure);
                        }
                    }
                    entry.output = Some(receipt);
                }
            }
            if let Some(peer) = &entry.peer {
                peer.advance(now);
                entry.wake.notify();
            }
            if entry.serving.is_none() {
                if entry.handoff {
                    let Some(prepared) = entry.prepared.take() else {
                        return Err(EndpointError::Protocol);
                    };
                    let Some(peer) = entry.peer.clone() else {
                        return Err(EndpointError::Protocol);
                    };
                    let Some(stream) = entry.stream.clone() else {
                        return Err(EndpointError::Protocol);
                    };
                    entry.serving = Some(self.runtime.spawn(prepared.serve(
                        stream,
                        peer.clone(),
                        entry.cancel.clone(),
                        entry.wake.clone(),
                    )));
                    entry.handoff = false;
                }
            }
            if let Some(task) = entry.serving.as_mut() {
                if let Poll::Ready(result) = Pin::new(task).poll(cx) {
                    result.map_err(|_| EndpointError::Protocol)?;
                    entry.serving = None;
                }
            }
        }
        self.client.governor().tick(self.client.pool_now());
        self.start_held();
        self.entries.retain(|_, entry| {
            !(entry.retired && entry.preparing.is_none() && entry.serving.is_none())
        });
        Ok(())
    }
    pub(in crate::transport_host) fn take_outbound(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<Outbound> {
        let keys: Vec<_> = self
            .entries
            .keys()
            .filter(|key| self.last_outbound.as_ref().is_none_or(|last| *key > last))
            .chain(
                self.entries
                    .keys()
                    .filter(|key| self.last_outbound.as_ref().is_some_and(|last| *key <= last)),
            )
            .cloned()
            .collect();
        for key in keys {
            let entry = self.entries.get_mut(&key).expect("live key");
            let request = &key.0;
            if entry.retired {
                continue;
            }
            let mut message = entry.output.take();
            if message.is_none() {
                if let Some(peer) = &entry.peer {
                    let next = entry
                        .next
                        .get_or_insert_with(|| super::super::session::next_message(peer));
                    if let Poll::Ready(result) = next.as_mut().poll(cx) {
                        entry.next = None;
                        entry.wake.notify();
                        message = result.ok().flatten();
                    }
                }
            }
            if let Some(envelope) = message {
                if matches!(
                    envelope.kind,
                    MessageKind::OpenFailed | MessageKind::Closed | MessageKind::Failed
                ) {
                    entry.retired = true;
                }
                self.last_outbound = Some(key.clone());
                return Some(Outbound {
                    request: request.clone(),
                    envelope,
                });
            }
        }
        None
    }
}
