//! S2.5 requirement-row tests for the bounded cross-repository merge.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::{LogOptions, LogRequest};

use super::coalesce::{CommitLogGroup, CommitLogProvenance};
use super::merge::{
    CommitLogMergeOptions, CommitLogMergedEvent, DEFAULT_MAX_ENTRIES, effective_max_entries,
    stream_test_cursors,
};
use super::{CommitLogEntry, CommitLogEvent, CommitLogIdentity, CommitLogTarget, CommitLogTime};

const MARKER_A: &str = "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91";

#[test]
fn l_coa_7_window_boundary_is_inclusive_and_one_second_beyond_splits() {
    let inclusive = merge(
        vec![
            vec![marked_entry("mem_a", "a", 160, MARKER_A)],
            vec![marked_entry("mem_b", "b", 100, MARKER_A)],
        ],
        options(None, 2),
    );
    assert_eq!(inclusive.groups.len(), 1);
    assert_eq!(member_hashes(&inclusive.groups[0]), ["a", "b"]);
    assert_eq!(inclusive.groups[0].ordering_timestamp_seconds(), 160);

    let exclusive = merge(
        vec![
            vec![marked_entry("mem_a", "a", 160, MARKER_A)],
            vec![marked_entry("mem_b", "b", 99, MARKER_A)],
        ],
        options(None, 2),
    );
    assert_eq!(exclusive.groups.len(), 2);
    assert_eq!(member_hashes(&exclusive.groups[0]), ["a"]);
    assert_eq!(member_hashes(&exclusive.groups[1]), ["b"]);
    assert!(
        exclusive.groups.iter().all(|group| {
            group.provenance() == &CommitLogProvenance::Marker(MARKER_A.to_owned())
        })
    );
}

#[test]
fn l_env_2_non_monotone_frontier_escape_repeats_marker_provenance() {
    let output = merge(
        vec![
            vec![marked_entry("mem_a", "a", 160, MARKER_A)],
            // Native Git order is deliberately non-monotone here. Seeing 0
            // closes the first group; the later 200 cannot travel backward
            // across the already-emitted 160 frontier.
            vec![
                marked_entry("mem_b", "b-old", 0, MARKER_A),
                marked_entry("mem_b", "b-late", 200, MARKER_A),
            ],
        ],
        options(None, 2),
    );

    assert_eq!(
        output
            .groups
            .iter()
            .map(CommitLogGroup::ordering_timestamp_seconds)
            .collect::<Vec<_>>(),
        [160, 0, 200]
    );
    assert_eq!(member_hashes(&output.groups[0]), ["a"]);
    assert_eq!(member_hashes(&output.groups[1]), ["b-old"]);
    assert_eq!(member_hashes(&output.groups[2]), ["b-late"]);
    assert!(
        output.groups.iter().all(|group| {
            group.provenance() == &CommitLogProvenance::Marker(MARKER_A.to_owned())
        })
    );
}

#[test]
fn f1_time_closed_output_blocked_group_rejects_a_frontier_late_sibling() {
    let output = merge(
        vec![
            vec![
                entry("mem_a", "a-blocker", 0),
                marked_entry("mem_a", "a-marker", 100, MARKER_A),
            ],
            vec![
                entry("mem_b", "b-blocker", 0),
                marked_entry("mem_b", "b-marker", 100, MARKER_A),
            ],
        ],
        options(None, 2),
    );
    let marker_groups = output
        .groups
        .iter()
        .filter(|group| group.provenance() == &CommitLogProvenance::Marker(MARKER_A.to_owned()))
        .collect::<Vec<_>>();

    assert_eq!(marker_groups.len(), 2);
    assert_eq!(member_hashes(marker_groups[0]), ["b-marker"]);
    assert_eq!(member_hashes(marker_groups[1]), ["a-marker"]);
}

#[test]
fn f2_late_group_membership_inherits_each_repository_predecessor() {
    let output = merge(
        vec![
            vec![
                entry("mem_a", "a-prior", 90),
                marked_entry("mem_a", "a-late", 100, MARKER_A),
            ],
            vec![marked_entry("mem_b", "b", 100, MARKER_A)],
        ],
        options(None, 2),
    );

    assert_eq!(
        output.groups.iter().map(member_hashes).collect::<Vec<_>>(),
        [vec!["a-prior"], vec!["b", "a-late"]]
    );

    let held = merge(
        vec![
            vec![
                entry("mem_a", "a-prior", 90),
                marked_entry("mem_a", "a-late", 100, MARKER_A),
            ],
            vec![marked_entry("mem_b", "b", 100, MARKER_A)],
            vec![entry("mem_c", "c-blocker", 50)],
        ],
        options(None, 3),
    );
    assert_eq!(
        held.groups.iter().map(member_hashes).collect::<Vec<_>>(),
        [vec!["a-prior"], vec!["b", "a-late"], vec!["c-blocker"]]
    );
}

