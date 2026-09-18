//! What a caller-established [`CopyBaseline`] changes, and what it must
//! never change (`GwzLaneCleanFixes.md` R2, R8; plan S1.5).

use super::*;

fn copied_and_lane_made() -> WorkObservation {
    WorkObservation {
        entries: vec![
            entry("target/", WorkKind::Ignored),
            entry(".venv/", WorkKind::Ignored),
            entry("notes.txt", WorkKind::Untracked),
        ],
        stash_entries: 2,
        ..WorkObservation::default()
    }
}

/// With no baseline nothing is established, so every hazard is the lane's
/// own: the classification every gwz before provenance made.
#[test]
fn no_baseline_leaves_every_hazard_the_lanes_own() {
    let report = classify_work(&copied_and_lane_made(), &GwzEvidence::default());
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(report.hazards.len(), 4);
    assert!(
        report
            .hazards
            .iter()
            .all(|hazard| hazard.provenance == Provenance::Unique),
        "{}",
        details(&report)
    );
    assert_eq!(
        classify_work_against(
            &copied_and_lane_made(),
            &GwzEvidence::default(),
            &CopyBaseline::default()
        ),
        report,
        "an empty baseline is the same as none"
    );
}

/// The copy's own unchanged data is still *reported* -- the refusal names
/// it under its category (S1.7) -- and it no longer refuses. The lane's own
/// untracked file and the changed copy still do.
#[test]
fn an_unchanged_copy_is_reported_and_does_not_refuse() {
    let mut baseline = CopyBaseline::default();
    baseline.set(path("target/"), Provenance::UnchangedCopy);
    baseline.set(path(".venv/"), Provenance::ChangedCopy);
    let report = classify_work_against(&copied_and_lane_made(), &GwzEvidence::default(), &baseline);
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(
        report
            .hazards
            .iter()
            .map(|hazard| hazard.provenance)
            .collect::<Vec<_>>(),
        vec![
            Provenance::UnchangedCopy,
            Provenance::ChangedCopy,
            Provenance::Unique,
            Provenance::Unique,
        ],
        "{}",
        details(&report)
    );
    assert_eq!(
        kinds(&report).len(),
        4,
        "no hazard is dropped: the report is the product"
    );
    // The waiver spelling is untouched: narrowing it is R11, Phase 3.
    assert!(
        report
            .hazards
            .iter()
            .all(|hazard| hazard.kind.force_name().is_some())
    );
}

/// A lane whose every hazard is the copy's, unchanged and still in the
/// family, is clean: R0's whole point. Nothing is hidden -- the hazards are
/// all still listed.
#[test]
fn a_lane_holding_only_unchanged_copies_is_clean_and_still_reports_them() {
    let mut baseline = CopyBaseline::default();
    for name in ["target/", ".venv/", "notes.txt"] {
        baseline.set(path(name), Provenance::UnchangedCopy);
    }
    baseline.set_stash(Provenance::UnchangedCopy);
    let report = classify_work_against(&copied_and_lane_made(), &GwzEvidence::default(), &baseline);
    assert_eq!(report.verdict, WorkVerdict::Clean);
    assert_eq!(report.hazards.len(), 4, "{}", details(&report));
    assert!(report.unknown.is_empty());
}

/// The native stash entries are one hazard, so they take one provenance:
/// the copied stash no longer refuses, a stash the lane made does.
#[test]
fn the_native_stash_takes_the_baselines_answer() {
    let observation = WorkObservation {
        stash_entries: 3,
        ..WorkObservation::default()
    };
    let mut copied = CopyBaseline::default();
    copied.set_stash(Provenance::UnchangedCopy);
    let report = classify_work_against(&observation, &GwzEvidence::default(), &copied);
    assert_eq!(report.verdict, WorkVerdict::Clean);
    assert_eq!(kinds(&report), vec![HazardKind::NativeStash]);
    assert_eq!(report.hazards[0].provenance, Provenance::UnchangedCopy);

    let report = classify_work_against(
        &observation,
        &GwzEvidence::default(),
        &CopyBaseline::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
}

/// A baseline never softens anything but worktree data: an open record, an
/// unfinished native operation and a suppressed path are the lane's state,
/// not data it inherited, and an unknown reason still dominates.
#[test]
fn a_baseline_softens_no_state_and_never_beats_unknown() {
    let mut baseline = CopyBaseline::default();
    baseline.set(path("src/main.rs"), Provenance::UnchangedCopy);
    baseline.set_stash(Provenance::UnchangedCopy);
    let observation = WorkObservation {
        suppressed: vec![suppressed(
            "src/main.rs",
            SuppressionFlag::AssumeUnchanged,
            PhysicalState::Differs,
        )],
        native_operation: Some(NativeOperation::Rebase),
        ..WorkObservation::default()
    };
    let report = classify_work_against(&observation, &GwzEvidence::default(), &baseline);
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert!(
        report
            .hazards
            .iter()
            .all(|hazard| hazard.provenance == Provenance::Unique),
        "{}",
        details(&report)
    );

    let unknown = Observation::Unknown(vec![UnknownReason::new(UnknownKind::Unreadable, "EIO")]);
    let report = classify_observed_work_against(&unknown, &GwzEvidence::default(), &baseline);
    assert_eq!(report.verdict, WorkVerdict::Unknown);
}
