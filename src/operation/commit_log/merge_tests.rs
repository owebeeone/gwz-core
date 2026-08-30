//! S2.5 requirement-row tests for the bounded cross-repository merge.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
fn l_env_3_cap_force_closes_an_open_group_with_seen_siblings_only() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let cursors = vec![
        CountingCursor::new(
            [marked_entry("mem_a", "a-seen", 100, MARKER_A)],
            pulls.clone(),
        ),
        CountingCursor::new(
            [marked_entry("mem_b", "b-seen", 100, MARKER_A)],
            pulls.clone(),
        ),
        CountingCursor::new(
            [
                entry("mem_c", "blocker", 99),
                marked_entry("mem_c", "c-unseen", 98, MARKER_A),
            ],
            pulls.clone(),
        ),
    ];
    let mut events = Vec::new();
    let stats = stream_test_cursors(cursors, options(Some(1), 1), |event| events.push(event));
    let groups = groups(events);

    assert_eq!(groups.len(), 1);
    assert_eq!(member_hashes(&groups[0]), ["a-seen", "b-seen"]);
    assert_eq!(
        groups[0].provenance(),
        &CommitLogProvenance::Marker(MARKER_A.to_owned())
    );
    assert_eq!(stats.groups_emitted(), 1);
    assert!(
        pulls.load(Ordering::SeqCst) <= 4,
        "one head per repo plus the selected cursor's advance must not reach c-unseen"
    );
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

#[test]
fn l_prf_2_and_l_env_4_jobs_values_are_byte_identical_and_ceiling_bounded() {
    let cursors = || {
        vec![
            vec![entry("mem_a", "a2", 200), entry("mem_a", "a1", 100)],
            vec![entry("mem_b", "b2", 200), entry("mem_b", "b1", 99)],
            vec![entry("mem_c", "c", 150)],
        ]
    };
    let one = merge(cursors(), options(None, 1));
    let two = merge(cursors(), options(None, 2));
    let eight = merge(cursors(), options(None, 8));

    assert_eq!(fingerprint(&one.groups), fingerprint(&two.groups));
    assert_eq!(fingerprint(&one.groups), fingerprint(&eight.groups));
    assert!(one.stats.max_concurrent_reads() <= 1);
    assert!(two.stats.max_concurrent_reads() <= 2);
    assert!(eight.stats.max_concurrent_reads() <= 8);
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

fn fingerprint(groups: &[CommitLogGroup]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for group in groups {
        bytes.extend_from_slice(group.ordering_timestamp_seconds().to_string().as_bytes());
        bytes.push(b'|');
        for entry in group.entries() {
            bytes.extend_from_slice(entry.target.member_id.as_bytes());
            bytes.push(b':');
            bytes.extend_from_slice(entry.commit_id.as_bytes());
            bytes.push(b',');
        }
        bytes.push(b'\n');
    }
    bytes
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
    entries: std::vec::IntoIter<CommitLogEntry>,
    pulls: Arc<AtomicUsize>,
}

impl CountingCursor {
    fn new(entries: impl IntoIterator<Item = CommitLogEntry>, pulls: Arc<AtomicUsize>) -> Self {
        Self {
            entries: entries.into_iter().collect::<Vec<_>>().into_iter(),
            pulls,
        }
    }
}

impl Iterator for CountingCursor {
    type Item = CommitLogEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.pulls.fetch_add(1, Ordering::SeqCst);
        self.entries.next().map(CommitLogEvent::Entry)
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
