//! Process and retained-child ownership for one endpoint.
use super::*;

/// One host context's HTTPS helper slots, shared by every endpoint its driver
/// opens (GwzCoreSessionDesign §5.6): the driver creates them once and hands
/// them to each endpoint's `AuthOwner`. A standalone endpoint is its own host.
#[derive(Clone)]
pub(crate) struct HelperSlots(pub(super) Arc<Semaphore>);

impl HelperSlots {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Semaphore::new(HELPER_SLOTS)))
    }
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl HelperSlots {
            pub(crate) async fn hold_all(&self) -> OwnedSemaphorePermit { self.0.clone().acquire_many_owned(8).await.unwrap() }
            pub(crate) fn available(&self) -> usize { self.0.available_permits() }
        }
    }
}

pub(super) struct AdmissionPermits {
    pub(super) _helper_slot: OwnedSemaphorePermit,
    pub(super) _endpoint_slot: Option<OwnedSemaphorePermit>,
}

pub(super) struct PendingChild {
    pub(super) child: Child,
    /// Held for its drop: the host's helper slot stays charged until the
    /// child is reaped or its owner is gone.
    pub(super) _permits: Arc<AdmissionPermits>,
}

pub(super) struct AuthOwnerInner {
    pub(super) cancelled: CancellationToken,
    pub(super) active: Arc<AtomicUsize>,
    pub(super) reaping: AtomicUsize,
    pub(super) pending: Mutex<Vec<PendingChild>>,
    pub(super) helper_slots: HelperSlots,
    pub(super) workers: Mutex<Vec<super::file_worker::PendingWorker>>,
}

/// Endpoint-scoped ownership for credential helper processes.
///
/// Helper admission comes from the host's `HelperSlots`, so another host's
/// live or retained helpers never exhaust it. Retained children and their
/// admission permits live in this owner only, until `reap_pending` joins
/// them; a child is killed before it is retained. An owner dropped with
/// children still retained drops them with it, which releases their slots
/// and leaves the killed children to tokio's background reaping.
#[derive(Clone)]
pub(crate) struct AuthOwner {
    pub(super) inner: Arc<AuthOwnerInner>,
}

impl AuthOwner {
    pub(crate) fn new(helper_slots: HelperSlots) -> Self {
        Self {
            inner: Arc::new(AuthOwnerInner {
                cancelled: CancellationToken::new(),
                active: Arc::new(AtomicUsize::new(0)),
                reaping: AtomicUsize::new(0),
                pending: Mutex::new(Vec::new()),
                workers: Mutex::new(Vec::new()),
                helper_slots,
            }),
        }
    }

    /// Cancel all owned active helper lookups. Retained children remain in the
    /// owner registry until `reap_pending` joins them or the owner is dropped.
    pub(crate) fn cancel(&self) {
        self.inner.cancelled.cancel();
    }

    pub(crate) fn pending_cleanup_count(&self) -> usize {
        let pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.len()
            + self.inner.reaping.load(Ordering::Acquire)
            + self
                .inner
                .workers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len()
    }

    /// Reaping owns a guarded batch, including while this future is cancelled.
    /// Concurrent transfers into the owner remain visible in the returned count.
    pub(crate) async fn reap_pending(&self, deadline: Instant) -> usize {
        let children = {
            let mut pending = self
                .inner
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let children = std::mem::take(&mut *pending);
            self.inner
                .reaping
                .fetch_add(children.len(), Ordering::AcqRel);
            children
        };
        let mut batch = ReapBatch {
            owner: self.clone(),
            children,
        };
        let mut index = 0;
        while index < batch.children.len() && Instant::now() < deadline {
            let result =
                tokio::time::timeout_at(deadline, batch.children[index].child.wait()).await;
            if matches!(result, Ok(Ok(_))) {
                batch.children.swap_remove(index);
                self.inner.reaping.fetch_sub(1, Ordering::AcqRel);
            } else {
                index += 1;
            }
        }
        drop(batch);
        super::file_worker::reap_ready(self);
        self.pending_cleanup_count()
    }

    pub(super) fn reap_ready(&self) {
        super::file_worker::reap_ready(self);
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain_mut(|pending| !matches!(pending.child.try_wait(), Ok(Some(_))));
    }

    pub(super) fn retain_pending(&self, pending: PendingChild) {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(pending);
    }
}

/// Unreaped children always keep their permits and their original owner.
struct ReapBatch {
    owner: AuthOwner,
    children: Vec<PendingChild>,
}
impl Drop for ReapBatch {
    fn drop(&mut self) {
        let count = self.children.len();
        let mut pending = self
            .owner
            .inner
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for mut child in self.children.drain(..) {
            let _ = child.child.start_kill();
            pending.push(child);
        }
        self.owner.inner.reaping.fetch_sub(count, Ordering::AcqRel);
    }
}

pub(super) struct ActiveGuard {
    pub(super) active: Arc<AtomicUsize>,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct HelperJob {
    child: Option<Child>,
    permits: Option<Arc<AdmissionPermits>>,
    owner: AuthOwner,
    process_group: Option<u32>,
}

impl HelperJob {
    pub(super) fn new(child: Child, permits: Arc<AdmissionPermits>, owner: AuthOwner) -> Self {
        let process_group = child.id();
        Self {
            process_group,
            child: Some(child),
            permits: Some(permits),
            owner,
        }
    }

    pub(super) fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("active helper child")
    }

    fn complete(&mut self) {
        self.process_group = None;
        drop(self.child.take());
        drop(self.permits.take());
    }

    pub(super) fn complete_if_exited(&mut self) {
        if self
            .child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_))))
        {
            self.complete();
        }
    }

    pub(super) async fn terminate(&mut self) -> Result<(), AuthError> {
        kill_process_group(self.process_group);
        let result = {
            let child = self.child_mut();
            let _ = child.start_kill();
            timeout(CLEANUP_GRACE, child.wait()).await
        };
        match result {
            Ok(Ok(_)) => {
                self.complete();
                Ok(())
            }
            Ok(Err(_)) | Err(_) => Err(AuthError::CleanupPending),
        }
    }
}

impl Drop for HelperJob {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let Some(permits) = self.permits.take() else {
            return;
        };
        kill_process_group(self.process_group);
        if matches!(child.try_wait(), Ok(Some(_))) {
            drop(permits);
            return;
        }
        let _ = child.start_kill();
        self.owner.retain_pending(PendingChild {
            child,
            _permits: permits,
        });
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        fn kill_process_group(group: Option<u32>) {
            if let Some(group) = group {
                // SAFETY: Command placed this child in its own group. The captured
                // positive PID names that group, never the caller's process group.
                unsafe { libc::kill(-(group as libc::pid_t), libc::SIGKILL); }
            }
        }
    } else {
        fn kill_process_group(_group: Option<u32>) {}
    }
}
