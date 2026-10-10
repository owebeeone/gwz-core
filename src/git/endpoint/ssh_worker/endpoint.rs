use super::*;
impl Endpoint {
    /// The endpoint. A driver in its process deposits each open's URL extras
    /// in `handoff`, from which the open takes them (TR2.18).
    pub(crate) fn with_handoff<C>(
        config: PoolConfig,
        registry: Registry,
        factory: impl FnOnce(Instant, Registry) -> C,
        io_timeout_ms: u64,
        handoff: Handoff,
    ) -> io::Result<Self>
    where
        C: Connector + Send + 'static,
        C::Resource: ChannelResource,
    {
        let reader: Reader = Arc::new(Registry::start);
        Self::build(config, registry, reader, factory, io_timeout_ms, handoff)
    }
    pub(super) fn build<C>(
        config: PoolConfig,
        registry: Registry,
        reader: Reader,
        factory: impl FnOnce(Instant, Registry) -> C,
        io_timeout_ms: u64,
        handoff: Handoff,
    ) -> io::Result<Self>
    where
        C: Connector + Send + 'static,
        C::Resource: ChannelResource,
    {
        let supervisor = registry.supervisor();
        let retention = Cleanup::reserve(&supervisor)?;
        let origin = Instant::now();
        let connector = factory(origin, registry.clone());
        if io_timeout_ms > i32::MAX as u64 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let capacity = config.max_requests;
        let cleanup = config.cleanup_timeout_ms;
        let policy = config.clone();
        let ceiling = config.per_host.min(config.per_user_host);
        let (pool, mut host) = PoolHost::new(config, connector, 0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        // Every connection state change on this pool reaches the machines
        // from the one thread that drives the host, in the order it acted.
        let governor = Governor::random(pool.control(), ceiling, false);
        host.set_observer(Arc::new(governor.clone()));
        static NEXT_WORKER: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_WORKER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| io::Error::other("endpoint IDs exhausted"))?;
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let status = Status::default();
        let worker_status = status.clone();
        let watch = Watch::default();
        let worker_watch = watch.clone();
        let client_pool = pool.clone();
        let join = thread::Builder::new()
            .name("gwz-ssh-endpoint".into())
            .spawn(move || {
                let run_pool = pool.clone();
                let report = worker_status.clone();
                ssh_shutdown::manage(
                    host,
                    Admissions::new(registry, reader, origin, cleanup),
                    pool,
                    worker_status,
                    worker_watch,
                    retention,
                    origin,
                    |host, admissions| {
                        run(
                            receiver,
                            run_pool,
                            host,
                            admissions,
                            io_timeout_ms,
                            cleanup,
                            worker_stop,
                            move || elapsed(origin),
                            id,
                            &report,
                        );
                    },
                );
            })?;
        Ok(Self {
            shared: Arc::new(Shared {
                sender,
                worker: join.thread().clone(),
                outstanding: Arc::new(AtomicUsize::new(0)),
                capacity: AtomicUsize::new(capacity),
                pool: client_pool,
                policy,
                io_timeout_ms,
                stop,
                origin,
                join: Mutex::new(Some(join)),
                status,
                watch,
                handoff,
                supervisor,
                governor,
            }),
            cleanup: Duration::from_millis(cleanup),
        })
    }
    pub(crate) fn validate_deadlines(&self, d: &Deadlines) -> io::Result<()> {
        fn tightens(value: i64, configured: u64, zero_allowed: bool) -> bool {
            value >= 0
                && (value > 0 || zero_allowed && configured == 0)
                && (configured == 0 || value as u64 <= configured)
        }
        let p = &self.shared.policy;
        if !tightens(d.allocation_ms, p.allocation_timeout_ms, false)
            || !tightens(d.connect_ms, p.connect_timeout_ms, true)
            || !tightens(d.io_ms, self.shared.io_timeout_ms, true)
            || !tightens(d.interaction_ms, p.interaction_timeout_ms, false)
            || !tightens(d.cleanup_ms, p.cleanup_timeout_ms, false)
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if d.connect_ms != 0 {
            let total = (d.allocation_ms as u64)
                .checked_add(d.connect_ms as u64)
                .and_then(|value| value.checked_add(d.interaction_ms as u64))
                .ok_or(io::ErrorKind::InvalidInput)?;
            Instant::now()
                .checked_add(Duration::from_millis(total))
                .ok_or(io::ErrorKind::InvalidInput)?;
        }
        Ok(())
    }
    pub(crate) fn start_identity_file_check(
        &self,
        selected: PathBuf,
        deadline: Option<Instant>,
    ) -> io::Result<Job<()>> {
        cfg_if::cfg_if! { if #[cfg(unix)] {
        Job::start(&self.shared.supervisor, deadline, self.cleanup, move |control| {
            control.check()?;
            use std::os::unix::fs::OpenOptionsExt;
            // O_NONBLOCK prevents special files (including a replaced FIFO) from
            // blocking before fstat establishes the regular-file requirement.
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(selected)
                .map_err(|error| io::Error::from(error.kind()))?;
            control.check()?;
            let metadata = file
                .metadata()
                .map_err(|error| io::Error::from(error.kind()))?;
            if !metadata.file_type().is_file() {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            Ok(())
        })
        } else {
            let _ = (selected, deadline);
            Err(io::ErrorKind::Unsupported.into())
        } }
    }

