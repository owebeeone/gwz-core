use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

#[cfg(test)]
use super::*;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
pub struct OperationRuntime {
    pub(crate) records: Arc<Mutex<HashMap<String, Arc<OperationRecord>>>>,
    pub(crate) event_capacity: usize,
}

pub(crate) struct OperationRecord {
    pub(crate) state: Mutex<OperationState>,
    pub(crate) complete: Condvar,
}

pub(crate) struct OperationState {
    pub(crate) events: VecDeque<crate::OperationEvent>,
    pub(crate) event_capacity: usize,
    pub(crate) next_sequence: i64,
    pub(crate) result: Option<crate::OperationResult>,
}

#[cfg(test)]
mod tests {
    use crate::model::{
        GitObjectIdentity, MemberId, OperationActor, OperationAttribution, SourceKind,
    };
    use crate::runtime::clock::TimestampMs;

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
        assert_eq!(results, (0..8).map(|value| value * 10).collect::<Vec<_>>());
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
            Vec::new()
        );
    }

    #[test]
    fn par_map_per_host_bounds_hostless_items_by_global_only() {
        // No host: bounded only by the global ceiling.
        let peak = run_tracking_peak(3, 1, |_| None);
        assert_eq!(peak, 3, "hostless items ignore per_host, use global=3");
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
    fn dry_run_plan_reports_member_plans_without_execution() {
        let context = sample_context(true);
        let plan = OperationPlan {
            operation_id: context.operation_id.clone(),
            action: ActionKind::Status,
            dry_run: context.dry_run,
            members: vec![MemberPlan {
                member_id: Some(MemberId::parse_str("mem_01").unwrap()),
                member_path: "repos/example".to_owned(),
                source_kind: SourceKind::Git,
                action: PlannedAction::Noop,
                requires_mutation: false,
                message: Some("status only".to_owned()),
            }],
        };

        assert!(plan.dry_run);
        assert!(!plan.requires_mutation());
        assert_eq!(plan.members[0].action, PlannedAction::Noop);
    }

    #[test]
    fn accepted_response_carries_operation_id_and_attribution() {
        let context = sample_context(false);
        let response = ResponseBuilder::accepted(&context, &[]);

        assert_eq!(response.meta.operation_id.as_deref(), Some("op_0001"));
        assert_eq!(response.meta.request_id, "req-1");
        assert_eq!(response.meta.action, crate::ActionKind::Status);
        assert_eq!(
            response.meta.aggregate_status,
            crate::AggregateStatus::Accepted
        );
        assert_eq!(
            response
                .meta
                .attribution
                .as_ref()
                .and_then(|value| value.actor.as_ref())
                .map(|actor| actor.actor_id.as_str()),
            Some("agent://local/session")
        );
    }

    #[test]
    fn execution_report_assembles_final_operation_result() {
        let context = sample_context(false);
        let report = ExecutionReport {
            members: vec![MemberExecution {
                member_id: Some(MemberId::parse_str("mem_01").unwrap()),
                member_path: "repos/example".to_owned(),
                source_kind: SourceKind::Git,
                status: MemberExecutionStatus::Rejected,
                error: Some(OperationError::new(
                    crate::model::ErrorCode::DivergedMember,
                    "member diverged",
                )),
            }],
            errors: vec![OperationError::new(
                crate::model::ErrorCode::DivergedMember,
                "member diverged",
            )],
        };

        let result = ResponseBuilder::result(&context, &report, TimestampMs(10), TimestampMs(20));

        assert_eq!(result.operation_id, "op_0001");
        assert_eq!(result.aggregate_status, crate::AggregateStatus::Rejected);
        assert_eq!(result.members[0].status, crate::MemberStatus::Rejected);
        assert_eq!(result.errors[0].code, crate::GwzErrorCode::DivergedMember);
        assert_eq!(
            result
                .attribution
                .as_ref()
                .and_then(|value| value.git_committer.as_ref())
                .map(|identity| identity.email.as_str()),
            Some("bot@example.invalid")
        );
    }

    #[test]
    fn partial_outcome_and_member_error_identity_survive_to_the_result() {
        let context = sample_context(false);
        let report = ExecutionReport {
            members: vec![
                MemberExecution {
                    member_id: Some(MemberId::parse_str("mem_01").unwrap()),
                    member_path: "repos/ok".to_owned(),
                    source_kind: SourceKind::Git,
                    status: MemberExecutionStatus::Ok,
                    error: None,
                },
                MemberExecution {
                    member_id: Some(MemberId::parse_str("mem_02").unwrap()),
                    member_path: "repos/bad".to_owned(),
                    source_kind: SourceKind::Git,
                    status: MemberExecutionStatus::Failed,
                    error: Some(OperationError::new(
                        crate::model::ErrorCode::GitCommandFailed,
                        "boom",
                    )),
                },
            ],
            errors: Vec::new(),
        };

        let result = ResponseBuilder::result(&context, &report, TimestampMs(10), TimestampMs(20));

        // F15: an applied + failed mix is Partial, not a blanket Failed.
        assert_eq!(result.aggregate_status, crate::AggregateStatus::Partial);
        // F15: the failed member's error keeps its identity instead of dropping it.
        let failed = result
            .members
            .iter()
            .find(|member| member.member_id == "mem_02")
            .expect("failed member present");
        let error = failed.error.as_ref().expect("member error present");
        assert_eq!(error.member_id.as_deref(), Some("mem_02"));
        assert_eq!(error.member_path.as_deref(), Some("repos/bad"));
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
    fn submit_returns_accepted_before_handler_finishes() {
        let runtime = OperationRuntime::new(8);
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let response = runtime
            .submit(sample_context(false), move |_context, _sink| {
                release_rx.recv().unwrap();
                ExecutionReport::default()
            })
            .unwrap();

        assert_eq!(
            response.meta.aggregate_status,
            crate::AggregateStatus::Accepted
        );
        assert_eq!(response.meta.operation_id.as_deref(), Some("op_0001"));
        assert!(runtime.try_result("op_0001").unwrap().is_none());

        release_tx.send(()).unwrap();
        assert_eq!(
            runtime.wait("op_0001").unwrap().aggregate_status,
            crate::AggregateStatus::Noop
        );
    }

    #[test]
    fn subscriber_receives_events_and_wait_does_not_require_drain() {
        let runtime = OperationRuntime::new(8);
        runtime
            .submit(sample_context(false), |_context, sink| {
                sink.emit(
                    crate::EventKind::MemberProgress,
                    crate::Severity::Info,
                    Some("mem_01".to_owned()),
                    Some("repos/example".to_owned()),
                    Some("checking status".to_owned()),
                );
                ExecutionReport::default()
            })
            .unwrap();
        let mut subscription = runtime.subscribe("op_0001").unwrap();

        let result = runtime.wait("op_0001").unwrap();
        let events = subscription.drain();

        assert_eq!(result.aggregate_status, crate::AggregateStatus::Noop);
        assert!(
            events
                .iter()
                .any(|event| event.kind == crate::EventKind::OperationStarted)
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == crate::EventKind::MemberProgress)
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == crate::EventKind::OperationFinished)
        );
        assert_eq!(
            events
                .first()
                .and_then(|event| event.attribution.as_ref())
                .and_then(|attribution| attribution.actor.as_ref())
                .map(|actor| actor.actor_id.as_str()),
            Some("agent://local/session")
        );
    }

    #[test]
    fn event_buffer_overflow_emits_reset_and_preserves_result() {
        let runtime = OperationRuntime::new(3);
        runtime
            .submit(sample_context(false), |_context, sink| {
                for index in 0..10 {
                    sink.emit(
                        crate::EventKind::MemberProgress,
                        crate::Severity::Info,
                        Some("mem_01".to_owned()),
                        Some("repos/example".to_owned()),
                        Some(format!("event {index}")),
                    );
                }
                ExecutionReport::default()
            })
            .unwrap();
        let mut subscription = runtime.subscribe("op_0001").unwrap();

        let result = runtime.wait("op_0001").unwrap();
        let events = subscription.drain();

        assert_eq!(result.operation_id, "op_0001");
        assert!(
            events
                .iter()
                .any(|event| event.kind == crate::EventKind::Reset)
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == crate::EventKind::OperationFinished)
        );
    }

    #[test]
    fn event_sequence_numbers_are_monotonic() {
        let runtime = OperationRuntime::new(16);
        runtime
            .submit(sample_context(false), |_context, sink| {
                sink.emit(
                    crate::EventKind::MemberStarted,
                    crate::Severity::Info,
                    None,
                    None,
                    None,
                );
                sink.emit(
                    crate::EventKind::MemberFinished,
                    crate::Severity::Info,
                    None,
                    None,
                    None,
                );
                ExecutionReport::default()
            })
            .unwrap();
        let mut subscription = runtime.subscribe("op_0001").unwrap();

        runtime.wait("op_0001").unwrap();
        let events = subscription.drain();

        assert!(
            events
                .windows(2)
                .all(|window| window[0].sequence < window[1].sequence)
        );
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
}
