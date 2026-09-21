//! File-admission owners survive worker unwind beside the physical pool.
use super::{
    agent_job::Job,
    ssh_key_snapshot::{Loaded, Registry},
    ssh_worker::OpenRequest,
};
use gwz_transport::pool::Key;
use std::{
    io,
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
pub(crate) type Reader = Arc<
    dyn Fn(&Registry, Key, PathBuf, Option<Instant>, Duration) -> io::Result<Job<Loaded>>
        + Send
        + Sync,
>;
struct Pending {
    request: OpenRequest,
    job: Job<Loaded>,
}
pub(crate) struct Admissions {
    registry: Registry,
    reader: Reader,
    origin: Instant,
    cleanup: Duration,
    items: Vec<Pending>,
    cursor: usize,
    failure: Option<io::ErrorKind>,
}
impl Admissions {
    pub(crate) fn new(
        registry: Registry,
        reader: Reader,
        origin: Instant,
        cleanup_ms: u64,
    ) -> Self {
        Self {
            registry,
            reader,
            origin,
            cleanup: Duration::from_millis(cleanup_ms),
            items: Vec::new(),
            cursor: 0,
            failure: None,
        }
    }
    pub(crate) fn start(&mut self, mut request: OpenRequest) {
        let path = request.selected.take().expect("selected plan");
        let deadline = request
            .deadline
            .map(|at| self.origin + Duration::from_millis(at));
        match (self.reader)(
            &self.registry,
            request.key.clone(),
            path,
            deadline,
            self.cleanup,
        ) {
            Ok(job) => self.items.push(Pending { request, job }),
            Err(error) => request.complete(Err(error.kind().into())),
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub(crate) fn take_failure(&mut self) -> Option<io::ErrorKind> {
        self.failure.take()
    }
    pub(crate) fn poll(
        &mut self,
        cx: &mut Context<'_>,
        now: u64,
        stopping: bool,
    ) -> Vec<OpenRequest> {
        let mut ready = Vec::new();
        for _ in 0..self.items.len().min(32) {
            if self.items.is_empty() {
                break;
            }
            self.cursor %= self.items.len();
            let item = &mut self.items[self.cursor];
            let now = now.max(self.origin.elapsed().as_millis() as u64);
            if stopping || item.request.expired(now) || item.request.reply.is_none() {
                item.request.reject(if stopping {
                    io::ErrorKind::BrokenPipe
                } else {
                    io::ErrorKind::TimedOut
                });
                match item.job.poll_disposed(cx) {
                    Poll::Ready(Ok(())) => {
                        self.items.swap_remove(self.cursor);
                        continue;
                    }
                    Poll::Ready(Err(error)) => {
                        self.failure.get_or_insert(error.kind());
                    }
                    Poll::Pending => {}
                }
            } else if let Poll::Ready(result) = item.job.poll_result(cx) {
                let admitted = result.and_then(|loaded| {
                    self.registry.intern(loaded, || {
                        if item
                            .request
                            .expired(self.origin.elapsed().as_millis() as u64)
                        {
                            Err(io::ErrorKind::TimedOut.into())
                        } else {
                            Ok(())
                        }
                    })
                });
                let mut item = self.items.swap_remove(self.cursor);
                match admitted {
                    Ok(entry) => {
                        item.request.identity = entry.identity();
                        item.request.authority = Some(entry);
                        ready.push(item.request);
                    }
                    Err(error) => item.request.complete(Err(error.kind().into())),
                }
                continue;
            }
            self.cursor += 1;
        }
        ready
    }
}
