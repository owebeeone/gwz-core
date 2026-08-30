use std::cmp::Ordering;
use std::collections::BTreeSet;
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
            let mut messages = {
                let _permit = budget.acquire();
                history.messages()
            };
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
    let mut state = MergeState::new(cursors.len());
    let mut stats = CommitLogMergeStats::default();

    loop {
        seal_groups(&mut state.pending, cursors);
        if emit_ready_groups(&mut state.pending, options.max_entries, &mut stats, emit) {
            return stats;
        }

        let Some(index) =
            frontier_blocker_cursor(&state.pending, cursors).or_else(|| next_cursor(cursors))
        else {
            emit_forced_groups(&mut state.pending, options.max_entries, &mut stats, emit);
            return stats;
        };
        let entry = cursors[index]
            .head
            .take()
            .expect("the selected cursor has an entry");

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
            advance_cursor(&mut cursors[index], emit);
            continue;
        }

        state.admit(index, entry);
        stats.max_buffered_entries = stats
            .max_buffered_entries
            .max(buffered_entry_count(&state.pending));

        if let Some(limit) = options.max_entries {
            let remaining = limit.saturating_sub(stats.groups_emitted);
            if remaining != 0 && state.pending.len() >= remaining {
                state.absorb_seen_siblings(cursors);
                stats.max_buffered_entries = stats
                    .max_buffered_entries
                    .max(buffered_entry_count(&state.pending));
                emit_forced_groups(&mut state.pending, Some(limit), &mut stats, emit);
                return stats;
            }
        }

        advance_cursor(&mut cursors[index], emit);
    }
}

type GroupId = u64;

struct PendingGroup {
    id: GroupId,
    group: CommitLogGroup,
    sealed: bool,
    repositories: BTreeSet<usize>,
    predecessors: BTreeSet<GroupId>,
}

struct MergeState {
    pending: Vec<PendingGroup>,
    last_group_by_cursor: Vec<Option<GroupId>>,
    next_group_id: GroupId,
}

impl MergeState {
    fn new(cursor_count: usize) -> Self {
        Self {
            pending: Vec::new(),
            last_group_by_cursor: vec![None; cursor_count],
            next_group_id: 0,
        }
    }

    fn admit(&mut self, cursor: usize, entry: CommitLogEntry) {
        if self.try_join(cursor, &entry) {
            return;
        }

        let id = self.next_group_id;
        self.next_group_id += 1;
        let predecessors = self.pending_predecessor(cursor).into_iter().collect();
        self.pending.push(PendingGroup {
            id,
            group: assemble_commit_log_groups([entry], true)
                .pop()
                .expect("one entry assembles one group"),
            sealed: false,
            repositories: [cursor].into_iter().collect(),
            predecessors,
        });
        self.last_group_by_cursor[cursor] = Some(id);
    }

    fn try_join(&mut self, cursor: usize, entry: &CommitLogEntry) -> bool {
        let predecessor = self.pending_predecessor(cursor);
        let candidate = (0..self.pending.len()).find_map(|index| {
            let pending = &self.pending[index];
            if pending.sealed
                || predecessor.is_some_and(|predecessor| {
                    group_depends_on(&self.pending, predecessor, pending.id)
                })
            {
                return None;
            }
            join_group(&pending.group, entry).map(|joined| (index, joined))
        });
        let Some((index, joined)) = candidate else {
            return false;
        };

        let group = &mut self.pending[index];
        group.group = joined;
        group.repositories.insert(cursor);
        if let Some(predecessor) = predecessor {
            group.predecessors.insert(predecessor);
        }
        self.last_group_by_cursor[cursor] = Some(group.id);
        true
    }

    fn pending_predecessor(&self, cursor: usize) -> Option<GroupId> {
        self.last_group_by_cursor[cursor]
            .filter(|id| self.pending.iter().any(|group| group.id == *id))
    }