#[test]
fn l_env_3_cap_force_closes_an_open_group_with_seen_siblings_only() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let cursors = vec![
        CountingCursor::new(
            [
                CommitLogEvent::Entry(marked_entry("mem_a", "a-seen", 100, MARKER_A)),
                CommitLogEvent::Degradation(degradation("mem_a", "past cap")),
            ],
            pulls.clone(),
        ),
        CountingCursor::new(
            [CommitLogEvent::Entry(marked_entry(
                "mem_b", "b-seen", 100, MARKER_A,
            ))],
            pulls.clone(),
        ),
        CountingCursor::new(
            [
                CommitLogEvent::Entry(entry("mem_c", "blocker", 99)),
                CommitLogEvent::Entry(marked_entry("mem_c", "c-unseen", 98, MARKER_A)),
            ],
            pulls.clone(),
        ),
    ];
    let mut events = Vec::new();
    let stats = stream_test_cursors(cursors, options(Some(1), 1), |event| events.push(event));
    assert!(
        events
            .iter()
            .all(|event| matches!(event, CommitLogMergedEvent::Group(_))),
        "the beyond-cap degradation sentinel must never be yielded"
    );
    let groups = groups(events);

    assert_eq!(groups.len(), 1);
    assert_eq!(member_hashes(&groups[0]), ["a-seen", "b-seen"]);
    assert_eq!(
        groups[0].provenance(),
        &CommitLogProvenance::Marker(MARKER_A.to_owned())
    );
    assert_eq!(stats.groups_emitted(), 1);
    assert_eq!(pulls.load(Ordering::SeqCst), 3, "prime each cursor only");
}

#[test]
fn f5_satisfied_cap_never_pulls_or_reports_the_successor() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let cursor = CountingCursor::new(
        [
            CommitLogEvent::Entry(entry("mem_a", "visible", 100)),
            CommitLogEvent::Degradation(degradation("mem_a", "past cap")),
        ],
        pulls.clone(),
    );
    let mut events = Vec::new();
    let stats = stream_test_cursors(
        vec![cursor],
        CommitLogMergeOptions::new(false, Some(1), 1),
        |event| events.push(event),
    );

    assert_eq!(pulls.load(Ordering::SeqCst), 1);
    assert_eq!(stats.groups_emitted(), 1);
    assert!(matches!(
        events.as_slice(),
        [CommitLogMergedEvent::Group(_)]
    ));
}

#[test]
fn l_env_1_orders_absolute_i64_instants_and_preserves_offsets() {
    let output = merge(
        vec![
            vec![entry_with_offset("mem_pre", "pre", i64::MIN, 14 * 60)],
            vec![entry_with_offset(
                "mem_future",
                "future",
                i64::MAX - 1,
                -12 * 60,
            )],
            vec![entry_with_offset("mem_later", "later", 10, -12 * 60)],
            vec![entry_with_offset("mem_earlier", "earlier", 9, 14 * 60)],
        ],
        options(None, 4),
    );

    assert_eq!(
        output
            .groups
            .iter()
            .map(|group| group.entries()[0].commit_id.as_str())
            .collect::<Vec<_>>(),
        ["future", "later", "earlier", "pre"]
    );
    assert_eq!(
        output.groups[0].entries()[0].committer.time.offset_minutes,
        -720
    );
    assert_eq!(
        output.groups[3].entries()[0].committer.time.offset_minutes,
        840
    );
}

