use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::model::{GitObjectIdentity, MemberId, OperationActor, OperationAttribution};

use super::*;

#[derive(Default)]
struct CollectingSink {
    events: Mutex<Vec<crate::OperationEvent>>,
}

impl EventSink for CollectingSink {
    fn deliver(&self, event: crate::OperationEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl CollectingSink {
    fn take(&self) -> Vec<crate::OperationEvent> {
        self.events.lock().unwrap().clone()
    }
}

fn sample_progress() -> crate::GitTransferProgress {
    crate::GitTransferProgress {
        phase: crate::GitProgressPhase::Receiving,
        received_objects: Some(1),
        total_objects: Some(10),
        received_bytes: None,
        indexed_deltas: None,
        total_deltas: None,
    }
}

fn progress_event_count(events: &[crate::OperationEvent]) -> usize {
    events
        .iter()
        .filter(|event| event.kind == crate::EventKind::MemberProgress)
        .count()
}

#[test]
fn member_progress_rate_limit_coalesces_per_member() {
    let context = sample_context(false);
    let sink = CollectingSink::default();
    // A 10s window: rapid successive updates fall inside it and coalesce.
    let emitter = EventEmitter::new(&context, &sink, 10_000);

    emitter.member_progress("mem_a", "repos/a", sample_progress()); // first: emits
    emitter.member_progress("mem_a", "repos/a", sample_progress()); // coalesced
    emitter.member_progress("mem_a", "repos/a", sample_progress()); // coalesced
    emitter.member_progress("mem_b", "repos/b", sample_progress()); // other member: emits

    // One per member (the first update each), the rest within the window dropped.
    assert_eq!(progress_event_count(&sink.take()), 2);
}

#[test]
fn member_progress_unlimited_when_interval_zero() {
    let context = sample_context(false);
    let sink = CollectingSink::default();
    let emitter = EventEmitter::new(&context, &sink, 0);

    for _ in 0..5 {
        emitter.member_progress("mem_a", "repos/a", sample_progress());
    }

    assert_eq!(progress_event_count(&sink.take()), 5);
}

/// Records each delivered sequence after yielding, which widens the window
/// between numbering an event and storing it.
#[derive(Default)]
struct YieldingSink {
    sequences: Mutex<Vec<i64>>,
}

impl EventSink for YieldingSink {
    fn deliver(&self, event: crate::OperationEvent) {
        std::thread::yield_now();
        self.sequences.lock().unwrap().push(event.sequence);
    }
}

#[test]
fn concurrent_emitters_deliver_in_sequence_order() {
    let context = sample_context(false);
    let sink = YieldingSink::default();
    let emitter = EventEmitter::new(&context, &sink, 0);
    let threads = 8;
    let per_thread = 250;
    let start = std::sync::Barrier::new(threads);
    std::thread::scope(|scope| {
        for thread in 0..threads {
            let (emitter, start) = (&emitter, &start);
            scope.spawn(move || {
                let member_id = format!("mem_{thread}");
                start.wait();
                for _ in 0..per_thread {
                    emitter.member_started(&member_id, "repos/app");
                }
            });
        }
    });
    let expected: Vec<i64> = (0..(threads * per_thread) as i64).collect();
    assert_eq!(*sink.sequences.lock().unwrap(), expected);
}

fn run_tracking_peak<K>(global: usize, per_host: usize, host_of: K) -> usize
where
    K: Fn(&usize) -> Option<String>,
{
    let active = AtomicUsize::new(0);
    let max_active = AtomicUsize::new(0);
    let results = par_map_per_host(
        (0..8).collect(),
        global,
        per_host,
        host_of,
        |value: usize| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            max_active.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(10));
            active.fetch_sub(1, Ordering::SeqCst);
            value * 10
        },
    );
    assert_eq!(
        results.unwrap(),
        (0..8).map(|value| value * 10).collect::<Vec<_>>()
    );
    max_active.load(Ordering::SeqCst)
}

