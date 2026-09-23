use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex};

use crate::model::{ErrorCode, ModelError, ModelResult};

struct Group {
    indices: VecDeque<usize>,
    active: usize,
    limit: usize,
}

struct Queue {
    groups: Vec<Group>,
    unclaimed: usize,
    started: bool,
    failed: bool,
}

struct GroupPermit<'a> {
    state: &'a Mutex<Queue>,
    changed: &'a Condvar,
    group: usize,
}
impl Drop for GroupPermit<'_> {
    fn drop(&mut self) {
        let mut queue = self.state.lock().unwrap_or_else(|error| error.into_inner());
        queue.groups[self.group].active -= 1;
        self.changed.notify_all();
    }
}

/// Applies `f` to each item with at most `global_limit` live scoped workers.
/// A hosted group has at most `per_host_limit` active member operations;
/// hostless items use only the global ceiling. Results retain input order.
pub fn par_map_per_host<T, R, K, F>(
    items: Vec<T>,
    global_limit: usize,
    per_host_limit: usize,
    host_of: K,
    f: F,
) -> ModelResult<Vec<R>>
where
    T: Send,
    R: Send,
    K: Fn(&T) -> Option<String>,
    F: Fn(T) -> R + Sync,
{
    par_map_per_host_with_cancel(
        items,
        global_limit,
        per_host_limit,
        host_of,
        f,
        || false,
        |_| unreachable!("uncancelled scheduler"),
    )
}

/// A cancellation-aware scheduler. Queued items receive `on_cancel` results
/// without entering `f`; already running member operations finish normally.
pub fn par_map_per_host_with_cancel<T, R, K, F, C, D>(
    items: Vec<T>,
    global_limit: usize,
    per_host_limit: usize,
    host_of: K,
    f: F,
    cancelled: C,
    on_cancel: D,
) -> ModelResult<Vec<R>>
where
    T: Send,
    R: Send,
    K: Fn(&T) -> Option<String>,
    F: Fn(T) -> R + Sync,
    C: Fn() -> bool + Sync,
    D: Fn(T) -> R + Sync,
{
    map_with_control(
        items,
        global_limit,
        per_host_limit,
        host_of,
        f,
        cancelled,
        on_cancel,
        usize::MAX,
    )
}

