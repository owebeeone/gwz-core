//! Engine shape parity — the same fixture, once on v0 and once on v1.
//!
//! **M5d charter §4.** "Same kinds, same ordering discipline, same
//! per-participant count as v0 for the same fixture; message text may differ.
//! Continue and abort arms get the same treatment." M5d raises
//! `ACTIVE_WRITER_FLOOR` to `V1`, so every ordinary merge becomes the `--no-ff`
//! merge these rows drive today. The defect class this suite exists to catch is
//! the 0.8.0 one named in `GwzM5-8M5d-ParityInventory.md`: a behaviour present
//! on the old path, silently absent on the new one, asserted by nothing.
//!
//! The comparison is deliberately shape-only. Ids, commits and marker names
//! differ between two runs of the same fixture, so events are normalized to
//! (kind, member, artifact class, merge state, participant outcome) before the
//! two streams are compared; messages are never compared, exactly as the
//! charter allows.

use super::*;

/// The comparable shape of one event: everything the charter pins, nothing
/// that a second run of the same fixture is entitled to change.
#[derive(Clone, Debug, PartialEq)]
struct EventShape {
    kind: crate::EventKind,
    member_id: Option<String>,
    artifact: Option<String>,
    merge_state: Option<crate::MergeOperationState>,
    member: Option<MemberShape>,
}

#[derive(Clone, Debug, PartialEq)]
struct MemberShape {
    target_id: String,
    path: String,
    state: crate::MergeParticipantState,
    integrated: bool,
    conflict_paths: usize,
}

/// Artifact paths carry a merge id, a commit and a marker uuid. Class them.
fn artifact_class(path: &str) -> String {
    if path.starts_with(".gwz/merge/done/") {
        return ".gwz/merge/done/<merge>.yaml".to_owned();
    }
    if path.starts_with(".gwz/merge/") {
        return ".gwz/merge/<merge>.yaml".to_owned();
    }
    if path.starts_with("git:@root/") {
        return "git:@root/<commit>".to_owned();
    }
    if path.starts_with(&format!("{}/", crate::artifact::MARKER_DIR)) {
        return format!("{}/<marker>.yaml", crate::artifact::MARKER_DIR);
    }
    path.to_owned()
}

fn shapes(sink: &CollectingSink) -> Vec<EventShape> {
    sink.take()
        .into_iter()
        .map(|event| EventShape {
            kind: event.kind,
            member_id: event.member_id,
            artifact: event.artifact_path.as_deref().map(artifact_class),
            merge_state: event.merge_state,
            member: event.merge_member.map(|member| MemberShape {
                target_id: member.target_id,
                path: member.path,
                state: member.state,
                integrated: member.resulting_commit.is_some(),
                conflict_paths: member.conflict_paths.len(),
            }),
        })
        .collect()
}

/// One `member_started` and one `merge_member_finished` per participant, and
/// the started one first — the discipline `start/execution.rs:22, 59` sets.
fn member_event_order(shapes: &[EventShape]) -> Vec<(crate::EventKind, String)> {
    shapes
        .iter()
        .filter(|shape| {
            matches!(
                shape.kind,
                crate::EventKind::MemberStarted | crate::EventKind::MemberFinished
            )
        })
        .map(|shape| {
            (
                shape.kind,
                shape.member_id.clone().unwrap_or_else(|| "?".to_owned()),
            )
        })
        .collect()
}