#[test]
fn l_ord_2_equal_time_group_tie_uses_least_sibling_member_then_hash() {
    let output = merge(
        vec![
            vec![marked_entry("mem_z", "z", 100, MARKER_A)],
            vec![marked_entry("mem_b", "b", 100, MARKER_A)],
            vec![entry("mem_a", "a", 100)],
            vec![entry("mem_c", "c", 100)],
        ],
        options(None, 4),
    );

    assert_eq!(
        output
            .groups
            .iter()
            .map(|group| group
                .entries()
                .iter()
                .map(|entry| entry.commit_id.as_str())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        [vec!["a"], vec!["b", "z"], vec!["c"]]
    );
}

#[test]
fn l_dep_1_default_is_global_50_and_explicit_windows_lift_only_the_default() {
    let default = LogRequest::default();
    assert_eq!(
        effective_max_entries(&default, false),
        Some(DEFAULT_MAX_ENTRIES)
    );

    let explicit = request_with_options(LogOptions {
        max_entries: Some(7),
        ..LogOptions::default()
    });
    assert_eq!(effective_max_entries(&explicit, false), Some(7));
    assert_eq!(effective_max_entries(&explicit, true), Some(7));

    let no_limit = request_with_options(LogOptions {
        max_entries: Some(0),
        ..LogOptions::default()
    });
    assert_eq!(effective_max_entries(&no_limit, false), None);
    assert_eq!(effective_max_entries(&default, true), None);

    for filtered in [
        LogOptions {
            since: Some("2026-08-01T00:00:00Z".to_owned()),
            ..LogOptions::default()
        },
        LogOptions {
            until: Some("@0".to_owned()),
            ..LogOptions::default()
        },
    ] {
        assert_eq!(
            effective_max_entries(&request_with_options(filtered), false),
            None
        );
    }

    let marker_pairs = (0..60)
        .map(|index| {
            let marker = marker_for(index);
            (
                marked_entry(
                    "mem_a",
                    &format!("a{index:02}"),
                    10_000 - index * 61,
                    &marker,
                ),
                marked_entry(
                    "mem_b",
                    &format!("b{index:02}"),
                    10_000 - index * 61,
                    &marker,
                ),
            )
        })
        .collect::<Vec<_>>();
    let output = merge(
        vec![
            marker_pairs.iter().map(|pair| pair.0.clone()).collect(),
            marker_pairs.into_iter().map(|pair| pair.1).collect(),
        ],
        options(effective_max_entries(&default, false), 2),
    );
    assert_eq!(output.groups.len(), DEFAULT_MAX_ENTRIES);
    assert!(output.groups.iter().all(|group| group.entries().len() == 2));
}

#[test]
fn l_prf_1_streaming_has_a_window_bounded_high_water_and_stops_at_cap() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let cursors = (0..3)
        .map(|index| GeneratedCursor::new(index, 10_000, pulls.clone()))
        .collect();
    let mut events = Vec::new();
    let stats = stream_test_cursors(cursors, options(Some(DEFAULT_MAX_ENTRIES), 3), |event| {
        events.push(event)
    });

    assert_eq!(groups(events).len(), DEFAULT_MAX_ENTRIES);
    assert!(stats.max_buffered_entries() <= 3);
    assert!(
        pulls.load(Ordering::SeqCst) <= DEFAULT_MAX_ENTRIES + 3,
        "the merge may read one head per cursor, never all 30,000 entries"
    );
}

#[rustfmt::skip]
#[test]
fn l_coa_7_frontier_eligibility_is_exact_and_bounded() {
    fn marked(member: &str, times: [i64; 3]) -> Vec<CommitLogEntry> {
        times.into_iter().enumerate().map(|(i, time)|
            marked_entry(&format!("mem_{member}"), &format!("{member}-{i}"), time, MARKER_A)).collect()
    }
    fn marker_hashes(output: &MergeOutput) -> Vec<Vec<&str>> {
        output.groups.iter().filter(|group|
            matches!(group.provenance(), CommitLogProvenance::Marker(_))).map(member_hashes).collect()
    }
    fn high_water(tail_len: usize) -> usize {
        fn inverted(member: &str, tail_len: usize) -> Vec<CommitLogEntry> {
            std::iter::once(entry(member, &format!("{member}-frontier"), 0)).chain((0..tail_len).map(|i|
                entry(member, &format!("{member}-tail-{i}"), 1_000_000 - i as i64 * 61))).collect()
        }
        let output = merge(vec![inverted("mem_a", tail_len), inverted("mem_b", tail_len)], options(None, 1));
        assert_eq!(output.groups.len(), 2 * (tail_len + 1));
        output.stats.max_buffered_entries()
    }

    let seen = merge(vec![marked("0", [0, 0, 100]), marked("1", [40, 0, 99]),
        marked("2", [39, 40, 100])], options(None, 1));
    assert_eq!(seen.groups.iter().map(member_hashes).collect::<Vec<_>>(), [
        vec!["1-0", "2-0", "0-0"], vec!["2-1", "0-1", "1-1"],
        vec!["2-2"], vec!["0-2"], vec!["1-2"]]);

    let blocker = std::iter::once(entry("mem_b", "prime", 99))
        .chain((1..63).map(|i| entry("mem_b", &format!("b-{i}"), 99)))
        .chain([marked_entry("mem_b", "b-late", 100, MARKER_A)]).collect();
    let joinable = merge(vec![vec![marked_entry("mem_a", "a", 100, MARKER_A)], blocker], options(None, 1));
    assert_eq!(marker_hashes(&joinable), [vec!["a", "b-late"]]);

    let blocker = std::iter::once(entry("mem_c", "prime", 99))
        .chain((0..63).map(|i| entry("mem_c", &format!("c-{i}"), 1_000 - i)))
        .chain([marked_entry("mem_c", "c-late", 97, MARKER_A)]).collect();
    let closed = merge(vec![vec![marked_entry("mem_a", "a", 100, MARKER_A)],
        vec![marked_entry("mem_b", "b-join", 98, MARKER_A)], blocker], options(None, 1));
    assert_eq!(marker_hashes(&closed), [vec!["a", "b-join"], vec!["c-late"]]);

    let short = high_water(100);
    let long = high_water(1_000);
    assert_eq!(short, 66, "two frontier entries plus K=64 patience");
    assert_eq!(long, short);
}

