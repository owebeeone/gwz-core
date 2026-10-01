//! The in-process carrier between a driver session's port and an endpoint
//! session's port. Each direction moves every message its source has ready,
//! in order, and parks only when neither mux has anything for it.
use super::*;

pub(in crate::transport_host) struct LocalLink {
    stop: Arc<AtomicBool>,
    thread: thread::Thread,
}
impl LocalLink {
    pub(in crate::transport_host) fn new(
        core: TransportPort,
        peer: TransportPort,
    ) -> ModelResult<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let link = thread::Builder::new()
            .name("gwz-placement-local".into())
            .spawn(move || {
                // Each lane's waiting futures wake this thread when either mux
                // changes; the timeout only covers a wake that never comes.
                let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
                let mut cx = Context::from_waker(&waker);
                let mut lanes = [Lane::new(&core, &peer), Lane::new(&peer, &core)];
                while !stopped.load(Ordering::Acquire) {
                    let mut moved = false;
                    for lane in &mut lanes {
                        match lane.pump(&mut cx) {
                            Ok(progress) => moved |= progress,
                            Err(()) => {
                                stopped.store(true, Ordering::Release);
                                break;
                            }
                        }
                    }
                    if !moved && !stopped.load(Ordering::Acquire) {
                        thread::park_timeout(Duration::from_millis(5));
                    }
                }
                drop(lanes);
                core.disconnect();
                peer.disconnect();
            })
            .map_err(|_| unavailable("local forwarding unavailable"))?;
        Ok(Self {
            stop,
            thread: link.thread().clone(),
        })
    }
    pub(in crate::transport_host) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.thread.unpark();
    }
}
/// One direction of the local carrier. The message being taken and the one
/// being delivered stay pending across passes, so that the change that lets
/// either finish wakes the link.
struct Lane {
    from: TransportPort,
    to: TransportPort,
    take: Option<Pin<Box<dyn Future<Output = ModelResult<Option<Attachment>>>>>>,
    give: Option<Pin<Box<dyn Future<Output = ModelResult<()>>>>>,
}
impl Lane {
    fn new(from: &TransportPort, to: &TransportPort) -> Self {
        Self {
            from: from.clone(),
            to: to.clone(),
            take: None,
            give: None,
        }
    }
    /// Moves every message `from` has ready into `to`, in order, up to a bound
    /// per pass; Err once either side has closed.
    fn pump(&mut self, cx: &mut Context<'_>) -> Result<bool, ()> {
        let mut moved = false;
        for _ in 0..64 {
            if let Some(give) = self.give.as_mut() {
                match give.as_mut().poll(cx) {
                    Poll::Ready(Ok(())) => {
                        self.give = None;
                        moved = true;
                    }
                    Poll::Ready(Err(_)) => return Err(()),
                    Poll::Pending => return Ok(moved),
                }
            }
            let from = &self.from;
            let take = self.take.get_or_insert_with(|| {
                let from = from.clone();
                Box::pin(async move { from.next_message().await })
            });
            match take.as_mut().poll(cx) {
                Poll::Ready(Ok(Some(item))) => {
                    self.take = None;
                    let to = self.to.clone();
                    self.give = Some(Box::pin(async move { to.deliver(item).await }));
                }
                Poll::Ready(_) => return Err(()),
                Poll::Pending => return Ok(moved),
            }
        }
        Ok(moved)
    }
}
impl Drop for LocalLink {
    fn drop(&mut self) {
        self.stop();
    }
}
