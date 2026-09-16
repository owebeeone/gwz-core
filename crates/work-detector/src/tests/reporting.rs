//! What the report says: unknown reasons, bounded diagnostics, hazard
//! order, verdict dominance and the force-name mapping.

use super::*;

/// W1 (LCM1.0c follow-up 2): a known observation may name paths it
/// could not establish. Each is an unknown reason in the report, with
/// its path and detail as reported, so the verdict is `Unknown` -- never
/// clean -- while the hazards found beside it are still listed.
#[test]
fn per_path_unknowns_in_a_known_observation_are_unknown_reasons_beside_the_hazards() {
    let observed = WorkObservation {
        entries: vec![entry("src/a.rs", WorkKind::Unstaged)],
        unknown: vec![UnknownReason {
            kind: UnknownKind::Unreadable,
            path: Some(path("vendor/blob.bin")),
            detail: "the worktree entry could not be read".to_owned(),
        }],
        ..WorkObservation::default()
    };
    let report = classify_work(&observed, &GwzEvidence::default());
    assert_eq!(report.verdict, WorkVerdict::Unknown);
    assert_eq!(kinds(&report), vec![HazardKind::Work(WorkKind::Unstaged)]);
    assert_eq!(unknown_kinds(&report), vec![UnknownKind::Unreadable]);
    assert_eq!(
        report.unknown[0].path.as_deref(),
        Some(b"vendor/blob.bin".as_slice())
    );
    assert!(report.unknown[0].detail.contains("could not be read"));

    // An empty list is the ordinary known observation: nothing changes.
    assert_eq!(
        classify_work(&observation(Vec::new()), &GwzEvidence::default()).verdict,
        WorkVerdict::Clean
    );
}

#[test]
fn truncated_diagnostics_keep_every_hazard() {
    let count = MAX_DETAILED_ENTRIES + 5;
    let entries = (0..count)
        .map(|index| entry(&format!("f{index}"), WorkKind::Untracked))
        .collect();
    let report = classify_work(&observation(entries), &GwzEvidence::default());
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(report.hazards.len(), count);
    assert!(report.truncated);
    assert!(!report.hazards[MAX_DETAILED_ENTRIES - 1].detail.is_empty());
    assert!(report.hazards[MAX_DETAILED_ENTRIES].detail.is_empty());
    assert_eq!(
        report.hazards[count - 1].path,
        Some(path(&format!("f{}", count - 1)))
    );
    assert!(
        report
            .hazards
            .iter()
            .all(|hazard| hazard.kind == HazardKind::Work(WorkKind::Untracked))
    );
}

#[test]
fn truncated_diagnostics_keep_every_unknown_reason() {
    let count = MAX_DETAILED_ENTRIES + 2;
    let entries = (0..count)
        .map(|index| {
            suppressed(
                &format!("u{index}"),
                SuppressionFlag::SkipWorktree,
                PhysicalState::Unobservable,
            )
        })
        .collect();
    let report = classify_work(&suppressed_observation(entries), &GwzEvidence::default());
    assert_eq!(report.verdict, WorkVerdict::Unknown);
    assert_eq!(report.unknown.len(), count);
    assert!(report.truncated);
    assert!(!report.unknown[MAX_DETAILED_ENTRIES - 1].detail.is_empty());
    assert!(report.unknown[MAX_DETAILED_ENTRIES].detail.is_empty());
    assert_eq!(
        report.unknown[count - 1].path,
        Some(path(&format!("u{}", count - 1)))
    );
}

// --- precedence and ordering -------------------------------------------

#[test]
fn unknown_dominates_dirty_and_the_hazards_are_still_listed() {
    let observed = WorkObservation {
        entries: vec![entry("a.rs", WorkKind::Staged)],
        suppressed: vec![suppressed(
            "b.rs",
            SuppressionFlag::SkipWorktree,
            PhysicalState::Unobservable,
        )],
        ..WorkObservation::default()
    };
    let report = classify_work(&observed, &GwzEvidence::default());
    assert_eq!(report.verdict, WorkVerdict::Unknown);
    assert_eq!(kinds(&report), vec![HazardKind::Work(WorkKind::Staged)]);
    assert_eq!(unknown_kinds(&report), vec![UnknownKind::Unreadable]);
}

