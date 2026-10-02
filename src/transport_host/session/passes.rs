use super::*;

impl Session {
    pub(super) fn start(state: State) -> ModelResult<Arc<Self>> {
        let session = Arc::new(Self {
            state: Mutex::new(state),
            event: Event::new(),
            origin: Instant::now(),
            capacity_gate: AtomicBool::new(false),
            admission_gate: AtomicBool::new(false),
            test_hooks: TestHooks::new(),
            passes: OnceLock::new(),
        });
        let weak = Arc::downgrade(&session);
        let passes = thread::Builder::new()
            .name("gwz-placement".into())
            .spawn(move || {
                // Whatever gives a pass work wakes this thread. The timeout
                // only runs the deadlines when nothing does.
                let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
                while let Some(session) = weak.upgrade() {
                    let moved = session.drive(&waker);
                    let done = {
                        let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
                        state.closed
                            && state.engine.as_ref().is_none_or(|e| e.pending() == 0)
                            && state.https.as_ref().is_none_or(|e| e.pending() == 0)
                    };
                    session.event.signal();
                    drop(session);
                    if done {
                        break;
                    }
                    if !moved {
                        thread::park_timeout(Duration::from_millis(5));
                    }
                }
            })
            .map_err(|_| unavailable("transport supervisor unavailable"))?;
        let _ = session.passes.set(passes.thread().clone());
        Ok(session)
    }
    /// Runs a pass now: the carrier moved a message into or out of its mux.
    pub(super) fn wake(&self) {
        if let Some(thread) = self.passes.get() {
            thread.unpark();
        }
    }
}