    pub(crate) fn pending_requests(&self) -> usize {
        self.shared.outstanding.load(Ordering::Acquire)
    }
    pub(crate) fn pool(&self) -> &Pool {
        &self.shared.pool
    }
    /// The limit machines of this endpoint's pool.
    pub(crate) fn governor(&self) -> &Governor {
        &self.shared.governor
    }
    /// The pool's clock in milliseconds: the time the pool, its host and the
    /// governor are all measured in.
    pub(crate) fn pool_now(&self) -> u64 {
        elapsed(self.shared.origin)
    }
    pub(crate) fn set_request_capacity(&self, capacity: usize) {
        self.shared.capacity.store(capacity, Ordering::Release);
    }
    /// Submits a bridged open, with a selected key or the agent, and returns
    /// at once. No thread and no supervised job waits for it: the worker owns
    /// it until it replies, and its reply wakes `context.waker`. A selected
    /// identity file is admitted by the worker before every checkout,
    /// including reuse.
    pub(crate) fn start_endpoint_open(
        &self,
        key: Key,
        selected: Option<PathBuf>,
        service: GitService,
        path: &str,
        context: BridgeContext,
        cancelled: Arc<AtomicBool>,
    ) -> io::Result<PendingOpen> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(stopped());
        }
        if path.is_empty() || path.len() > 16_384 || path.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid repository operand",
            ));
        }
        self.shared
            .outstanding
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.shared.capacity.load(Ordering::Acquire)).then_some(n + 1)
            })
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "endpoint admission full"))?;
        let permit = Permit(self.shared.outstanding.clone());
        let (reply, result) = mpsc::sync_channel(1);
        self.validate_deadlines(&context.deadlines)?;
        let d = &context.deadlines;
        let timeout = (d.connect_ms != 0).then(|| {
            Duration::from_millis(
                (d.allocation_ms as u64)
                    .saturating_add(d.connect_ms as u64)
                    .saturating_add(d.interaction_ms as u64),
            )
        });
        let absolute = timeout
            .map(|duration| {
                Instant::now()
                    .checked_add(duration)
                    .ok_or(io::ErrorKind::InvalidInput)
            })
            .transpose()?;
        // What the URL holds beyond the destination, which the driver
        // deposited before this open could arrive: taken once, here.
        let url = self
            .shared
            .handoff
            .take(&context.session_id, context.stream_id)
            .map(Arc::new);
        let request = OpenRequest {
            key,
            identity: Identity::Ambient,
            service,
            path: path.to_owned(),
            permit,
            reply: Some(reply),
            selected,
            authority: None,
            url,
            progress: Progress::default(),
            setup_slot: Arc::default(),
            cancelled,
            deadline: absolute.map(|at| at.duration_since(self.shared.origin).as_millis() as u64),
            context,
        };
        if self.shared.sender.send(request).is_err() {
            return Err(stopped());
        }
        self.shared.worker.unpark();
        Ok(PendingOpen { reply: result })
    }
    /// Has `waker` woken when the worker has ended and its shutdown status
    /// has settled, as a watcher with work of its own to do after (a placement
    /// session that waits for the worker's cleanup to close).
    pub(crate) fn watch_shutdown(&self, waker: &Waker) {
        self.shared.watch.register(waker);
    }
    pub(crate) fn shutdown_status(&self) -> ShutdownStatus {
        *self.shared.status.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Cancels pending/active work through every clone, even with timeouts disabled.
    pub(crate) fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.worker.unpark();
    }
}
