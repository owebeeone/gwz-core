use super::*;
pub(super) fn run<C>(
    receiver: Receiver<OpenRequest>,
    pool: Pool,
    host: &mut PoolHost<C>,
    admissions: &mut Admissions,
    io_timeout_ms: u64,
    cleanup: u64,
    stop: Arc<AtomicBool>,
    clock: impl Fn() -> u64,
    worker_id: u64,
    status: &Status,
) where
    C: Connector,
    C::Resource: ChannelResource,
{
    let _stop_on_exit = StopOnExit(stop.clone());
    let stall_ms = host.stall_slot();
    stall_ms.store(io_timeout_ms, Ordering::Relaxed);
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut pending = Vec::<Pending>::new();
    let mut active = Vec::<Active>::new();
    let mut serial = 0_i64;
    let mut passwords = 0_u64;
    let mut stopping_at = None;
    let session = format!("ssh-worker-{worker_id}");
    loop {
        let now = clock();
        if stop.load(Ordering::Acquire) && stopping_at.is_none() {
            stopping_at = Some(now.saturating_add(cleanup));
            for item in pending.drain(..) {
                item.request.complete(Err(stopped()));
            }
            active.clear(); // disconnect Git callers before waiting for physical disposal
            pool.shutdown();
        }
        pending.retain_mut(|item| {
            if item.request.expired(now) {
                item.request.reject(io::ErrorKind::TimedOut);
                false
            } else {
                true
            }
        });
        let ready = admissions.poll(&mut cx, now, stopping_at.is_some());
        if let Some(error) = admissions.take_failure() {
            ssh_shutdown::fail(status, error);
            stop.store(true, Ordering::Release);
        }
        if host
            .step_reported(&mut cx, now, |connection| {
                let item = pending
                    .iter()
                    .find(|p| p.checkout.opening_connection() == Some(connection));
                let stall = item
                    .map(|p| {
                        let io_ms = p.request.context.deadlines.io_ms;
                        if io_ms <= 0 { 0 } else { io_ms as u64 }
                    })
                    .unwrap_or(io_timeout_ms);
                stall_ms.store(stall, Ordering::Relaxed);
                item.map(|p| Opening {
                    progress: p.request.progress.clone(),
                    url: p.request.url.clone(),
                    selected: p.request.authority.clone(),
                    setup_slot: p.request.setup_slot.clone(),
                    path: p.request.path.clone(),
                    allocation_ms: p.request.context.deadlines.allocation_ms as u64,
                    interaction_ms: p.request.context.deadlines.interaction_ms as u64,
                    ..Opening::default()
                })
                .unwrap_or_default()
            })
            .is_err()
        {
            ssh_shutdown::fail(status, io::ErrorKind::Other);
            break;
        }
        ssh_shutdown::sample(status, host, admissions);
        if let Some(error) = host.take_disposal_error() {
            ssh_shutdown::fail(status, error.kind());
            stop.store(true, Ordering::Release);
            if stopping_at.is_none() {
                continue;
            }
        }
        if stopping_at.is_some_and(|at| now >= at)
            || (stopping_at.is_some() && host.shutdown_complete() && admissions.is_empty())
        {
            break; // The owner transfers unfinished cleanup to the reserved supervisor slot.
        }
        let incoming = receiver.try_iter().take(32);
        for mut request in ready.into_iter().chain(incoming) {
            if stopping_at.is_some() {
                request.complete(Err(stopped()));
                continue;
            }
            if request.expired(now) {
                request.complete(Err(io::ErrorKind::TimedOut.into()));
                continue;
            }
            if request.selected.is_some() {
                admissions.start(request);
                continue;
            }
            let Some(next) = serial.checked_add(1) else {
                request.complete(Err(io::Error::other("stream IDs exhausted")));
                continue;
            };
            serial = next;
            // A URL password authenticates its own open's connection only, as
            // libgit2's does (TR2.18): the open gets an identity of its own,
            // so no other open, with the password or without it, shares it.
            if request
                .url
                .as_ref()
                .is_some_and(|url| url.password().is_some())
            {
                let Some(next) = passwords.checked_add(1) else {
                    request.complete(Err(io::Error::other("password identities exhausted")));
                    continue;
                };
                passwords = next;
                request.identity = Identity::Explicit(format!("url-password-{next}"));
            }
            let mut policy = gwz_transport::pool::Request::new(
                request.key.clone(),
                request.identity.clone(),
                Owner::new(&session, serial.to_string()),
            );
            let d = &request.context.deadlines;
            policy.allocation_timeout_ms = Some(d.allocation_ms as u64);
            policy.connect_timeout_ms = Some(d.connect_ms as u64);
            policy.interaction_timeout_ms = Some(d.interaction_ms as u64);
            match pool.checkout_until(policy, request.deadline) {
                Ok(checkout) => pending.push(Pending { checkout, request }),
                Err(error) => request.complete(Err(io::Error::other(error))),
            }
        }
        let mut index = 0;
        while index < pending.len() {
            if pending[index].request.expired(now) {
                let item = pending.swap_remove(index);
                item.request.complete(Err(io::ErrorKind::TimedOut.into()));
                continue;
            }
            let ready = pin!(&mut pending[index].checkout).poll(&mut cx);
            match ready {
                Poll::Pending => index += 1,
                Poll::Ready(result) => {
                    let item = pending.swap_remove(index);
                    let result = result.map_err(io::Error::other).and_then(|lease| {
                        attach(host, lease, &item.request, &session, now, &mut active)
                    });
                    item.request.complete(result);
                }
            }
        }
        let mut index = 0;
        while index < active.len() {
            let exchange = &mut active[index];
            let discard = exchange.discard.load(Ordering::Acquire);
            let disposition = match host.resource(exchange.lease.as_ref().expect("active lease")) {
                Ok(resource) => {
                    let result = match resource.pump() {
                        Some(pump) => transfer(pump, exchange, &mut cx, now),
                        None => Err(()),
                    };
                    match result {
                        Ok(false) => None,
                        Ok(true) if !discard && resource.reclaim() => Some(Disposition::Reusable),
                        _ => Some(Disposition::Discarded),
                    }
                }
                Err(_) => Some(Disposition::Discarded),
            };
            if let Some(disposition) = disposition {
                let exchange = active.swap_remove(index);
                // Active owns a disconnect guard, so consume its lease through
                // a separate release helper after detaching that guard below.
                release(host, exchange, disposition);
            } else {
                index += 1;
            }
        }
        // The host has no socket readiness API yet. Park between bounded polls;
        // callers wake immediately, timers run independently, and idle pools sleep.
        let wait = if active.is_empty()
            && pending.is_empty()
            && admissions.is_empty()
            && stopping_at.is_none()
        {
            host.next_deadline()
                .map_or(1000, |at| at.saturating_sub(now).min(1000))
        } else {
            1
        };
        thread::park_timeout(Duration::from_millis(wait.max(1)));
    }
    for item in pending {
        item.request.complete(Err(stopped()));
    }
    drop(active);
    // Dropping receiver releases queued permits and wakes waiting opens.
}