#[test]
fn f6_jobs_values_overlap_to_the_ceiling_and_preserve_complete_events() {
    fn run(jobs: usize) -> (Vec<CommitLogMergedEvent>, usize) {
        let gate = Arc::new(OverlapGate::new(jobs.min(4)));
        let cursors = vec![
            OverlapCursor::new(
                [CommitLogEvent::Entry(marked_entry(
                    "mem_z", MARKER_A, 200, MARKER_A,
                ))],
                gate.clone(),
            ),
            OverlapCursor::new(
                [CommitLogEvent::Entry(marked_entry(
                    "mem_b", "b", 200, MARKER_A,
                ))],
                gate.clone(),
            ),
            OverlapCursor::new(
                [CommitLogEvent::Entry(entry_with_offset(
                    "mem_a", "a", 150, -720,
                ))],
                gate.clone(),
            ),
            OverlapCursor::new(
                [CommitLogEvent::Degradation(degradation(
                    "mem_d",
                    "deterministic degradation",
                ))],
                gate.clone(),
            ),
        ];
        let mut events = Vec::new();
        let stats = stream_test_cursors(cursors, options(None, jobs), |event| events.push(event));
        assert_eq!(stats.max_concurrent_reads(), jobs.min(4));
        (events, gate.max_active())
    }

    let one = run(1);
    let two = run(2);
    let eight = run(8);

    assert_eq!(one.0, two.0);
    assert_eq!(one.0, eight.0);
    assert_eq!(one.1, 1);
    assert_eq!(two.1, 2);
    assert_eq!(eight.1, 4);
}

struct MergeOutput {
    groups: Vec<CommitLogGroup>,
    stats: super::merge::CommitLogMergeStats,
}

fn merge(cursors: Vec<Vec<CommitLogEntry>>, options: CommitLogMergeOptions) -> MergeOutput {
    let cursors = cursors
        .into_iter()
        .map(|entries| entries.into_iter().map(CommitLogEvent::Entry))
        .collect();
    let mut events = Vec::new();
    let stats = stream_test_cursors(cursors, options, |event| events.push(event));
    MergeOutput {
        groups: groups(events),
        stats,
    }
}

fn groups(events: Vec<CommitLogMergedEvent>) -> Vec<CommitLogGroup> {
    events
        .into_iter()
        .filter_map(|event| match event {
            CommitLogMergedEvent::Group(group) => Some(group),
            CommitLogMergedEvent::Degradation(_) => None,
        })
        .collect()
}

fn options(max_entries: Option<usize>, jobs: usize) -> CommitLogMergeOptions {
    CommitLogMergeOptions::new(true, max_entries, jobs)
}

fn request_with_options(options: LogOptions) -> LogRequest {
    LogRequest {
        options: Some(options),
        ..LogRequest::default()
    }
}

fn member_hashes(group: &CommitLogGroup) -> Vec<&str> {
    group
        .entries()
        .iter()
        .map(|entry| entry.commit_id.as_str())
        .collect()
}

fn degradation(member_id: &str, detail: &str) -> super::CommitLogDegradation {
    super::CommitLogDegradation {
        target: CommitLogTarget {
            member_id: member_id.to_owned(),
            member_path: member_id.to_owned(),
            source_kind: crate::artifact::ArtifactSourceKind::Git,
        },
        kind: super::CommitLogDegradationKind::HistoryUnreadable,
        operand: Some("HEAD".to_owned()),
        detail: detail.to_owned(),
    }
}

