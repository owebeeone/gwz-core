use std::cmp::Ordering;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex};

use super::coalesce::{COALESCING_WINDOW_SECONDS, CommitLogGroup, assemble_commit_log_groups};
use super::request::CommitLogHistories;
use super::{CommitLogDegradation, CommitLogEntry, CommitLogEvent, RepositoryHistory};

pub(super) const DEFAULT_MAX_ENTRIES: usize = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CommitLogMergeOptions {
    coalesce: bool,
    max_entries: Option<usize>,
    jobs: usize,
}

impl CommitLogMergeOptions {
    pub(super) fn new(coalesce: bool, max_entries: Option<usize>, jobs: usize) -> Self {
        Self {
            coalesce,
            max_entries,
            jobs: jobs.max(1),
        }
    }

    fn from_request(request: &crate::LogRequest, has_explicit_range: bool) -> Self {
        let options = request.options.as_ref();
        Self::new(
            options.and_then(|options| options.coalesce).unwrap_or(true),
            effective_max_entries(request, has_explicit_range),
            crate::operation::resolve_jobs(
                request
                    .meta
                    .policy
                    .as_ref()
                    .and_then(|policy| policy.concurrency),
            ),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CommitLogMergedEvent {
    Group(CommitLogGroup),
    Degradation(CommitLogDegradation),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CommitLogMergeStats {
    groups_emitted: usize,
    max_buffered_entries: usize,
    max_concurrent_reads: usize,
}

impl CommitLogMergeStats {
    pub(super) fn groups_emitted(self) -> usize {
        self.groups_emitted
    }

    pub(super) fn max_buffered_entries(self) -> usize {
        self.max_buffered_entries
    }

    pub(super) fn max_concurrent_reads(self) -> usize {
        self.max_concurrent_reads
    }
}

/// Resolve only S2.5's depth policy. S2.6 owns the filters themselves.
pub(super) fn effective_max_entries(
    request: &crate::LogRequest,
    has_explicit_range: bool,
) -> Option<usize> {
    let options = request.options.as_ref();
    if let Some(max_entries) = options.and_then(|options| options.max_entries) {
        return usize::try_from(max_entries)
            .ok()
            .filter(|max_entries| *max_entries != 0);
    }
    if has_explicit_range
        || options.is_some_and(|options| options.since.is_some() || options.until.is_some())
    {
        None
    } else {
        Some(DEFAULT_MAX_ENTRIES)
    }
}

/// Stream the S2.2 request histories through the bounded merge.
pub(super) fn stream_request_histories(
    histories: CommitLogHistories,
    request: &crate::LogRequest,
    emit: impl FnMut(CommitLogMergedEvent),
) -> CommitLogMergeStats {
    let options = CommitLogMergeOptions::from_request(request, histories.has_explicit_range());
    stream_histories(histories.into_histories(), options, emit)
}

fn stream_histories(
    histories: Vec<RepositoryHistory>,
    options: CommitLogMergeOptions,
    emit: impl FnMut(CommitLogMergedEvent),
) -> CommitLogMergeStats {
    stream_sources(
        histories,
        options,
        |history, pull, reply, budget| {
            let mut messages = history.messages();
            serve_cursor(&mut messages, pull, reply, &budget);
        },
        emit,
    )
}

#[cfg(test)]
pub(super) fn stream_test_cursors<I>(
    cursors: Vec<I>,
    options: CommitLogMergeOptions,
    emit: impl FnMut(CommitLogMergedEvent),
) -> CommitLogMergeStats
where
    I: Iterator<Item = CommitLogEvent> + Send,
{
    stream_sources(
        cursors,
        options,
        |mut cursor, pull, reply, budget| serve_cursor(&mut cursor, pull, reply, &budget),
        emit,
    )
}

fn stream_sources<S, R>(
    sources: Vec<S>,
    options: CommitLogMergeOptions,
    run_source: R,
    mut emit: impl FnMut(CommitLogMergedEvent),
) -> CommitLogMergeStats
where
    S: Send,
    R: Fn(S, Receiver<()>, SyncSender<Option<CommitLogEvent>>, Arc<ReadBudget>) + Sync,
{
    let budget = Arc::new(ReadBudget::new(options.jobs));
    let mut stats = std::thread::scope(|scope| {
        let mut cursors = Vec::with_capacity(sources.len());
        for source in sources {
            let (pull_tx, pull_rx) = sync_channel(0);
            let (reply_tx, reply_rx) = sync_channel(0);
            let budget = budget.clone();
            let run_source = &run_source;
            scope.spawn(move || run_source(source, pull_rx, reply_tx, budget));
            cursors.push(CursorState {
                pull: pull_tx,
                reply: reply_rx,
                head: None,
                done: false,
            });
        }

        prime_cursors(&mut cursors, &mut emit);
        let stats = merge_cursors(&mut cursors, options, &mut emit);
        drop(cursors);
        stats
    });
    stats.max_concurrent_reads = budget.max_active();
    stats
}

fn serve_cursor(
    cursor: &mut impl Iterator<Item = CommitLogEvent>,
    pull: Receiver<()>,
    reply: SyncSender<Option<CommitLogEvent>>,
    budget: &Arc<ReadBudget>,
) {
    while pull.recv().is_ok() {
        let next = {
            let _permit = budget.acquire();
            cursor.next()
        };
        let done = next.is_none();
        if reply.send(next).is_err() || done {
            break;
        }
    }
}

struct CursorState {
    pull: SyncSender<()>,
    reply: Receiver<Option<CommitLogEvent>>,
    head: Option<CommitLogEntry>,
    done: bool,
}

fn prime_cursors(cursors: &mut [CursorState], emit: &mut impl FnMut(CommitLogMergedEvent)) {
    for cursor in cursors.iter_mut().filter(|cursor| !cursor.done) {
        if cursor.pull.send(()).is_err() {
            cursor.done = true;
        }
    }
    for cursor in cursors {
        receive_head(cursor, emit);
    }
}

fn advance_cursor(cursor: &mut CursorState, emit: &mut impl FnMut(CommitLogMergedEvent)) {
    if cursor.done || cursor.pull.send(()).is_err() {
        cursor.done = true;
        return;
    }
    receive_head(cursor, emit);
}

fn receive_head(cursor: &mut CursorState, emit: &mut impl FnMut(CommitLogMergedEvent)) {
    while !cursor.done {
        match cursor.reply.recv() {
            Ok(Some(CommitLogEvent::Entry(entry))) => {
                cursor.head = Some(entry);
                return;
            }
            Ok(Some(CommitLogEvent::Degradation(degradation))) => {
                emit(CommitLogMergedEvent::Degradation(degradation));
                if cursor.pull.send(()).is_err() {
                    cursor.done = true;
                }
            }
            Ok(None) | Err(_) => cursor.done = true,
        }
    }
}

fn merge_cursors(
    cursors: &mut [CursorState],
    options: CommitLogMergeOptions,
    emit: &mut impl FnMut(CommitLogMergedEvent),
) -> CommitLogMergeStats {
    let mut pending = Vec::<CommitLogGroup>::new();
    let mut stats = CommitLogMergeStats::default();

    loop {
        let Some(index) = next_cursor(cursors) else {
            let ready = order_forced_groups(std::mem::take(&mut pending));
            emit_ready_groups(ready, options.max_entries, &mut stats, emit);
            return stats;
        };
        let entry = cursors[index]
            .head
            .take()
            .expect("the selected cursor has an entry");
        advance_cursor(&mut cursors[index], emit);

        if !options.coalesce {
            let group = assemble_commit_log_groups([entry], false)
                .pop()
                .expect("one entry assembles one group");
            emit(CommitLogMergedEvent::Group(group));
            stats.groups_emitted += 1;
            if options
                .max_entries
                .is_some_and(|limit| stats.groups_emitted >= limit)
            {
                return stats;
            }
            continue;
        }

        admit_entry(&mut pending, entry);
        stats.max_buffered_entries = stats
            .max_buffered_entries
            .max(buffered_entry_count(&pending));

        let ready = take_emittable_groups(&mut pending, cursors);
        if emit_ready_groups(ready, options.max_entries, &mut stats, emit) {
            return stats;
        }

        if let Some(limit) = options.max_entries {
            let remaining = limit.saturating_sub(stats.groups_emitted);
            if remaining != 0 && pending.len() >= remaining {
                absorb_seen_siblings(&mut pending, cursors);
                stats.max_buffered_entries = stats
                    .max_buffered_entries
                    .max(buffered_entry_count(&pending));
                let forced = order_forced_groups(std::mem::take(&mut pending));
                for group in forced.into_iter().take(remaining) {
                    emit(CommitLogMergedEvent::Group(group));
                    stats.groups_emitted += 1;
                }
                return stats;
            }
        }
    }
}

fn next_cursor(cursors: &[CursorState]) -> Option<usize> {
    cursors
        .iter()
        .enumerate()
        .filter_map(|(index, cursor)| cursor.head.as_ref().map(|entry| (index, entry)))
        .min_by(|(_, left), (_, right)| compare_entries(left, right))
        .map(|(index, _)| index)
}

fn admit_entry(pending: &mut Vec<CommitLogGroup>, entry: CommitLogEntry) {
    for group in pending.iter_mut() {
        if let Some(joined) = join_group(group, &entry) {
            *group = joined;
            return;
        }
    }
    pending.push(
        assemble_commit_log_groups([entry], true)
            .pop()
            .expect("one entry assembles one group"),
    );
}

fn join_group(group: &CommitLogGroup, entry: &CommitLogEntry) -> Option<CommitLogGroup> {
    let candidate_time = entry.committer.time.seconds;
    let mut oldest = candidate_time;
    let mut newest = candidate_time;
    for sibling in group.entries() {
        oldest = oldest.min(sibling.committer.time.seconds);
        newest = newest.max(sibling.committer.time.seconds);
    }
    if newest.abs_diff(oldest) > COALESCING_WINDOW_SECONDS as u64 {
        return None;
    }

    let expected = group.entries().len() + 1;
    let mut assembled =
        assemble_commit_log_groups(group.entries().iter().cloned().chain([entry.clone()]), true);
    (assembled.len() == 1 && assembled[0].entries().len() == expected)
        .then(|| assembled.pop().expect("one assembled group remains"))
}

fn absorb_seen_siblings(pending: &mut [CommitLogGroup], cursors: &mut [CursorState]) {
    for cursor in cursors {
        let Some(entry) = cursor.head.as_ref() else {
            continue;
        };
        for group in pending.iter_mut() {
            if let Some(joined) = join_group(group, entry) {
                *group = joined;
                cursor.head = None;
                break;
            }
        }
    }
}

fn group_is_closed(group: &CommitLogGroup, cursors: &[CursorState]) -> bool {
    let threshold = group
        .ordering_timestamp_seconds()
        .saturating_sub(COALESCING_WINDOW_SECONDS);
    cursors.iter().all(|cursor| {
        cursor
            .head
            .as_ref()
            .is_none_or(|entry| entry.committer.time.seconds < threshold)
    })
}

fn take_emittable_groups(
    pending: &mut Vec<CommitLogGroup>,
    cursors: &[CursorState],
) -> Vec<CommitLogGroup> {
    let mut ready = Vec::new();
    loop {
        let next = (0..pending.len())
            .filter(|index| {
                group_is_closed(&pending[*index], cursors)
                    && !blocked_by_earlier_group(pending, *index)
            })
            .min_by(|left, right| compare_groups(&pending[*left], &pending[*right]));
        let Some(index) = next else {
            break;
        };
        ready.push(pending.remove(index));
    }
    ready
}

fn order_forced_groups(mut groups: Vec<CommitLogGroup>) -> Vec<CommitLogGroup> {
    let mut ordered = Vec::with_capacity(groups.len());
    while !groups.is_empty() {
        let index = (0..groups.len())
            .filter(|index| !blocked_by_earlier_group(&groups, *index))
            .min_by(|left, right| compare_groups(&groups[*left], &groups[*right]))
            .expect("the first pending group is never blocked");
        ordered.push(groups.remove(index));
    }
    ordered
}

fn blocked_by_earlier_group(groups: &[CommitLogGroup], index: usize) -> bool {
    groups[..index]
        .iter()
        .any(|earlier| groups_share_repository(earlier, &groups[index]))
}

fn groups_share_repository(left: &CommitLogGroup, right: &CommitLogGroup) -> bool {
    left.entries().iter().any(|left_entry| {
        right
            .entries()
            .iter()
            .any(|right_entry| left_entry.target.member_id == right_entry.target.member_id)
    })
}

fn emit_ready_groups(
    groups: Vec<CommitLogGroup>,
    max_entries: Option<usize>,
    stats: &mut CommitLogMergeStats,
    emit: &mut impl FnMut(CommitLogMergedEvent),
) -> bool {
    for group in groups {
        if max_entries.is_some_and(|limit| stats.groups_emitted >= limit) {
            return true;
        }
        emit(CommitLogMergedEvent::Group(group));
        stats.groups_emitted += 1;
    }
    max_entries.is_some_and(|limit| stats.groups_emitted >= limit)
}

fn compare_groups(left: &CommitLogGroup, right: &CommitLogGroup) -> Ordering {
    right
        .ordering_timestamp_seconds()
        .cmp(&left.ordering_timestamp_seconds())
        .then_with(|| group_tiebreak(left).cmp(&group_tiebreak(right)))
}

fn compare_entries(left: &CommitLogEntry, right: &CommitLogEntry) -> Ordering {
    right
        .committer
        .time
        .seconds
        .cmp(&left.committer.time.seconds)
        .then_with(|| left.target.member_id.cmp(&right.target.member_id))
        .then_with(|| left.commit_id.cmp(&right.commit_id))
}

fn group_tiebreak(group: &CommitLogGroup) -> (&str, &str) {
    group
        .entries()
        .iter()
        .map(|entry| (entry.target.member_id.as_str(), entry.commit_id.as_str()))
        .min()
        .expect("a group always contains an entry")
}

fn buffered_entry_count(groups: &[CommitLogGroup]) -> usize {
    groups.iter().map(|group| group.entries().len()).sum()
}

struct ReadBudget {
    state: Mutex<ReadBudgetState>,
    available: Condvar,
}

struct ReadBudgetState {
    available: usize,
    active: usize,
    max_active: usize,
}

impl ReadBudget {
    fn new(limit: usize) -> Self {
        Self {
            state: Mutex::new(ReadBudgetState {
                available: limit.max(1),
                active: 0,
                max_active: 0,
            }),
            available: Condvar::new(),
        }
    }

    fn acquire(&self) -> ReadPermit<'_> {
        let mut state = self.state.lock().expect("log read budget poisoned");
        while state.available == 0 {
            state = self
                .available
                .wait(state)
                .expect("log read budget poisoned");
        }
        state.available -= 1;
        state.active += 1;
        state.max_active = state.max_active.max(state.active);
        ReadPermit { budget: self }
    }

    fn max_active(&self) -> usize {
        self.state
            .lock()
            .expect("log read budget poisoned")
            .max_active
    }
}

struct ReadPermit<'a> {
    budget: &'a ReadBudget,
}

impl Drop for ReadPermit<'_> {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().expect("log read budget poisoned");
        state.active -= 1;
        state.available += 1;
        self.budget.available.notify_one();
    }
}
