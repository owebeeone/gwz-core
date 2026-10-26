use super::*;
use crate::git::endpoint::{
    agent_auth::{Authenticator, Check, Evidence, Possession, Verdict},
    agent_job::{self as job, Job},
    ssh_pool::{Lessee, Proofs},
};
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
    let mut deferred = Vec::<Deferred>::new();
    let mut active = Vec::<Active>::new();
    // Leases another binding last used, waiting for this binding's reuse proof, and the proofs bindings have passed
    // (reuse design §4).
    let mut proving = Vec::<SharedProof>::new();
    let mut proofs = Proofs::new();
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
            for item in deferred.drain(..) {
                item.request.complete(Err(stopped()));
            }
            for shared in proving.drain(..) {
                for waiter in shared.waiters {
                    let _ = host.release(waiter.lease, Disposition::Discarded);
                    waiter.request.complete(Err(stopped()));
                }
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
        deferred.retain_mut(|item| {
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
                    identity: Some(p.request.identity.clone()),
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
        // An open that waited for a closing exchange goes to the pool when that
        // exchange has ended, or when its wait has.
        let mut index = 0;
        while index < deferred.len() {
            let item = &deferred[index];
            let waiting = now < item.until
                && active.iter().any(|exchange| {
                    exchange
                        .closing
                        .as_ref()
                        .is_some_and(|closing| closing.claimed == Some(item.claim))
                });
            if waiting {
                index += 1;
                continue;
            }
            let Deferred {
                request, policy, ..
            } = deferred.swap_remove(index);
            match pool.checkout_until(policy.clone(), request.deadline) {
                Ok(checkout) => pending.push(Pending {
                    checkout,
                    request,
                    policy,
                }),
                Err(error) => request.complete(Err(io::Error::other(error))),
            }
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
            // Disabled operations must not lease an enabled operation's
            // helper-authenticated connection. Keep selection in Opening and
            // partition only this endpoint's existing opaque pool identity.
            let pool_identity = if request
                .url
                .as_ref()
                .is_some_and(|url| !url.helpers_allowed())
            {
                Identity::Explicit(match &request.identity {
                    Identity::Ambient => "helpers-disabled:ambient".into(),
                    Identity::Explicit(token) => format!("helpers-disabled:explicit:{token}"),
                    Identity::Https | Identity::HttpsScoped(_) => {
                        request.complete(Err(io::ErrorKind::InvalidInput.into()));
                        continue;
                    }
                })
            } else {
                request.identity.clone()
            };
            let mut policy = gwz_transport::pool::Request::new(
                request.key.clone(),
                pool_identity,
                Owner::new(&session, serial.to_string()),
            );
            let d = &request.context.deadlines;
            policy.allocation_timeout_ms = Some(d.allocation_ms as u64);
            policy.connect_timeout_ms = Some(d.connect_ms as u64);
            policy.interaction_timeout_ms = Some(d.interaction_ms as u64);
            policy.tag = request.tag.clone();
            // An open for a connection that is closing waits for it, once; a
            // test of the site's limit does not, for it needs a connection of
            // its own at once (§4.7).
            if !request.carrier
                && let Some(until) =
                    claim_closing(&mut active, &request.key, &policy.identity, serial, now)
            {
                deferred.push(Deferred {
                    request,
                    policy,
                    claim: serial,
                    until,
                });
                continue;
            }
            match pool.checkout_until(policy.clone(), request.deadline) {
                Ok(checkout) => pending.push(Pending {
                    checkout,
                    request,
                    policy,
                }),
                Err(error) => request.complete(Err(io::Error::other(error))),
            }
        }
        let mut index = 0;
        while index < pending.len() {
            if pending[index].request.expired(now) {
                let item = pending.swap_remove(index);
                item.request.complete_expired(io::ErrorKind::TimedOut);
                continue;
            }
            let ready = pin!(&mut pending[index].checkout).poll(&mut cx);
            match ready {
                Poll::Pending => index += 1,
                Poll::Ready(result) => {
                    let item = pending.swap_remove(index);
                    let unserved = match result {
                        Ok(lease) => match screen(
                            host,
                            &mut proofs,
                            &mut proving,
                            lease,
                            item.request,
                            item.policy,
                        ) {
                            Screened::Attach(lease, request, policy) => {
                                attach(host, lease, request, policy, &session, now, &mut active)
                            }
                            Screened::Fresh(request, policy) => {
                                Some(Unserved::Revoked(request, policy))
                            }
                            Screened::Denied(request, policy) => {
                                Some(Unserved::Denied(request, policy))
                            }
                            Screened::Waiting => None,
                        },
                        Err(error) => Some(Unserved::Failed(item.request, io::Error::other(error))),
                    };
                    if let Some(unserved) = unserved {
                        unserved.settle(&pool, &mut pending);
                    }
                }
            }
        }
        for screened in settle_proofs(host, &mut proofs, &mut proving, &mut cx, now) {
            let unserved = match screened {
                Screened::Attach(lease, request, policy) => {
                    attach(host, lease, request, policy, &session, now, &mut active)
                }
                Screened::Fresh(request, policy) => Some(Unserved::Revoked(request, policy)),
                Screened::Denied(request, policy) => Some(Unserved::Denied(request, policy)),
                Screened::Waiting => None,
            };
            if let Some(unserved) = unserved {
                unserved.settle(&pool, &mut pending);
            }
        }
        let mut index = 0;
        while index < active.len() {
            let exchange = &mut active[index];
            if exchange.held.as_ref().is_some_and(Held::cancelled) {
                let mut exchange = active.swap_remove(index);
                let held = exchange.held.take().expect("held open");
                release(host, exchange, Disposition::Discarded);
                held.cancel();
                continue;
            }
            let discard = exchange.discard.load(Ordering::Acquire);
            let mut opened = false;
            let (disposition, failed) = match host
                .resource(exchange.lease.as_ref().expect("active lease"))
            {
                Ok(resource) => {
                    let result = match resource.pump() {
                        Some(pump) if exchange.closing.is_some() => {
                            close_step(pump, exchange, &mut cx, now, discard)
                        }
                        Some(pump) => {
                            let result = transfer(pump, exchange, &mut cx, now, discard);
                            opened = pump.opened();
                            result
                        }
                        None => Err(()),
                    };
                    match result {
                        Ok(Turn::Running | Turn::Closing) => (None, false),
                        Ok(Turn::Finished) if resource.reclaim() => {
                            (Some(Disposition::Reusable), false)
                        }
                        Ok(Turn::Finished | Turn::Over) => (Some(Disposition::Discarded), false),
                        Err(()) => (Some(Disposition::Discarded), true),
                    }
                }
                Err(_) => (Some(Disposition::Discarded), true),
            };
            if opened && let Some(held) = exchange.held.take() {
                held.reply();
            }
            if let Some(disposition) = disposition {
                let mut exchange = active.swap_remove(index);
                let held = exchange.held.take();
                // Active owns a disconnect guard, so consume its lease through
                // a separate release helper after detaching that guard below.
                release(host, exchange, disposition);
                if let Some(held) = held {
                    held.unserved(failed).settle(&pool, &mut pending);
                }
            } else {
                index += 1;
            }
        }
        // The host has no socket readiness API yet. Park between bounded polls;
        // callers wake immediately, timers run independently, and idle pools sleep.
        let wait = if active.is_empty()
            && pending.is_empty()
            && proving.is_empty()
            && deferred.is_empty()
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
    for item in deferred {
        item.request.complete(Err(stopped()));
    }
    for shared in proving {
        for waiter in shared.waiters {
            let _ = host.release(waiter.lease, Disposition::Discarded);
            waiter.request.complete(Err(stopped()));
        }
    }
    drop(active);
    // Dropping receiver releases queued permits and wakes waiting opens.
}

/// Claims the closing exchange, if one is, that an open for `key` and
/// `identity` waits for, and returns when its wait ends: `WAIT_FOR_CLOSE_MS`,
/// or less if the close is given up on sooner. One open waits for one close,
/// and an open waits once (design §6).
fn claim_closing(
    active: &mut [Active],
    key: &Key,
    identity: &Identity,
    claim: i64,
    now: u64,
) -> Option<u64> {
    let exchange = active.iter_mut().find(|exchange| {
        exchange.key == *key
            && exchange.identity == *identity
            && !exchange.discard.load(Ordering::Acquire)
            && exchange
                .closing
                .as_ref()
                .is_some_and(|closing| !closing.failed && closing.claimed.is_none())
    })?;
    let closing = exchange.closing.as_mut()?;
    closing.claimed = Some(claim);
    let wait = WAIT_FOR_CLOSE_MS.min(closing.deadline.saturating_sub(now));
    Some(now.saturating_add(wait))
}

/// What a lease does next, once its binding's reuse authority for it is known (reuse design §4).
enum Screened {
    /// It serves the open now.
    Attach(Lease, OpenRequest, gwz_transport::pool::Request),
    /// It cannot serve the open (it is back to idle, or retired): the open asks again for a new connection.
    Fresh(OpenRequest, gwz_transport::pool::Request),
    /// The proof of it failed and it is retired: the open asks again for a new connection, and takes no idle one.
    Denied(OpenRequest, gwz_transport::pool::Request),
    /// It waits for its binding's proof.
    Waiting,
}
/// A lease waiting for its binding's reuse proof. While it waits the connection is that binding's lease.
struct Waiter {
    lease: Lease,
    request: OpenRequest,
    policy: gwz_transport::pool::Request,
}
/// One reuse proof a binding needs, which every lease of that binding needing the same proof shares (reuse design §4,
/// "A shared job"). It runs under the latest deadline among its waiters when it started; a waiter whose own deadline
/// passes leaves alone, and a job that ended on its deadline while a later waiter still has time starts again for it.
struct SharedProof {
    binding: String,
    key: Option<Vec<u8>>,
    host: Option<(String, u16, Vec<u8>)>,
    evidence: Arc<Evidence>,
    job: Job<Verdict>,
    deadline: Option<u64>,
    waiters: Vec<Waiter>,
}

/// The host names a trust check looks the host key up under: the key's host lowercased, as the pool keys it, and the
/// host as the open's URL wrote it, as a new connection's check does (TR2.18).
fn trust_names(request: &OpenRequest) -> Vec<String> {
    let lowered = request.key.host.to_ascii_lowercase();
    let mut names = vec![lowered.clone()];
    if let Some(url) = &request.url
        && url.host() != lowered
        && url.host().eq_ignore_ascii_case(&lowered)
    {
        names.push(url.host().to_owned());
    }
    names
}

/// The latest deadline among `waiters`, or none when one of them has none.
fn latest(waiters: &[Waiter]) -> Option<u64> {
    waiters
        .iter()
        .map(|waiter| waiter.request.deadline)
        .try_fold(0, |latest, deadline| deadline.map(|at| latest.max(at)))
}

/// Starts the proof `shared` describes for its waiters, under the latest of their deadlines and the stall of the
/// open that asks first, as an authentication sign wait runs.
fn start_proof(
    evidence: &Arc<Evidence>,
    names: Vec<String>,
    key: &Option<Vec<u8>>,
    host: &Option<(String, u16, Vec<u8>)>,
    waiters: &[Waiter],
) -> io::Result<(Job<Verdict>, Option<u64>)> {
    let deadline = latest(waiters);
    let stall = waiters.first().map_or(0, |waiter| {
        waiter.request.context.deadlines.io_ms.max(0) as u64
    });
    let check = Check {
        possession: key.clone(),
        trust: host
            .as_ref()
            .map(|(_, port, host_key)| (names, *port, host_key.clone())),
    };
    Ok((evidence.authority.start(check, deadline, stall)?, deadline))
}

/// Decides what `lease` needs before it serves `request`, whose binding is its bridge's session (contract §7.1; reuse
/// design §4). A lease the same binding last used, or a connection set up for this binding, serves at once; one
/// another binding last used serves only after this binding has proved the connection's authority now, so it joins
/// or starts the binding's proof. A connection that keeps no evidence (a password authenticated it) goes back to idle
/// for the binding that used it, and the open asks again for a new connection.
fn screen<C>(
    host: &mut PoolHost<C>,
    proofs: &mut Proofs,
    proving: &mut Vec<SharedProof>,
    lease: Lease,
    request: OpenRequest,
    policy: gwz_transport::pool::Request,
) -> Screened
where
    C: Connector,
    C::Resource: ChannelResource,
{
    let binding = request.context.session_id.clone();
    // A connection found closed while idle is the attach path's to settle.
    if host.lost(&lease).is_some() {
        return Screened::Attach(lease, request, policy);
    }
    let Ok(lessee) = host.lessee(&lease, &binding) else {
        return Screened::Attach(lease, request, policy);
    };
    let evidence = host.evidence(&lease);
    let key = request.key.clone();
    match lessee {
        Lessee::Same => return Screened::Attach(lease, request, policy),
        Lessee::First => {
            host.record_lessee(&lease, &binding);
            // The setup made for this binding's open is its proof of the key and host key it accepted.
            if let Some(evidence) = &evidence {
                if let Authenticator::Agent(accepted) = &evidence.authenticator {
                    proofs.add_key(&binding, accepted);
                }
                proofs.add_host(&binding, &key.host, key.port, &evidence.host_key);
            }
            return Screened::Attach(lease, request, policy);
        }
        Lessee::Other => {}
    }
    let Some(evidence) = evidence else {
        let bounced = lease.connection().ok();
        let _ = host.release_unused(lease);
        return Screened::Fresh(request, excluding(policy, bounced));
    };
    let possession = match &evidence.authenticator {
        Authenticator::Agent(accepted) => {
            (!proofs.has_key(&binding, accepted)).then(|| accepted.clone())
        }
        // The selected key was read again before this open was admitted, and its token names those exact bytes: the
        // pool leases a connection only for an equal token, which this asserts (reuse design §4).
        Authenticator::KeyFile(token) => {
            let equal = request
                .authority
                .as_ref()
                .is_some_and(|entry| entry.identity() == Identity::Explicit(token.clone()));
            if !equal {
                let bounced = lease.connection().ok();
                let _ = host.release(lease, Disposition::Discarded);
                return Screened::Fresh(request, excluding(policy, bounced));
            }
            None
        }
    };
    let trust = (!proofs.has_host(&binding, &key.host, key.port, &evidence.host_key))
        .then(|| (key.host.clone(), key.port, evidence.host_key.clone()));
    if possession.is_none() && trust.is_none() {
        host.record_lessee(&lease, &binding);
        return Screened::Attach(lease, request, policy);
    }
    let waiter = Waiter {
        lease,
        request,
        policy,
    };
    if let Some(shared) = proving.iter_mut().find(|shared| {
        shared.binding == binding && shared.key == possession && shared.host == trust
    }) {
        shared.waiters.push(waiter);
        return Screened::Waiting;
    }
    let waiters = vec![waiter];
    match start_proof(
        &evidence,
        trust_names(&waiters[0].request),
        &possession,
        &trust,
        &waiters,
    ) {
        Ok((job, deadline)) => {
            proving.push(SharedProof {
                binding,
                key: possession,
                host: trust,
                evidence,
                job,
                deadline,
                waiters,
            });
            Screened::Waiting
        }
        Err(error) => {
            // No proof job can start (a full job budget is WouldBlock): the connection goes back to idle and the open
            // fails as a setup that cannot start does, with Capacity (reuse design §4's table).
            for waiter in waiters {
                let _ = host.release_unused(waiter.lease);
                waiter.request.complete(Err(io::Error::from(error.kind())));
            }
            Screened::Waiting
        }
    }
}

/// Polls the binding proofs, and returns what each finished one's waiters do next (reuse design §4's table): a passed
/// proof is recorded for its binding and its leases serve; a failed one retires the connection and every idle
/// connection with the same key (possession) or the same host and host key (trust), and its opens ask again for a
/// new connection; a key no binding can prove sends its leases back to idle and its opens to a new connection. A
/// waiter cancelled or past its deadline gives its connection back to idle and fails alone.
fn settle_proofs<C>(
    host: &mut PoolHost<C>,
    proofs: &mut Proofs,
    proving: &mut Vec<SharedProof>,
    cx: &mut Context<'_>,
    now: u64,
) -> Vec<Screened>
where
    C: Connector,
    C::Resource: ChannelResource,
{
    let mut next = Vec::new();
    let mut index = 0;
    while index < proving.len() {
        let shared = &mut proving[index];
        let mut kept = Vec::with_capacity(shared.waiters.len());
        for waiter in shared.waiters.drain(..) {
            if waiter.request.expired(now) {
                let _ = host.release_unused(waiter.lease);
                waiter.request.complete_expired(io::ErrorKind::TimedOut);
            } else {
                kept.push(waiter);
            }
        }
        shared.waiters = kept;
        if shared.waiters.is_empty() {
            // Nobody waits now, but the proof is the binding's (contract section 7.1: only the binding's end cancels
            // it): the job runs on under the deadline it started with, and a proof that passes is recorded for the
            // binding, so its next lease asks nothing (reuse design section 4; step 1.6 remediation, C-P3-5).
            let Poll::Ready(result) = shared.job.poll_result(cx) else {
                index += 1;
                continue;
            };
            let shared = proving.swap_remove(index);
            if let Ok(verdict) = result
                && passed(&verdict)
            {
                record(proofs, &shared);
            }
            continue;
        }
        let verdict = match shared.job.poll_result(cx) {
            Poll::Pending => {
                index += 1;
                continue;
            }
            Poll::Ready(result) => result,
        };
        let shared = proving.swap_remove(index);
        let verdict = match verdict {
            Ok(verdict) => verdict,
            Err(error) => {
                // The job ended on its own clocks or was cancelled. Waiters with time left past the job's deadline
                // get a job of their own; the others fail as their deadline or cancellation says.
                let restart = job::timeout_reason(&error) == Some(job::TimeoutReason::Aggregate)
                    && shared.deadline.is_some_and(|ended| {
                        shared
                            .waiters
                            .iter()
                            .any(|waiter| waiter.request.deadline.is_none_or(|at| at > ended))
                    });
                if restart {
                    let names = trust_names(&shared.waiters[0].request);
                    match start_proof(
                        &shared.evidence,
                        names,
                        &shared.key,
                        &shared.host,
                        &shared.waiters,
                    ) {
                        Ok((job, deadline)) => {
                            proving.push(SharedProof {
                                job,
                                deadline,
                                ..shared
                            });
                            continue;
                        }
                        Err(error) => fail_waiters(host, shared.waiters, error.kind()),
                    }
                } else {
                    fail_waiters(host, shared.waiters, error.kind());
                }
                continue;
            }
        };
        let refused = verdict.possession == Some(Possession::Refused);
        let untrusted = verdict.trusted == Some(false);
        let unprovable = verdict.possession == Some(Possession::Unprovable);
        if refused || untrusted {
            let key = shared.key.clone();
            let trust = shared.host.clone();
            for waiter in &shared.waiters {
                host.retire_idle(&waiter.lease, |pool_key, resource| {
                    resource.reuse_evidence().is_some_and(|evidence| {
                        let same_key = refused
                            && key.as_ref().is_some_and(|key| {
                                evidence.authenticator == Authenticator::Agent(key.clone())
                            });
                        let same_host = untrusted
                            && trust.as_ref().is_some_and(|(name, port, host_key)| {
                                pool_key.host == *name
                                    && pool_key.port == *port
                                    && evidence.host_key == *host_key
                            });
                        same_key || same_host
                    })
                });
            }
            for waiter in shared.waiters {
                let _ = host.release(waiter.lease, Disposition::Discarded);
                next.push(Screened::Denied(waiter.request, waiter.policy));
            }
            continue;
        }
        if unprovable {
            for waiter in shared.waiters {
                let bounced = waiter.lease.connection().ok();
                let _ = host.release_unused(waiter.lease);
                next.push(Screened::Fresh(
                    waiter.request,
                    excluding(waiter.policy, bounced),
                ));
            }
            continue;
        }
        record(proofs, &shared);
        for waiter in shared.waiters {
            host.record_lessee(&waiter.lease, &shared.binding);
            next.push(Screened::Attach(
                waiter.lease,
                waiter.request,
                waiter.policy,
            ));
        }
    }
    next
}

/// Ends `waiters` whose proof could not finish: each connection back to idle, each open failed with `kind`.
fn fail_waiters<C>(host: &mut PoolHost<C>, waiters: Vec<Waiter>, kind: io::ErrorKind)
where
    C: Connector,
{
    for waiter in waiters {
        let _ = host.release_unused(waiter.lease);
        waiter.request.complete(Err(kind.into()));
    }
}

/// Whether a proof's verdict lets its binding reuse: every part asked passed.
fn passed(verdict: &Verdict) -> bool {
    verdict
        .possession
        .is_none_or(|possession| possession == Possession::Proven)
        && verdict.trusted.is_none_or(|trusted| trusted)
}

/// Records a passed proof for its binding.
fn record(proofs: &mut Proofs, shared: &SharedProof) {
    if let Some(key) = &shared.key {
        proofs.add_key(&shared.binding, key);
    }
    if let Some((name, port, host_key)) = &shared.host {
        proofs.add_host(&shared.binding, name, *port, host_key);
    }
}

/// `policy` asking again with the connection `bounced` excluded (step 1.6 remediation, S-P2-2): the next compatible
/// idle connection, the binding's own among them, or a new one, is offered instead of the one that could not serve it.
fn excluding(
    mut policy: gwz_transport::pool::Request,
    bounced: Option<gwz_transport::pool::ConnectionId>,
) -> gwz_transport::pool::Request {
    if let Some(bounced) = bounced
        && !policy.exclude.contains(&bounced)
    {
        policy.exclude.push(bounced);
    }
    policy
}