fn marked_entry(member_id: &str, hash: &str, seconds: i64, marker: &str) -> CommitLogEntry {
    let mut entry = entry(member_id, hash, seconds);
    entry.message =
        format!("subject\n\nGWZ-Commit-ID: {marker}\nGWZ-Workspace-ID: ws_test\n").into_bytes();
    entry
}

fn marker_for(index: i64) -> String {
    format!("01987b0c-2f75-7c4a-9a32-{index:012x}")
}

fn entry(member_id: &str, hash: &str, seconds: i64) -> CommitLogEntry {
    entry_with_offset(member_id, hash, seconds, 0)
}

fn entry_with_offset(
    member_id: &str,
    hash: &str,
    seconds: i64,
    offset_minutes: i32,
) -> CommitLogEntry {
    let time = CommitLogTime {
        seconds,
        offset_minutes,
    };
    CommitLogEntry {
        target: CommitLogTarget {
            member_id: member_id.to_owned(),
            member_path: member_id.to_owned(),
            source_kind: crate::artifact::ArtifactSourceKind::Git,
        },
        commit_id: hash.to_owned(),
        parent_ids: Vec::new(),
        author: CommitLogIdentity {
            name: b"Author".to_vec(),
            email: b"author@example.invalid".to_vec(),
            time,
        },
        committer: CommitLogIdentity {
            name: b"Committer".to_vec(),
            email: b"committer@example.invalid".to_vec(),
            time,
        },
        message: hash.as_bytes().to_vec(),
        message_encoding: None,
    }
}

struct CountingCursor {
    events: std::vec::IntoIter<CommitLogEvent>,
    pulls: Arc<AtomicUsize>,
}

impl CountingCursor {
    fn new(events: impl IntoIterator<Item = CommitLogEvent>, pulls: Arc<AtomicUsize>) -> Self {
        Self {
            events: events.into_iter().collect::<Vec<_>>().into_iter(),
            pulls,
        }
    }
}

impl Iterator for CountingCursor {
    type Item = CommitLogEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.pulls.fetch_add(1, Ordering::SeqCst);
        self.events.next()
    }
}

struct OverlapGate {
    state: Mutex<OverlapState>,
    ready: Condvar,
    target: usize,
}

struct OverlapState {
    active: usize,
    max_active: usize,
    released: bool,
}

impl OverlapGate {
    fn new(target: usize) -> Self {
        Self {
            state: Mutex::new(OverlapState {
                active: 0,
                max_active: 0,
                released: false,
            }),
            ready: Condvar::new(),
            target,
        }
    }

    fn enter(&self) {
        let mut state = self.state.lock().unwrap();
        state.active += 1;
        state.max_active = state.max_active.max(state.active);
        if state.active == self.target {
            state.released = true;
            self.ready.notify_all();
        }
        while !state.released {
            state = self.ready.wait(state).unwrap();
        }
        state.active -= 1;
    }

    fn max_active(&self) -> usize {
        self.state.lock().unwrap().max_active
    }
}

struct OverlapCursor {
    events: std::vec::IntoIter<CommitLogEvent>,
    gate: Arc<OverlapGate>,
    first: bool,
}

impl OverlapCursor {
    fn new(events: impl IntoIterator<Item = CommitLogEvent>, gate: Arc<OverlapGate>) -> Self {
        Self {
            events: events.into_iter().collect::<Vec<_>>().into_iter(),
            gate,
            first: true,
        }
    }
}

impl Iterator for OverlapCursor {
    type Item = CommitLogEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if std::mem::take(&mut self.first) {
            self.gate.enter();
        }
        self.events.next()
    }
}

struct GeneratedCursor {
    member: usize,
    remaining: usize,
    pulls: Arc<AtomicUsize>,
}

impl GeneratedCursor {
    fn new(member: usize, remaining: usize, pulls: Arc<AtomicUsize>) -> Self {
        Self {
            member,
            remaining,
            pulls,
        }
    }
}

impl Iterator for GeneratedCursor {
    type Item = CommitLogEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.pulls.fetch_add(1, Ordering::SeqCst);
        let index = self.remaining.checked_sub(1)?;
        self.remaining = index;
        Some(CommitLogEvent::Entry(entry(
            &format!("mem_{}", self.member),
            &format!("{}-{index}", self.member),
            (index as i64) * 61 + self.member as i64,
        )))
    }
}