#[test]
fn par_map_per_host_caps_concurrency_per_host() {
    // One host, per-host 2: capped at 2 despite a high global ceiling.
    let peak = run_tracking_peak(50, 2, |_| Some("h".to_owned()));
    assert_eq!(peak, 2, "single host should run exactly per_host=2 at once");
}

#[test]
fn par_map_per_host_overlaps_distinct_hosts() {
    // Two hosts, per-host 1: each host serialized, but the two overlap.
    let peak = run_tracking_peak(50, 1, |value| {
        Some(if value % 2 == 0 { "a" } else { "b" }.to_owned())
    });
    assert_eq!(peak, 2, "two hosts at per_host=1 should overlap to 2");
    assert_eq!(
        par_map_per_host(Vec::<usize>::new(), 4, 8, |_| None, |value| value),
        Ok(Vec::<usize>::new())
    );
}

#[test]
fn par_map_per_host_bounds_hostless_items_by_global_only() {
    // No host: bounded only by the global ceiling.
    let peak = run_tracking_peak(3, 1, |_| None);
    assert_eq!(peak, 3, "hostless items ignore per_host, use global=3");
}

#[test]
fn omitted_concurrency_uses_new_defaults() {
    assert_eq!(super::resolve_jobs(None), 100);
    assert_eq!(super::resolve_per_host(None), 32);
    assert_eq!(super::resolve_jobs(Some(1)), 1);
    assert_eq!(super::resolve_per_host(Some(1)), 1);
}

#[test]
fn jobs_one_creates_only_one_worker_even_with_a_large_host_limit() {
    let worker_ids = std::sync::Mutex::new(std::collections::HashSet::new());
    let result = par_map_per_host(
        (0..256).collect::<Vec<_>>(),
        1,
        10_000,
        |_| Some("one.example".to_owned()),
        |value| {
            worker_ids
                .lock()
                .unwrap()
                .insert(std::thread::current().id());
            value
        },
    )
    .unwrap();
    assert_eq!(result, (0..256).collect::<Vec<_>>());
    assert_eq!(worker_ids.lock().unwrap().len(), 1);
}