fn counts_by_kind(shapes: &[EventShape]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for shape in shapes {
        let key = match shape.artifact.as_deref() {
            // Counted by `record_writes`, not compared: see `stream`.
            Some(OPEN_RECORD) => continue,
            Some(artifact) => format!("ArtifactWritten({artifact})"),
            None => format!("{:?}", shape.kind),
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

const OPEN_RECORD: &str = ".gwz/merge/<merge>.yaml";

/// The comparable stream: everything but the open record's own writes.
///
/// The two engines checkpoint at their own granularity. v0 persists once per
/// `persist_merge_record` call; v1 commits once per transition of its
/// state machine and additionally journals a pending action durably before
/// each physical mutation — the crash-recoverable journal is the whole point
/// of the v1 record. So the NUMBER of `.gwz/merge/<id>.yaml` writes is an
/// engine property the charter does not pin: it asks for "same kinds, same
/// ordering DISCIPLINE, same per-participant count". This projection drops
/// those writes so the two orders can be compared exactly, and
/// `assert_reports_follow_their_write` puts the discipline back — no reported
/// outcome, state change or publication artifact appears except immediately
/// after a durable record write.
fn stream(shapes: &[EventShape]) -> Vec<EventShape> {
    shapes
        .iter()
        .filter(|shape| shape.artifact.as_deref() != Some(OPEN_RECORD))
        .cloned()
        .collect()
}

/// **E-8, the write-before-event pin, on whichever engine produced this
/// stream.** `merge/store/persistence.rs:16-17` is the v0 statement of it: the
/// record write, then the event that reports it. Every participant outcome,
/// every durable state change and the first of the four publication artifacts
/// must therefore sit immediately behind a record write.
fn assert_reports_follow_their_write(shapes: &[EventShape], label: &str) {
    let mut previous: Option<&EventShape> = None;
    let mut publication_started = false;
    for shape in shapes {
        let reports_a_write = match shape.artifact.as_deref() {
            Some(OPEN_RECORD) | Some(".gwz/merge/done/<merge>.yaml") => false,
            Some(_) => !std::mem::replace(&mut publication_started, true),
            None => matches!(
                shape.kind,
                crate::EventKind::MemberFinished | crate::EventKind::OperationStateChanged
            ),
        };
        if reports_a_write {
            assert_eq!(
                previous.and_then(|event| event.artifact.clone()).as_deref(),
                Some(OPEN_RECORD),
                "{label}: {shape:?} was reported without the durable write in front of it"
            );
        }
        previous = Some(shape);
    }
}

/// The record's own writes, which the two engines are entitled to space
/// differently — but both must make at least one.
fn record_writes(shapes: &[EventShape]) -> usize {
    shapes
        .iter()
        .filter(|shape| shape.artifact.as_deref() == Some(OPEN_RECORD))
        .count()
}

/// A member whose `main` and `feature/source` both moved off a shared base, on
/// DIFFERENT files. Both engines must do a true merge here, so the two runs
/// are comparable participant state by participant state.
fn diverged_workspace(label: &str) -> (TempDir, crate::git::Git2Backend) {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, label);
    let member = temp.path().join("remote");
    let (base, _source) = feature_commit(&backend, &member, "SOURCE.md", "source\n");
    commit_file(
        &member,
        "LOCAL.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    (temp, backend)
}

/// The same member, but both sides edit the same file: both engines stop at
/// `AwaitingResolution`.
fn conflicted_workspace(label: &str) -> (TempDir, crate::git::Git2Backend) {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, label);
    let member = temp.path().join("remote");
    let (base, _source) = feature_commit(&backend, &member, "README.md", "source\n");
    commit_file(
        &member,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    (temp, backend)
}

fn no_ff_request() -> crate::MergeRequest {
    crate::MergeRequest {
        mode: Some(crate::MergeMode::NoFf),
        ..request(false)
    }
}

/// Everything a response says about its rows, minus the values a second run is
/// entitled to change.
#[derive(Debug, PartialEq)]
struct ResponseShape {
    state: crate::MergeOperationState,
    open: bool,
    counts: crate::MergeParticipantCounts,
    publication_step: Option<crate::MergePublicationStep>,
    preservation: Option<usize>,
    rows: Vec<RowShape>,
}

#[derive(Debug, PartialEq)]
struct RowShape {
    target_id: String,
    path: String,
    source_ref: String,
    target_branch: String,
    state: crate::MergeParticipantState,
    integrated: bool,
    live_commit_present: bool,
    predicted: Option<crate::MergeAnalysisKind>,
    prediction_complete: Option<bool>,
    conflict_paths: usize,
    has_error: bool,
}

fn response_shape(response: &crate::MergeResponse) -> ResponseShape {
    ResponseShape {
        state: response.state,
        open: response.open,
        counts: response.participant_counts.clone(),
        publication_step: response.publication_step,
        preservation: response.preservation.as_ref().map(Vec::len),
        rows: response
            .repos
            .iter()
            .map(|repo| RowShape {
                target_id: repo.target_id.clone(),
                path: repo.path.clone(),
                source_ref: repo.source_ref.clone(),
                target_branch: repo.target_branch.clone(),
                state: repo.state,
                integrated: repo.resulting_commit.is_some(),
                live_commit_present: repo.live_commit.is_some(),
                predicted: repo.predicted,
                prediction_complete: repo.prediction_complete,
                conflict_paths: repo.conflict_paths.len(),
                has_error: repo.error.is_some(),
            })
            .collect(),
    }
}

/// **The parity row.** One clean two-participant-state fixture, merged
/// ordinarily (v0 today) and with `--no-ff` (v1), with the two event streams
/// and the two completed responses compared shape for shape.
///
/// This is the row that would have caught all four defects the M5d parity
/// inventory ranked at 2: absent `member_started` / `merge_member_finished`
/// (E-2, E-3), a single `operation_state_changed` (E-7), no `artifact_written`
/// at all (E-4, E-5, E-6), and a completed response with `repos: []` and
/// default counts (R-1…R-4).
#[test]
fn an_ordinary_and_a_no_ff_merge_report_the_same_shape() {
    let (v0_temp, v0_backend) = diverged_workspace("parity-ordinary");
    let (v1_temp, v1_backend) = diverged_workspace("parity-no-ff");
    let v0_sink = CollectingSink::default();
    let v1_sink = CollectingSink::default();

    let ordinary = crate::workspace_ops::handle_merge_with_events(
        &v0_backend,
        v0_temp.path(),
        request(false),
        "op_parity_ordinary",
        &v0_sink,
    )
    .unwrap();
    let no_ff = crate::workspace_ops::handle_merge_with_events(
        &v1_backend,
        v1_temp.path(),
        no_ff_request(),
        "op_parity_no_ff",
        &v1_sink,
    )
    .unwrap();

    // The two runs really are the two engines.
    assert_eq!(
        record_envelope(v0_temp.path(), ordinary.merge_id.as_deref().unwrap()),
        ("gwz.merge-operation/v0".to_owned(), 0)
    );
    assert_eq!(
        record_envelope(v1_temp.path(), no_ff.merge_id.as_deref().unwrap()),
        ("gwz.merge-operation/v1".to_owned(), 1)
    );

    let ordinary_events = shapes(&v0_sink);
    let no_ff_events = shapes(&v1_sink);
    if std::env::var_os("GWZ_PARITY_DUMP").is_some() {
        for (label, events) in [("v0", &ordinary_events), ("v1", &no_ff_events)] {
            for event in events.iter() {
                println!("{label}: {event:?}");
            }
        }
        println!("v0 counts {:?}", counts_by_kind(&ordinary_events));
        println!("v1 counts {:?}", counts_by_kind(&no_ff_events));
    }

    assert_eq!(
        counts_by_kind(&no_ff_events),
        counts_by_kind(&ordinary_events),
        "same event kinds, and the same count of every artifact but the record"
    );
    assert_eq!(
        member_event_order(&no_ff_events),
        member_event_order(&ordinary_events),
        "same per-participant events, in the same order"
    );
    assert_eq!(
        stream(&no_ff_events),
        stream(&ordinary_events),
        "same kinds, same order, same payload shape"
    );
    assert_reports_follow_their_write(&ordinary_events, "ordinary");
    assert_reports_follow_their_write(&no_ff_events, "no-ff");
    assert!(record_writes(&no_ff_events) > 0);
    assert_eq!(response_shape(&no_ff), response_shape(&ordinary));

    // The four defects, each named where it would show.
    assert_eq!(no_ff.participant_counts.total, 1, "R-1/R-2: rows are present");
    assert_eq!(no_ff.participant_counts.merged, 1);
    let row = merge_repo(&no_ff, "mem_remote");
    assert!(row.predicted.is_some(), "R-6: `predicted` is decorated");
    assert!(
        row.prediction_complete.is_some(),
        "R-6: `prediction_complete` is decorated"
    );
    assert!(row.live_commit.is_some(), "R-7: `live_commit` is decorated");
    assert_eq!(
        publication_artifacts(&no_ff_events).len(),
        4,
        "E-6: the four publication artifacts, in the documented order"
    );
    assert_eq!(
        no_ff_events
            .iter()
            .filter(|shape| shape.artifact.as_deref() == Some(".gwz/merge/done/<merge>.yaml"))
            .count(),
        1,
        "E-5: the archive is reported once"
    );
}

/// The four composition-evidence artifacts, in stream order.
/// `gwz-cli/docs/MachineOutput.md:396-406` pins both the set and the order.
fn publication_artifacts(shapes: &[EventShape]) -> Vec<String> {
    let expected = [
        "git:@root/<commit>",
        &format!("{}/<marker>.yaml", crate::artifact::MARKER_DIR),
        crate::artifact::LOCK_PATH,
        ".git/info/exclude",
    ]
    .map(str::to_owned);
    let found = shapes
        .iter()
        .filter_map(|shape| shape.artifact.clone())
        .filter(|artifact| expected.contains(artifact))
        .collect::<Vec<_>>();
    assert_eq!(found, expected.to_vec(), "documented publication order");
    found
}

/// The conflicted arm, then the continue arm — both engines, both compared.
#[test]
fn a_conflicted_start_and_its_continue_report_the_same_shape() {
    let (v0_temp, v0_backend) = conflicted_workspace("parity-conflict-ordinary");
    let (v1_temp, v1_backend) = conflicted_workspace("parity-conflict-no-ff");
    let v0_start = CollectingSink::default();
    let v1_start = CollectingSink::default();

    let ordinary = crate::workspace_ops::handle_merge_with_events(
        &v0_backend,
        v0_temp.path(),
        request(false),
        "op_parity_conflict_ordinary",
        &v0_start,
    )
    .unwrap();
    let no_ff = crate::workspace_ops::handle_merge_with_events(
        &v1_backend,
        v1_temp.path(),
        no_ff_request(),
        "op_parity_conflict_no_ff",
        &v1_start,
    )
    .unwrap();
    assert_eq!(ordinary.state, crate::MergeOperationState::AwaitingResolution);
    assert_eq!(no_ff.state, crate::MergeOperationState::AwaitingResolution);
    assert_eq!(stream(&shapes(&v1_start)), stream(&shapes(&v0_start)));
    assert_eq!(
        counts_by_kind(&shapes(&v1_start)),
        counts_by_kind(&shapes(&v0_start))
    );
    assert_eq!(
        member_event_order(&shapes(&v1_start)),
        member_event_order(&shapes(&v0_start))
    );
    assert_reports_follow_their_write(&shapes(&v0_start), "conflicted ordinary start");
    assert_reports_follow_their_write(&shapes(&v1_start), "conflicted no-ff start");
    assert_eq!(response_shape(&no_ff), response_shape(&ordinary));

    // Resolve identically on both sides, then continue on both engines.
    for temp in [v0_temp.path(), v1_temp.path()] {
        fs::write(temp.join("remote/README.md"), "resolved\n").unwrap();
    }
    let v0_continue = CollectingSink::default();
    let v1_continue = CollectingSink::default();
    for (backend, temp, sink, id, operation) in [
        (
            &v0_backend,
            v0_temp.path(),
            &v0_continue,
            ordinary.merge_id.clone(),
            "op_parity_stage_ordinary",
        ),
        (
            &v1_backend,
            v1_temp.path(),
            &v1_continue,
            no_ff.merge_id.clone(),
            "op_parity_stage_no_ff",
        ),
    ] {
        handle_stage(
            backend,
            temp,
            crate::StageRequest {
                meta: request_meta(),
                cwd: temp.to_string_lossy().into_owned(),
                pathspecs: vec!["remote/README.md".to_owned()],
                all: None,
            },
            operation,
        )
        .unwrap();
        let _ = (sink, id);
    }

    let ordinary_done = crate::workspace_ops::handle_merge_with_events(
        &v0_backend,
        v0_temp.path(),
        recovery_request(crate::MergeOp::Resume, ordinary.merge_id.clone()),
        "op_parity_continue_ordinary",
        &v0_continue,
    )
    .unwrap();
    let no_ff_done = crate::workspace_ops::handle_merge_with_events(
        &v1_backend,
        v1_temp.path(),
        recovery_request(crate::MergeOp::Resume, no_ff.merge_id.clone()),
        "op_parity_continue_no_ff",
        &v1_continue,
    )
    .unwrap();

    assert_eq!(ordinary_done.state, crate::MergeOperationState::Completed);
    assert_eq!(no_ff_done.state, crate::MergeOperationState::Completed);
    if std::env::var_os("GWZ_PARITY_DUMP").is_some() {
        for (label, sink) in [("v0", &v0_continue), ("v1", &v1_continue)] {
            for event in shapes(sink) {
                println!("continue {label}: {event:?}");
            }
        }
    }
    assert_eq!(stream(&shapes(&v1_continue)), stream(&shapes(&v0_continue)));
    assert_eq!(
        counts_by_kind(&shapes(&v1_continue)),
        counts_by_kind(&shapes(&v0_continue))
    );
    assert_eq!(
        member_event_order(&shapes(&v1_continue)),
        member_event_order(&shapes(&v0_continue))
    );
    assert_reports_follow_their_write(&shapes(&v0_continue), "ordinary continue");
    assert_reports_follow_their_write(&shapes(&v1_continue), "no-ff continue");
    assert_eq!(response_shape(&no_ff_done), response_shape(&ordinary_done));
}

/// The abort arm, both engines.
#[test]
fn an_aborted_merge_reports_the_same_shape_on_both_engines() {
    let (v0_temp, v0_backend) = conflicted_workspace("parity-abort-ordinary");
    let (v1_temp, v1_backend) = conflicted_workspace("parity-abort-no-ff");

    let ordinary = crate::workspace_ops::handle_merge_with_events(
        &v0_backend,
        v0_temp.path(),
        request(false),
        "op_parity_abort_start_ordinary",
        &CollectingSink::default(),
    )
    .unwrap();
    let no_ff = crate::workspace_ops::handle_merge_with_events(
        &v1_backend,
        v1_temp.path(),
        no_ff_request(),
        "op_parity_abort_start_no_ff",
        &CollectingSink::default(),
    )
    .unwrap();

    let v0_sink = CollectingSink::default();
    let v1_sink = CollectingSink::default();
    let ordinary_aborted = crate::workspace_ops::handle_merge_with_events(
        &v0_backend,
        v0_temp.path(),
        recovery_request(crate::MergeOp::Abort, ordinary.merge_id.clone()),
        "op_parity_abort_ordinary",
        &v0_sink,
    )
    .unwrap();
    let no_ff_aborted = crate::workspace_ops::handle_merge_with_events(
        &v1_backend,
        v1_temp.path(),
        recovery_request(crate::MergeOp::Abort, no_ff.merge_id.clone()),
        "op_parity_abort_no_ff",
        &v1_sink,
    )
    .unwrap();

    assert_eq!(ordinary_aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(no_ff_aborted.state, crate::MergeOperationState::Aborted);
    if std::env::var_os("GWZ_PARITY_DUMP").is_some() {
        for (label, sink) in [("v0", &v0_sink), ("v1", &v1_sink)] {
            for event in shapes(sink) {
                println!("abort {label}: {event:?}");
            }
        }
    }
    assert_eq!(stream(&shapes(&v1_sink)), stream(&shapes(&v0_sink)));
    assert_eq!(
        counts_by_kind(&shapes(&v1_sink)),
        counts_by_kind(&shapes(&v0_sink))
    );
    assert_eq!(
        member_event_order(&shapes(&v1_sink)),
        member_event_order(&shapes(&v0_sink))
    );
    assert_reports_follow_their_write(&shapes(&v0_sink), "ordinary abort");
    assert_reports_follow_their_write(&shapes(&v1_sink), "no-ff abort");
    assert_eq!(response_shape(&no_ff_aborted), response_shape(&ordinary_aborted));
}

/// The exact `(schema, record_schema_version)` pair on disk, wherever the
/// record currently lives.
fn record_envelope(root: &Path, merge_id: &str) -> (String, u64) {
    let open = root.join(format!(".gwz/merge/{merge_id}.yaml"));
    let done = root.join(format!(".gwz/merge/done/{merge_id}.yaml"));
    let path = if open.exists() { open } else { done };
    let text = fs::read_to_string(&path).unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    (
        value["schema"].as_str().unwrap().to_owned(),
        value["record_schema_version"].as_u64().unwrap(),
    )
}