fn map_with_control<T, R, K, F, C, D>(
    items: Vec<T>,
    global_limit: usize,
    per_host_limit: usize,
    host_of: K,
    f: F,
    cancelled: C,
    on_cancel: D,
    spawn_limit: usize,
) -> ModelResult<Vec<R>>
where
    T: Send,
    R: Send,
    K: Fn(&T) -> Option<String>,
    F: Fn(T) -> R + Sync,
    C: Fn() -> bool + Sync,
    D: Fn(T) -> R + Sync,
{
    let count = items.len();
    if count == 0 {
        return Ok(Vec::new());
    }
    let workers = count.min(global_limit.max(1));
    let mut grouped: HashMap<Option<String>, VecDeque<usize>> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        grouped.entry(host_of(item)).or_default().push_back(index);
    }
    let groups = grouped
        .into_iter()
        .map(|(host, indices)| Group {
            indices,
            active: 0,
            limit: if host.is_some() {
                per_host_limit.max(1)
            } else {
                workers
            },
        })
        .collect();
    let state = Mutex::new(Queue {
        groups,
        unclaimed: count,
        started: false,
        failed: false,
    });
    let changed = Condvar::new();
    let slots: Vec<Mutex<Option<T>>> = items
        .into_iter()
        .map(|item| Mutex::new(Some(item)))
        .collect();
    let results: Vec<Mutex<Option<R>>> = (0..count).map(|_| Mutex::new(None)).collect();

    std::thread::scope(|scope| -> ModelResult<()> {
        for worker in 0..workers {
            if worker == spawn_limit {
                let mut queue = state.lock().expect("member queue poisoned");
                queue.failed = true;
                changed.notify_all();
                return Err(ModelError::new(
                    ErrorCode::IoError,
                    "cannot start member worker",
                ));
            }
            let state = &state;
            let changed = &changed;
            let slots = &slots;
            let results = &results;
            let f = &f;
            let cancelled = &cancelled;
            let on_cancel = &on_cancel;
            if let Err(error) = std::thread::Builder::new()
                .name("gwz-member".into())
                .spawn_scoped(scope, move || {
                    loop {
                        let (group, index) = {
                            let mut queue = state.lock().expect("member queue poisoned");
                            loop {
                                if queue.failed {
                                    return;
                                }
                                if !queue.started {
                                    queue = changed.wait(queue).expect("member queue poisoned");
                                    continue;
                                }
                                if queue.unclaimed == 0 {
                                    return;
                                }
                                if let Some((group, index)) = queue
                                    .groups
                                    .iter_mut()
                                    .enumerate()
                                    .find_map(|(group, row)| {
                                        if row.active < row.limit {
                                            row.indices.pop_front().map(|index| {
                                                row.active += 1;
                                                (group, index)
                                            })
                                        } else {
                                            None
                                        }
                                    })
                                {
                                    queue.unclaimed -= 1;
                                    break (group, index);
                                }
                                queue = changed.wait(queue).expect("member queue poisoned");
                            }
                        };
                        let item = slots[index]
                            .lock()
                            .expect("member slot poisoned")
                            .take()
                            .expect("each member is taken once");
                        let _permit = GroupPermit {
                            state,
                            changed,
                            group,
                        };
                        let result = if cancelled() {
                            on_cancel(item)
                        } else {
                            f(item)
                        };
                        *results[index].lock().expect("member result poisoned") = Some(result);
                    }
                })
            {
                let mut queue = state.lock().expect("member queue poisoned");
                queue.failed = true;
                changed.notify_all();
                return Err(ModelError::new(
                    ErrorCode::IoError,
                    format!("cannot start member worker: {error}"),
                ));
            }
        }
        state.lock().expect("member queue poisoned").started = true;
        changed.notify_all();
        Ok(())
    })?;

    Ok(results
        .into_iter()
        .map(|cell| {
            cell.into_inner()
                .expect("member result poisoned")
                .expect("every member has a result")
        })
        .collect())
}

cfg_if::cfg_if! { if #[cfg(test)] {
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn failed_worker_spawn_never_enters_a_handler() {
        let entered = AtomicUsize::new(0);
        let result = map_with_control(
            (0..16).collect(), 4, 4, |_| Some("host".into()),
            |item| { entered.fetch_add(1, Ordering::SeqCst); item },
            || false, |item| item, 2,
        );
        assert!(matches!(result, Err(ModelError { code: ErrorCode::IoError, .. })));
        assert_eq!(entered.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cancelling_queued_items_does_not_enter_the_handler() {
        let cancelled = AtomicBool::new(false);
        let entered = AtomicUsize::new(0);
        let result = par_map_per_host_with_cancel(
            (0..32).collect(), 1, 32, |_| Some("host".into()),
            |item| { entered.fetch_add(1, Ordering::SeqCst); cancelled.store(true, Ordering::SeqCst); item },
            || cancelled.load(Ordering::SeqCst),
            |_| usize::MAX,
        ).unwrap();
        assert_eq!(entered.load(Ordering::SeqCst), 1);
        assert_eq!(result[0], 0);
        assert!(result[1..].iter().all(|item| *item == usize::MAX));
    }

    #[test]
    fn panicking_handler_releases_its_host_permit() {
        let completed = AtomicUsize::new(0);
        let result = std::panic::catch_unwind(|| {
            let _ = par_map_per_host((0..16).collect(), 4, 1, |_| Some("host".into()), |item| {
                if item == 0 { panic!("worker failure"); }
                completed.fetch_add(1, Ordering::SeqCst);
                item
            });
        });
        assert!(result.is_err());
        assert_eq!(completed.load(Ordering::SeqCst), 15);
    }
}
} }
