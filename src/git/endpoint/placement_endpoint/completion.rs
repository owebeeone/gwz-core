use super::*;

impl PlacementEndpoint {
    pub(super) fn finish_checks(&mut self, now_ms: u64, cx: &mut Context<'_>) {
        let mut index = 0;
        while index < self.checks.len() {
            // Logical completion never waits for a blocked filesystem job to
            // return. Its physical owner remains charged until disposal.
            let expired_key = {
                let check = &mut self.checks[index];
                if !check.cancelled && now_ms >= check.deadline {
                    check.cancelled = true;
                    check.job.cancel();
                    Some(check.key.clone())
                } else {
                    None
                }
            };
            if let Some(key) = expired_key {
                if let Some(state) = self.requests.get_mut(&key) {
                    if !state.terminal {
                        state.terminal = true;
                        let message = envelope_for(
                            state,
                            MessageKind::IdentityCheckFailed,
                            Some(Failure {
                                setup_cause: None,
                                code: ErrorCode::Timeout,
                                effect: Effect::None,
                                facts: None,
                            }),
                            None,
                        );
                        self.push_outbound(key.0, message);
                    }
                }
            }
            let check = &mut self.checks[index];
            let expired = now_ms >= check.deadline;
            if expired {
                check.cancelled = true;
            }
            let result = if check.cancelled {
                match check.job.poll_disposed(cx) {
                    Poll::Ready(Ok(())) => Some(Err(if expired {
                        ErrorCode::Timeout
                    } else {
                        ErrorCode::Cancelled
                    })),
                    Poll::Ready(Err(_)) => None,
                    Poll::Pending => None,
                }
            } else {
                match check.job.poll_result(cx) {
                    Poll::Ready(Ok(())) => Some(Ok(())),
                    Poll::Ready(Err(error)) => Some(Err(match error.kind() {
                        io::ErrorKind::InvalidInput => ErrorCode::InvalidRequest,
                        io::ErrorKind::TimedOut => ErrorCode::Timeout,
                        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
                            ErrorCode::Unavailable
                        }
                        _ => ErrorCode::Io,
                    })),
                    Poll::Pending => None,
                }
            };
            let Some(result) = result else {
                index += 1;
                continue;
            };
            let key = self.checks.swap_remove(index).key;
            let Some(state) = self.requests.get_mut(&key) else {
                continue;
            };
            let already_terminal = state.terminal;
            state.terminal = true;
            if already_terminal {
                continue;
            }
            let message = match result {
                Ok(_) => envelope_for(state, MessageKind::IdentityChecked, None, None),
                Err(code) => envelope_for(
                    state,
                    MessageKind::IdentityCheckFailed,
                    Some(Failure {
                        setup_cause: None,
                        code,
                        effect: Effect::None,
                        facts: None,
                    }),
                    None,
                ),
            };
            self.push_outbound(key.0, message);
        }
    }

    pub(super) fn finish_opens(&mut self, now_ms: u64) {
        let mut index = 0;
        while index < self.opens.len() {
            let expired = {
                let open = &mut self.opens[index];
                if !open.abandoned && open.deadline.is_some_and(|deadline| now_ms >= deadline) {
                    open.abandoned = true;
                    open.cancelled
                        .store(true, std::sync::atomic::Ordering::Release);
                    Some(open.key.clone())
                } else {
                    None
                }
            };
            if let Some(key) = expired {
                if let Some(state) = self.requests.get_mut(&key) {
                    if !state.terminal {
                        state.terminal = true;
                        let message = envelope_for(
                            state,
                            MessageKind::OpenFailed,
                            Some(Failure {
                                setup_cause: None,
                                code: ErrorCode::Timeout,
                                effect: Effect::None,
                                facts: None,
                            }),
                            None,
                        );
                        self.push_outbound(key.0, message);
                    }
                }
            }
            // An abandoned open waits for the worker's reply too, which its
            // cancellation brings early; a late success is cancelled below.
            let Poll::Ready(result) = self.opens[index].reply.poll() else {
                index += 1;
                continue;
            };
            let job = self.opens.swap_remove(index);
            if job.abandoned {
                if let Ok((attachment, _)) = result {
                    attachment.cancel();
                }
                if let Some(state) = self.requests.get_mut(&job.key) {
                    if !state.terminal {
                        state.terminal = true;
                        let message = envelope_for(
                            state,
                            MessageKind::OpenFailed,
                            Some(Failure {
                                setup_cause: None,
                                code: ErrorCode::Timeout,
                                effect: Effect::None,
                                facts: None,
                            }),
                            None,
                        );
                        self.push_outbound(job.key.0, message);
                    }
                }
                continue;
            }
            match result {
                Ok((attachment, mut opened)) => {
                    opened.endpoint_id = self.endpoint_id.clone();
                    opened.trust_owner = self.trust_owner.clone();
                    if !self.requests.contains_key(&job.key) {
                        attachment.cancel();
                        continue;
                    }
                    let message = {
                        let state = self.requests.get_mut(&job.key).expect("open request");
                        let message = envelope_for(state, MessageKind::Opened, None, Some(opened));
                        state.attachment = Some(attachment);
                        message
                    };
                    self.push_outbound(job.key.0.clone(), message);
                }
                Err(error) => {
                    if !self.requests.contains_key(&job.key) {
                        continue;
                    }
                    let message = {
                        let state = self.requests.get_mut(&job.key).expect("open request");
                        state.terminal = true;
                        envelope_for(
                            state,
                            MessageKind::OpenFailed,
                            Some(failure_for(error)),
                            None,
                        )
                    };
                    self.push_outbound(job.key.0.clone(), message);
                }
            }
        }
    }
}
