use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};




/// A counting semaphore over a fixed permit budget; used as the global ceiling.
pub(crate) struct Semaphore {
    pub(crate) permits: Mutex<usize>,
    pub(crate) available: Condvar,
}

impl Semaphore {
    pub(crate) fn new(permits: usize) -> Self {
        Self {
            permits: Mutex::new(permits.max(1)),
            available: Condvar::new(),
        }
    }

    pub(crate) fn acquire(&self) {
        let mut permits = self.permits.lock().expect("semaphore poisoned");
        while *permits == 0 {
            permits = self.available.wait(permits).expect("semaphore poisoned");
        }
        *permits -= 1;
    }

    pub(crate) fn release(&self) {
        *self.permits.lock().expect("semaphore poisoned") += 1;
        self.available.notify_one();
    }
}

/// Applies `f` to each item across scoped worker threads, bounding concurrency
/// both globally (`global_limit`, the `--jobs` ceiling) and per host
/// (`per_host_limit`). Items are keyed by `host_of`; `None` means no parseable
/// host (local) and is bounded only by the global ceiling. Different hosts run
/// concurrently, and results preserve input order. `f` runs on workers, so
/// anything it borrows (sink, backend, workspace root) must be `Sync`.
pub fn par_map_per_host<T, R, K, F>(
    items: Vec<T>,
    global_limit: usize,
    per_host_limit: usize,
    host_of: K,
    f: F,
) -> Vec<R>
where
    T: Send,
    R: Send,
    K: Fn(&T) -> Option<String>,
    F: Fn(T) -> R + Sync,
{
    let count = items.len();
    if count == 0 {
        return Vec::new();
    }
    // Group input indices by host (None = local).
    let mut groups: HashMap<Option<String>, Vec<usize>> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        groups.entry(host_of(item)).or_default().push(index);
    }
    let group_list: Vec<(Option<String>, Vec<usize>)> = groups.into_iter().collect();
    let cursors: Vec<AtomicUsize> = (0..group_list.len()).map(|_| AtomicUsize::new(0)).collect();
    let slots: Vec<Mutex<Option<T>>> = items
        .into_iter()
        .map(|item| Mutex::new(Some(item)))
        .collect();
    let results: Vec<Mutex<Option<R>>> = (0..count).map(|_| Mutex::new(None)).collect();
    let global = Semaphore::new(global_limit);
    let f = &f;

    std::thread::scope(|scope| {
        for (group_index, (host, indices)) in group_list.iter().enumerate() {
            // A hosted group runs at most `per_host_limit` at once; local items
            // are bounded only by the global ceiling.
            let group_limit = if host.is_some() {
                per_host_limit.max(1)
            } else {
                global_limit.max(1)
            };
            for _ in 0..group_limit.min(indices.len()) {
                let cursor = &cursors[group_index];
                let indices = indices.as_slice();
                let slots = &slots;
                let results = &results;
                let global = &global;
                scope.spawn(move || {
                    loop {
                        let position = cursor.fetch_add(1, Ordering::Relaxed);
                        if position >= indices.len() {
                            break;
                        }
                        let index = indices[position];
                        let item = slots[index]
                            .lock()
                            .expect("par_map slot poisoned")
                            .take()
                            .expect("each item is taken once");
                        global.acquire();
                        let result = f(item);
                        global.release();
                        *results[index].lock().expect("par_map result poisoned") = Some(result);
                    }
                });
            }
        }
    });

    results
        .into_iter()
        .map(|cell| {
            cell.into_inner()
                .expect("par_map result poisoned")
                .expect("every index produces a result")
        })
        .collect()
}

