use super::*;

impl PlacementEndpoint {
    pub(crate) fn cancel_request(&mut self, request: &str) {
        let keys: Vec<_> = self
            .requests
            .keys()
            .filter(|key| key.0 == request)
            .cloned()
            .collect();
        for key in keys {
            self.cancel_stream(&key);
        }
        self.queued_opens.retain(|queued| queued.key.0 != request);
        for check in &mut self.checks {
            if check.key.0 == request {
                check.cancelled = true;
            }
        }
        for open in &mut self.opens {
            if open.key.0 == request {
                open.abandoned = true;
                open.cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        }
        // Its members are all finished: no wake opens a probe for it.
        self.retries.remove(request);
    }
    /// Ends one cancelled stream: queues its `Cancelled` terminal, unless it
    /// has one already, and abandons its attached exchange.
    pub(super) fn cancel_stream(&mut self, key: &RequestKey) {
        let is_check = self.checks.iter().any(|check| check.key == *key);
        let Some(state) = self.requests.get_mut(key) else {
            return;
        };
        let message = if !state.terminal {
            state.terminal = true;
            Some(envelope_for(
                state,
                if is_check {
                    MessageKind::IdentityCheckFailed
                } else if state.attachment.is_none() {
                    MessageKind::OpenFailed
                } else {
                    MessageKind::Failed
                },
                Some(Failure {
                    detail: None,
                    setup_cause: None,
                    code: ErrorCode::Cancelled,
                    effect: Effect::None,
                    facts: None,
                }),
                None,
            ))
        } else {
            None
        };
        if let Some(attachment) = &state.attachment {
            attachment.cancel();
        }
        if let Some(message) = message {
            self.push_outbound(key.0.clone(), message);
        }
    }
    pub(crate) fn shutdown(&mut self) {
        if self.shutting_down {
            return;
        }
        self.shutting_down = true;
        let requests: Vec<_> = self.requests.keys().cloned().collect();
        for request in requests {
            let is_check = self.checks.iter().any(|check| check.key == request);
            if let Some(state) = self.requests.get_mut(&request) {
                if !state.terminal {
                    state.terminal = true;
                    let message = envelope_for(
                        state,
                        if is_check {
                            MessageKind::IdentityCheckFailed
                        } else if state.attachment.is_none() {
                            MessageKind::OpenFailed
                        } else {
                            MessageKind::Failed
                        },
                        Some(Failure {
                            detail: None,
                            setup_cause: None,
                            code: ErrorCode::CarrierLost,
                            effect: Effect::None,
                            facts: None,
                        }),
                        None,
                    );
                    self.push_outbound(request.0.clone(), message);
                }
            }
        }
        for state in self.requests.values() {
            if let Some(attachment) = &state.attachment {
                attachment.cancel();
            }
        }
        for open in &mut self.opens {
            open.abandoned = true;
            open.cancelled
                .store(true, std::sync::atomic::Ordering::Release);
        }
        for check in &mut self.checks {
            check.cancelled = true;
        }
        self.endpoint.shutdown();
        self.outbound.clear();
        self.terminal_outbound.clear();
        self.requests.clear();
        self.queued_opens.clear();
        self.retries.clear();
    }
}
