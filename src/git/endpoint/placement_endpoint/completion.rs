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
                                detail: None,
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
                        detail: None,
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
                    Some((open.key.clone(), retry_key(&open.pool_key, &open.envelope)))
                } else {
                    None
                }
            };
            if let Some((key, machine)) = expired {
                // The attempt ran out of its own time before its setup had an
                // outcome: no verdict for the key.
                self.retries.machine(&key.0, &machine).abandoned(&key);
                self.fail_open(
                    &key,
                    Failure {
                        detail: None,
                        setup_cause: None,
                        code: ErrorCode::Timeout,
                        effect: Effect::None,
                        facts: None,
                    },
                );
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
                self.fail_open(
                    &job.key,
                    Failure {
                        detail: None,
                        setup_cause: None,
                        code: ErrorCode::Timeout,
                        effect: Effect::None,
                        facts: None,
                    },
                );
                continue;
            }
            match result {
                Ok((attachment, mut opened)) => {
                    let admitted = self
                        .retries
                        .machine(&job.key.0, &retry_key(&job.pool_key, &job.envelope))
                        .succeeded(&job.key, !opened.reused);
                    if !admitted {
                        attachment.discard_after_use();
                    }
                    opened.endpoint_id = self.endpoint_id.clone();
                    opened.trust_owner = self.trust_owner.clone();
                    if !self.requests.contains_key(&job.key) {
                        attachment.cancel();
                        continue;
                    }
                    let facts = std::mem::take(&mut opened.facts);
                    opened.facts = self.keep_facts(&job.key, Some(facts)).unwrap_or_default();
                    let message = {
                        let state = self.requests.get_mut(&job.key).expect("open request");
                        let message = envelope_for(state, MessageKind::Opened, None, Some(opened));
                        state.attachment = Some(attachment);
                        message
                    };
                    self.push_outbound(job.key.0.clone(), message);
                }
                Err(error) => {
                    self.attempt_failed(
                        job.key,
                        job.pool_key,
                        job.envelope,
                        &error,
                        job.attempts,
                        now_ms,
                    );
                }
            }
        }
    }

    /// A member's attempt failed with `error`. The key's machine decides,
    /// from the failure and its phase (the retry plan's §4): the member waits
    /// in the queue for another attempt, or is finished.
    pub(super) fn attempt_failed(
        &mut self,
        key: RequestKey,
        pool_key: Key,
        envelope: Envelope,
        error: &io::Error,
        attempts: u32,
        now: u64,
    ) {
        if !self.requests.contains_key(&key) {
            return;
        }
        let (failure, phase) = open_failure(error);
        let verdict = setup_retry::classify(&failure, phase);
        let jitter = self.jitter.draw();
        let machine = retry_key(&pool_key, &envelope);
        let allowed = self.retries.max_retries(&key.0).saturating_add(1);
        // A refusal that looks like the host's limit is judged by the site's
        // machine (§4.8): when it is evidence, the member is requeued and the
        // key is not made to count it (§5.1).
        if verdict == Verdict::Retry
            && let Some(signal) = setup_retry::suspect(&failure, phase, true)
        {
            let ruling = self.endpoint.governor().scoped(&key.0).setup_failed(
                &pool_key,
                &key.1.to_string(),
                signal,
                self.endpoint.pool_now(),
            );
            if setup_retry::requeues(ruling, false) {
                self.retries.machine(&key.0, &machine).abandoned(&key);
                self.requeue(key, pool_key, envelope, failure, attempts, allowed, now);
                return;
            }
        }
        let outcome = self.retries.machine(&key.0, &machine).failed(
            &key,
            verdict,
            failure.clone(),
            now,
            jitter,
        );
        match outcome {
            Outcome::Retry => {
                self.requeue(key, pool_key, envelope, failure, attempts, allowed, now)
            }
            // Its own attempts' facts, also when the key finishes it with a
            // failure another member's setup recorded.
            Outcome::Finish(last) => self.fail_open(
                &key,
                Failure {
                    facts: failure.facts,
                    ..last.wire_failure()
                },
            ),
            Outcome::Return => self.fail_open(&key, failure),
        }
    }

    /// Returns a member to the queue for another attempt, or finishes it when
    /// the `allowed` attempts it made are spent (§5.3): the key stays as it is.
    #[allow(clippy::too_many_arguments)] // One member's attempt, as it ended.
    fn requeue(
        &mut self,
        key: RequestKey,
        pool_key: Key,
        envelope: Envelope,
        failure: Failure,
        attempts: u32,
        allowed: u32,
        now: u64,
    ) {
        if attempts >= allowed {
            let facts = failure.facts.clone();
            let spent = setup_retry::spent(failure, false, attempts, allowed);
            self.fail_open(&key, Failure { facts, ..spent });
            return;
        }
        self.keep_facts(&key, failure.facts);
        let allocation = envelope
            .open
            .as_ref()
            .map_or(0, |open| open.deadlines.allocation_ms.max(0) as u64);
        self.queued_opens.push_back(QueuedOpen {
            key,
            pool_key,
            envelope,
            allocation: AllocationClock::new(now, allocation),
            attempts,
        });
    }
}
