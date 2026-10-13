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
                    let machine = retry_key(&job.pool_key, &job.envelope);
                    let admitted = self
                        .retries
                        .machine(&job.key.0, &machine)
                        .succeeded(&job.key, !opened.reused);
                    self.health(&job.key.0, &job.pool_key, &machine);
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
        // A member that ran out of its allocation waiting on a local budget:
        // the pool host saw which, and for how long, and the wire carries only
        // that it was local (Code P3-6).
        let local_wait = matches!(
            setup_retry::CapacityReport::of_failure(&failure),
            Some(setup_retry::CapacityReport::Local)
        )
        .then(|| {
            self.endpoint
                .governor()
                .take_local_wait(&pool_key.site(), self.endpoint.pool_now())
        })
        .flatten();
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
                // No verdict for the key, but a Down key's retest that ends
                // so is a failed one: its next is paced 30 s on (§5.5).
                self.retries
                    .machine(&key.0, &machine)
                    .retest_refused(&key, now);
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
        self.health(&key.0, &pool_key, &machine);
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
            // The budget is spent, or a retest failed: the members queued on
            // the key share its failure, and later arrivals park (§5.5).
            Outcome::Sweep(last) => {
                // A failed retest has no attempt count: its member and the
                // members parked on it say it is still failing (§5.3).
                if last.attempt == 0 {
                    self.note_end(&key, setup_retry::EndNote::StillFailing);
                }
                self.sweep(&key.0, &machine, &last);
                self.fail_open(
                    &key,
                    Failure {
                        facts: failure.facts,
                        ..last.wire_failure()
                    },
                );
            }
            Outcome::Return => {
                if let Some(wait) = local_wait {
                    self.note_end(&key, setup_retry::EndNote::LocalWait(wait));
                }
                self.fail_open(&key, failure)
            }
        }
    }

    /// Tells the site's limit machine what the retry machine's last call
    /// changed about the key's health, which freezes or restores `N` (§5.5).
    pub(super) fn health(&mut self, operation: &str, pool_key: &Key, machine: &RetryKey) {
        let machine = self.retries.machine(operation, machine);
        let changes = machine
            .take_change()
            .into_iter()
            .chain(machine.take_down().then_some(setup_retry::Change::Down));
        for change in changes.collect::<Vec<_>>() {
            self.endpoint.governor().scoped(operation).health(
                pool_key,
                change,
                self.endpoint.pool_now(),
            );
        }
    }

    /// Finishes every member of `operation` queued on `machine`'s key with the
    /// failure the key went Down on, as the key's budget being spent finishes
    /// them (§5.5).
    fn sweep(&mut self, operation: &str, machine: &RetryKey, last: &setup_retry::Final) {
        let (swept, kept): (VecDeque<_>, VecDeque<_>) = std::mem::take(&mut self.queued_opens)
            .into_iter()
            .partition(|queued| {
                queued.key.0 == operation
                    && retry_key(&queued.pool_key, &queued.envelope) == *machine
            });
        self.queued_opens = kept;
        for queued in swept {
            if last.attempt == 0 {
                self.note_end(&queued.key, setup_retry::EndNote::StillFailing);
            }
            let failure = Failure {
                facts: None,
                ..last.clone().wire_failure()
            };
            self.fail_open(&queued.key, failure);
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
            let spent = setup_retry::spent(failure, None, attempts, allowed);
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