#[test]
fn the_hazard_order_is_deterministic_and_follows_the_input() {
    let observed = WorkObservation {
        entries: vec![
            entry("a.rs", WorkKind::Staged),
            entry("b.rs", WorkKind::Untracked),
        ],
        suppressed: vec![suppressed(
            "c.rs",
            SuppressionFlag::AssumeUnchanged,
            PhysicalState::Differs,
        )],
        sparse_absent: Vec::new(),
        native_operation: Some(NativeOperation::Rebase),
        stash_entries: 1,
        unknown: Vec::new(),
    };
    let evidence = GwzEvidence {
        merge: EvidenceState::Open {
            detail: "open".to_owned(),
        },
        stash: EvidenceState::Open {
            detail: "open".to_owned(),
        },
        other: vec![EvidenceItem {
            kind: "exchange".to_owned(),
            state: EvidenceState::Open {
                detail: "open".to_owned(),
            },
        }],
    };
    let expected = vec![
        HazardKind::Work(WorkKind::Staged),
        HazardKind::Work(WorkKind::Untracked),
        HazardKind::Suppressed,
        HazardKind::OpenNativeOperation,
        HazardKind::NativeStash,
        HazardKind::OpenGwzMerge,
        HazardKind::OpenGwzStash,
        HazardKind::OpenGwzRecord,
    ];
    let first = classify_work(&observed, &evidence);
    let second = classify_work(&observed, &evidence);
    assert_eq!(kinds(&first), expected);
    assert_eq!(first, second);
    assert_eq!(first.verdict, WorkVerdict::Dirty);
}

// --- unknown observations ----------------------------------------------

#[test]
fn an_unreadable_entry_makes_the_whole_observation_unknown() {
    let reason = UnknownReason {
        kind: UnknownKind::Unreadable,
        path: Some(path("locked/dir")),
        detail: "permission denied".to_owned(),
    };
    let report = classify_observed_work(
        &Observation::Unknown(vec![reason.clone()]),
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Unknown);
    assert_ne!(report.verdict, WorkVerdict::Clean);
    assert_eq!(report.unknown, vec![reason]);
    assert!(report.hazards.is_empty());
}

#[test]
fn an_unknown_observation_still_reports_the_evidence_hazards() {
    let evidence = GwzEvidence {
        merge: EvidenceState::Open {
            detail: "open".to_owned(),
        },
        ..GwzEvidence::default()
    };
    let report = classify_observed_work(
        &Observation::Unknown(vec![UnknownReason::new(
            UnknownKind::LimitExceeded,
            "entry budget reached",
        )]),
        &evidence,
    );
    assert_eq!(report.verdict, WorkVerdict::Unknown);
    assert_eq!(kinds(&report), vec![HazardKind::OpenGwzMerge]);
    assert_eq!(unknown_kinds(&report), vec![UnknownKind::LimitExceeded]);
}

#[test]
fn a_known_observation_classifies_the_same_through_either_entry_point() {
    let observed = observation(vec![entry("a.rs", WorkKind::Ignored)]);
    let evidence = GwzEvidence::default();
    assert_eq!(
        classify_observed_work(&Observation::Known(observed.clone()), &evidence),
        classify_work(&observed, &evidence)
    );
}

// --- the disposal force vocabulary --------------------------------------

#[test]
fn every_hazard_kind_maps_onto_one_dispose_force_name() {
    for kind in [
        HazardKind::Work(WorkKind::Staged),
        HazardKind::Work(WorkKind::Ignored),
        HazardKind::Suppressed,
        HazardKind::NativeStash,
    ] {
        assert_eq!(kind.force_name(), Some("dirty"), "{kind:?}");
    }
    for kind in [
        HazardKind::OpenNativeOperation,
        HazardKind::OpenGwzMerge,
        HazardKind::OpenGwzStash,
        HazardKind::OpenGwzRecord,
    ] {
        assert_eq!(kind.force_name(), Some("open-merge"), "{kind:?}");
    }
    assert_eq!(HazardKind::UninterpretableEvidence.force_name(), None);
}

#[test]
fn a_hazard_with_no_force_name_never_stands_alone_as_dirty() {
    let evidence = GwzEvidence {
        merge: EvidenceState::Unknown {
            detail: "record version 9".to_owned(),
        },
        ..GwzEvidence::default()
    };
    let report = classify_work(
        &observation(vec![entry("a.rs", WorkKind::Staged)]),
        &evidence,
    );
    assert!(
        report
            .hazards
            .iter()
            .any(|hazard| hazard.kind.force_name().is_none())
    );
    assert_eq!(report.verdict, WorkVerdict::Unknown);
    assert!(!report.unknown.is_empty());
}
