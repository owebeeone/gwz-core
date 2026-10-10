//! Process and retained-child ownership for one endpoint.
use super::*;

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

/// A bounded file read that outlived the lookup that started it. The blocking read owns its buffer and both
/// admission permits until it returns, so this registry keeps its handle.
pub(super) struct PendingWorker {
    pub(super) handle: JoinHandle<Result<SecretBuffer, AuthError>>,
    pub(super) _permits: Arc<AdmissionPermits>,
}

pub(super) struct PendingChild {
    pub(super) child: HelperChild,
    /// The helper's process tree. A helper is retired only when its whole tree is gone, so the tree stays here
    /// until `drained` says so.
    pub(super) tree: Option<ProcessTree>,
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
    pub(super) workers: Mutex<Vec<PendingWorker>>,
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
    /// A helper is reaped when its leader has been waited for and its whole
    /// process tree is gone.
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
            let pending = &mut batch.children[index];
            let reaped = tokio::time::timeout_at(deadline, async {
                let status = pending.child.wait().await;
                if let Some(tree) = &pending.tree {
                    confirm_drained(tree, Some(deadline)).await;
                }
                status
            })
            .await;
            let drained = batch.children[index]
                .tree
                .as_ref()
                .is_none_or(ProcessTree::drained);
            if matches!(reaped, Ok(Ok(_))) && drained {
                batch.children.swap_remove(index);
                self.inner.reaping.fetch_sub(1, Ordering::AcqRel);
            } else {
                index += 1;
            }
        }
        drop(batch);
        reap_ready_workers(self);
        self.pending_cleanup_count()
    }

    pub(super) fn reap_ready(&self) {
        reap_ready_workers(self);
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain_mut(|pending| {
                if !matches!(pending.child.try_wait(), Ok(Some(_))) {
                    return true;
                }
                // The leader is gone. A retained tree that still has a member (one that was created as the job
                // ended) is ended again, so that an abandoned lookup cannot hold a host slot for good.
                match &pending.tree {
                    Some(tree) if !tree.drained() => {
                        tree.kill();
                        true
                    }
                    _ => false,
                }
            });
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
    child: Option<HelperChild>,
    permits: Option<Arc<AdmissionPermits>>,
    owner: AuthOwner,
    tree: Option<ProcessTree>,
}

impl HelperJob {
    pub(super) fn new(
        (child, tree): (HelperChild, ProcessTree),
        permits: Arc<AdmissionPermits>,
        owner: AuthOwner,
    ) -> Self {
        Self {
            tree: Some(tree),
            child: Some(child),
            permits: Some(permits),
            owner,
        }
    }

    pub(super) fn child_mut(&mut self) -> &mut HelperChild {
        self.child.as_mut().expect("active helper child")
    }

    fn complete(&mut self) {
        self.tree = None;
        drop(self.child.take());
        drop(self.permits.take());
    }

    /// A helper that succeeded is retired once its leader has exited: the owner lets go of the tree, and then of
    /// the helper's admission permits. What the helper left running is left running (as under Git, on Unix and
    /// in 1.0.17), and holds no handle of gwz's. A leader still running stays owned, and dropping the job retains
    /// it for the owner to reap.
    pub(super) fn retire(&mut self) {
        if !self
            .child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_))))
        {
            return;
        }
        if let Some(tree) = &self.tree {
            tree.retire();
        }
        self.complete();
    }

    pub(super) async fn terminate(&mut self) -> Result<(), AuthError> {
        if let Some(tree) = &self.tree {
            tree.kill();
        }
        let result = {
            let tree = self.tree.as_ref();
            let child = self.child.as_mut().expect("active helper child");
            let _ = child.start_kill();
            timeout(CLEANUP_GRACE, async {
                let status = child.wait().await;
                if let Some(tree) = tree {
                    confirm_drained(tree, None).await;
                }
                status
            })
            .await
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
        let tree = self.tree.take();
        if let Some(tree) = &tree {
            tree.kill();
        }
        if matches!(child.try_wait(), Ok(Some(_))) && tree.as_ref().is_none_or(ProcessTree::drained)
        {
            drop(permits);
            return;
        }
        let _ = child.start_kill();
        self.owner.retain_pending(PendingChild {
            child,
            tree,
            _permits: permits,
        });
    }
}

/// Waits until `tree` has no member, ending any that appears meanwhile, or until `deadline`. Callers that
/// need a bound also check [`ProcessTree::drained`] afterwards.
async fn confirm_drained(tree: &ProcessTree, deadline: Option<Instant>) {
    while !tree.drained() {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return;
        }
        tree.kill();
        sleep(DRAIN_POLL).await;
    }
}

/// Finished blocking workers own no open file. Dropping their completed
/// join handle disposes the zeroizing result without blocking this thread.
fn reap_ready_workers(owner: &AuthOwner) {
    owner
        .inner
        .workers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|pending| !pending.handle.is_finished());
}
