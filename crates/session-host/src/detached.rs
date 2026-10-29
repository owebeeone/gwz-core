//! The host's record of workers detached at a close bound (gwz-dev
//! `dev-docs/GwzCoreSessionDesign.md` §5.1 and §8 step 5, as
//! `GwzSplitCommandChannel.md` §1 and §3.2 adopt them for the served core).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Every session of one host shares one record: a clone is another handle to it | 2.0 §1 "Second client on one server" |
//! | Close records each worker it detaches, against its workspace, until the worker's thread ends | §8 step 5 |
//! | A write or push on a workspace with a recorded worker is refused, in every session, not run beside it | 2.0 §3.2 |
//!
//! The served core adopts the workspace registry only for this: sessions do
//! not queue behind each other's writes, and the cross-process try-lock
//! refuses them as it refuses a second CLI process (2.0 §1).

use std::fmt;
use std::sync::{Arc, Mutex, Weak};

use crate::lock;

/// The record. `Default` gives an empty one.
#[derive(Clone, Default)]
pub struct DetachedWorkers {
    entries: Arc<Mutex<Vec<Entry>>>,
}

/// A detached worker's workspace and its liveness.
type Entry = (String, Weak<()>);

/// What a running worker holds: the record sees the worker as running until
/// its thread drops this.
#[derive(Clone, Default)]
pub struct WorkerAlive {
    alive: Arc<()>,
}

impl WorkerAlive {
    /// A marker for a worker that starts now.
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Debug for WorkerAlive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkerAlive")
            .finish_non_exhaustive()
    }
}

impl DetachedWorkers {
    /// An empty record.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a worker detached on `workspace`, until its thread ends.
    pub fn record(&self, workspace: &str, worker: &WorkerAlive) {
        let mut entries = lock(&self.entries);
        entries.retain(|(_, alive)| alive.strong_count() > 0);
        entries.push((workspace.to_owned(), Arc::downgrade(&worker.alive)));
    }

    /// Whether a detached worker still runs on `workspace`.
    pub fn running_on(&self, workspace: &str) -> bool {
        let mut entries = lock(&self.entries);
        entries.retain(|(_, alive)| alive.strong_count() > 0);
        entries.iter().any(|(recorded, _)| recorded == workspace)
    }

    /// How many detached workers still run, on any workspace.
    pub fn count(&self) -> usize {
        let mut entries = lock(&self.entries);
        entries.retain(|(_, alive)| alive.strong_count() > 0);
        entries.len()
    }
}

impl fmt::Debug for DetachedWorkers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DetachedWorkers")
            .field("count", &self.count())
            .finish()
    }
}
