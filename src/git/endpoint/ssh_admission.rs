//! File-admission owners survive worker unwind beside the physical pool.
use super::{
    agent_job::Job,
    ssh_key_snapshot::{Loaded, Registry},
    ssh_worker::OpenRequest,
};
use gwz_transport::pool::Key;
use std::{
    collections::VecDeque,
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
    /// Requests whose key read waits for a free reservation or supervised job,
    /// in arrival order. Their own deadlines bound the wait.
    waiting: VecDeque<OpenRequest>,
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
            waiting: VecDeque::new(),
            cursor: 0,
            failure: None,
        }
    }
    pub(super) fn start(&mut self, request: OpenRequest) {
        self.waiting.push_back(request);
        self.start_waiting();
    }
    /// Starts the waiting key reads, oldest first, until one finds the key
    /// registry's reservations or the supervised jobs all taken: a full budget
    /// is backpressure, never the request's failure.
    fn start_waiting(&mut self) {
        while let Some(mut request) = self.waiting.pop_front() {
            let path = request.selected.take().expect("selected plan");
            let deadline = request
                .deadline
                .map(|at| self.origin + Duration::from_millis(at));
            match (self.reader)(
                &self.registry,
                request.key.clone(),
                path.clone(),
                deadline,
                self.cleanup,
            ) {
                Ok(job) => self.items.push(Pending { request, job }),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    request.selected = Some(path);
                    self.waiting.push_front(request);
                    break;
                }
                Err(error) => request.complete(Err(error.kind().into())),
            }
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.items.len() + self.waiting.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty() && self.waiting.is_empty()
    }
    pub(crate) fn take_failure(&mut self) -> Option<io::ErrorKind> {
        self.failure.take()
    }
    pub(super) fn poll(
        &mut self,
        cx: &mut Context<'_>,
        now: u64,
        stopping: bool,
    ) -> Vec<OpenRequest> {
        let now = now.max(self.origin.elapsed().as_millis() as u64);
        let mut index = 0;
        while index < self.waiting.len() {
            let request = &mut self.waiting[index];
            if stopping || request.expired(now) || !request.has_reply() {
                request.reject(if stopping {
                    io::ErrorKind::BrokenPipe
                } else {
                    io::ErrorKind::TimedOut
                });
                self.waiting.remove(index);
            } else {
                index += 1;
            }
        }
        if !stopping {
            self.start_waiting();
        }
        let mut ready = Vec::new();
        for _ in 0..self.items.len().min(32) {
            if self.items.is_empty() {
                break;
            }
            self.cursor %= self.items.len();
            let item = &mut self.items[self.cursor];
            let now = now.max(self.origin.elapsed().as_millis() as u64);
            if stopping || item.request.expired(now) || !item.request.has_reply() {
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