    fn absorb_seen_siblings(&mut self, cursors: &mut [CursorState]) {
        for (cursor, state) in cursors.iter_mut().enumerate() {
            let Some(entry) = state.head.as_ref().cloned() else {
                continue;
            };
            if self.try_join(cursor, &entry) {
                state.head = None;
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

fn seal_groups(groups: &mut [PendingGroup], cursors: &[CursorState]) {
    for group in groups.iter_mut().filter(|group| !group.sealed) {
        let threshold = group
            .group
            .ordering_timestamp_seconds()
            .saturating_sub(COALESCING_WINDOW_SECONDS);
        group.sealed = cursors.iter().enumerate().all(|(index, cursor)| {
            group.repositories.contains(&index)
                || cursor.done
                || cursor
                    .head
                    .as_ref()
                    .is_some_and(|entry| entry.committer.time.seconds < threshold)
        });
    }
}

fn frontier_blocker_cursor(groups: &[PendingGroup], cursors: &[CursorState]) -> Option<usize> {
    let root = (0..groups.len())
        .filter(|index| {
            !groups[*index].sealed
                && !group_is_blocked(groups, *index)
                && groups.iter().any(|group| {
                    group.sealed && group_depends_on(groups, group.id, groups[*index].id)
                })
        })
        .min_by(|left, right| compare_groups(&groups[*left].group, &groups[*right].group))?;
    let threshold = groups[root]
        .group
        .ordering_timestamp_seconds()
        .saturating_sub(COALESCING_WINDOW_SECONDS);
    cursors
        .iter()
        .enumerate()
        .filter(|(index, cursor)| {
            !groups[root].repositories.contains(index)
                && cursor
                    .head
                    .as_ref()
                    .is_some_and(|entry| entry.committer.time.seconds >= threshold)
        })
        .min_by(|(_, left), (_, right)| {
            compare_entries(
                left.head.as_ref().expect("a blocker has a head"),
                right.head.as_ref().expect("a blocker has a head"),
            )
        })
        .map(|(index, _)| index)
}

fn group_depends_on(groups: &[PendingGroup], group: GroupId, ancestor: GroupId) -> bool {
    let mut frontier = vec![group];
    let mut seen = BTreeSet::new();
    while let Some(id) = frontier.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(group) = groups.iter().find(|group| group.id == id) else {
            continue;
        };
        if group.predecessors.contains(&ancestor) {
            return true;
        }
        frontier.extend(group.predecessors.iter().copied());
    }
    false
}

fn group_is_blocked(groups: &[PendingGroup], index: usize) -> bool {
    groups[index]
        .predecessors
        .iter()
        .any(|predecessor| groups.iter().any(|group| group.id == *predecessor))
}

fn emit_ready_groups(
    groups: &mut Vec<PendingGroup>,
    max_entries: Option<usize>,
    stats: &mut CommitLogMergeStats,
    emit: &mut impl FnMut(CommitLogMergedEvent),
) -> bool {
    loop {
        if max_entries.is_some_and(|limit| stats.groups_emitted >= limit) {
            return true;
        }
        let next = (0..groups.len())
            .filter(|index| groups[*index].sealed && !group_is_blocked(groups, *index))
            .min_by(|left, right| compare_groups(&groups[*left].group, &groups[*right].group));
        let Some(index) = next else {
            return false;
        };
        let group = groups.remove(index);
        emit(CommitLogMergedEvent::Group(group.group));
        stats.groups_emitted += 1;
    }
}

fn emit_forced_groups(
    groups: &mut Vec<PendingGroup>,
    max_entries: Option<usize>,
    stats: &mut CommitLogMergeStats,
    emit: &mut impl FnMut(CommitLogMergedEvent),
) {
    while !groups.is_empty() && max_entries.is_none_or(|limit| stats.groups_emitted < limit) {
        let index = (0..groups.len())
            .filter(|index| !group_is_blocked(groups, *index))
            .min_by(|left, right| compare_groups(&groups[*left].group, &groups[*right].group))
            .expect("the acyclic predecessor graph has an unblocked group");
        let group = groups.remove(index);
        emit(CommitLogMergedEvent::Group(group.group));
        stats.groups_emitted += 1;
    }
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

fn buffered_entry_count(groups: &[PendingGroup]) -> usize {
    groups.iter().map(|group| group.group.entries().len()).sum()
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