#[test]
fn event_emitter_sequences_events_and_carries_progress() {
    let context = sample_context(false);
    let sink = CollectingSink::default();
    let emitter = EventEmitter::new(&context, &sink, 0);

    emitter.operation_started();
    emitter.member_started("mem_app", "repos/app");
    emitter.member_progress(
        "mem_app",
        "repos/app",
        crate::GitTransferProgress {
            phase: crate::GitProgressPhase::Receiving,
            received_objects: Some(5),
            total_objects: Some(10),
            received_bytes: Some(1024),
            indexed_deltas: None,
            total_deltas: None,
        },
    );
    emitter.member_finished("mem_app", "repos/app");
    emitter.operation_finished();

    let events = sink.take();
    assert_eq!(
        events.iter().map(|event| event.kind).collect::<Vec<_>>(),
        vec![
            crate::EventKind::OperationStarted,
            crate::EventKind::MemberStarted,
            crate::EventKind::MemberProgress,
            crate::EventKind::MemberFinished,
            crate::EventKind::OperationFinished,
        ]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(events[0].operation_id, "op_0001");
    assert_eq!(events[1].member_path.as_deref(), Some("repos/app"));
    let progress = events[2].progress.as_ref().expect("progress carried");
    assert_eq!(progress.phase, crate::GitProgressPhase::Receiving);
    assert_eq!(progress.received_objects, Some(5));
    assert!(events[3].progress.is_none());
}

#[test]
fn merge_state_change_event_carries_structured_state() {
    let context = sample_context(false);
    let sink = CollectingSink::default();
    let emitter = EventEmitter::new(&context, &sink, 0);

    emitter.operation_state_changed(crate::MergeOperationState::Finalizing);

    let events = sink.take();
    assert_eq!(events[0].kind, crate::EventKind::OperationStateChanged);
    assert_eq!(
        events[0].merge_state,
        Some(crate::MergeOperationState::Finalizing)
    );
}

#[test]
fn merge_outcome_and_artifact_events_carry_structured_payloads() {
    let context = sample_context(false);
    let sink = CollectingSink::default();
    let emitter = EventEmitter::new(&context, &sink, 0);
    let member = crate::MergeRepoSummary {
        target_id: "mem_app".to_owned(),
        path: "repos/app".to_owned(),
        state: crate::MergeParticipantState::Merged,
        ..crate::MergeRepoSummary::default()
    };

    emitter.artifact_written(".gwz/merge/merge_1.yaml");
    emitter.merge_member_finished(member.clone());

    let events = sink.take();
    assert_eq!(
        events.iter().map(|event| event.kind).collect::<Vec<_>>(),
        [
            crate::EventKind::ArtifactWritten,
            crate::EventKind::MemberFinished,
        ]
    );
    assert_eq!(
        events[0].artifact_path.as_deref(),
        Some(".gwz/merge/merge_1.yaml")
    );
    assert_eq!(events[1].merge_member.as_ref(), Some(&member));
}

#[test]
fn dispatch_context_preserves_status_request_meta() {
    let request = crate::StatusRequest {
        meta: crate::RequestMeta {
            request_id: "req-1".to_owned(),
            schema_version: "gwz.v0".to_owned(),
            dry_run: Some(true),
            attribution: Some(crate::OperationAttribution::from(&sample_attribution())),
            ..crate::RequestMeta::default()
        },
        ..Default::default()
    };

    let context = OperationRequest::Status(request)
        .context("op_0001")
        .expect("status context");

    assert_eq!(context.action, ActionKind::Status);
    assert_eq!(context.operation_id, "op_0001");
    assert_eq!(context.request_id, "req-1");
    assert!(context.dry_run);
    assert_eq!(
        context
            .attribution
            .as_ref()
            .unwrap()
            .actor
            .as_ref()
            .unwrap()
            .actor_id,
        "agent://local/session"
    );
}

#[test]
fn merge_context_rejects_invalid_author_and_committer_before_dispatch() {
    for identity_field in ["author", "committer"] {
        let invalid = crate::GitObjectIdentity {
            name: "Alice <work>".to_owned(),
            email: "alice@example.invalid".to_owned(),
            time_ms: None,
            timezone_offset_minutes: None,
        };
        let attribution = crate::OperationAttribution {
            actor: None,
            git_author: (identity_field == "author").then_some(invalid.clone()),
            git_committer: (identity_field == "committer").then_some(invalid),
            credential_ref: None,
        };
        let request = crate::MergeRequest {
            meta: crate::RequestMeta {
                request_id: "req-merge".to_owned(),
                schema_version: "gwz.v0".to_owned(),
                attribution: Some(attribution),
                ..Default::default()
            },
            op: crate::MergeOp::Start,
            source_ref: Some("feature/x".to_owned()),
            merge_id: None,
            mode: None,
            message: None,
            preserve: None,
            filesystem_strict: None,
            local_source_name: None,
            wait_seconds: None,
        };

        let error = OperationRequest::Merge(request)
            .context("op_merge")
            .unwrap_err();

        assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
        assert!(error.message.contains("name"), "{identity_field}: {error}");
    }
}

#[test]
fn member_lock_manager_serializes_mutating_member_access() {
    let locks = MemberLockManager::default();
    let member_id = MemberId::parse_str("mem_01").unwrap();
    let first = locks.try_lock(&member_id).expect("first lock");

    assert!(locks.try_lock(&member_id).is_none());
    drop(first);
    assert!(locks.try_lock(&member_id).is_some());
}

fn sample_context(dry_run: bool) -> OperationContext {
    OperationContext {
        operation_id: "op_0001".to_owned(),
        request_id: "req-1".to_owned(),
        schema_version: "gwz.v0".to_owned(),
        action: ActionKind::Status,
        dry_run,
        attribution: Some(sample_attribution()),
    }
}

fn sample_attribution() -> OperationAttribution {
    OperationAttribution {
        actor: Some(OperationActor::new("agent://local/session")),
        git_author: Some(GitObjectIdentity::new("Agent", "agent@example.invalid")),
        git_committer: Some(GitObjectIdentity::new("Bot", "bot@example.invalid")),
        credential_ref: Some("cred:test".to_owned()),
    }
}
