//! Open native operations and decoded GWZ records, including the
//! uninterpretable and external-layout evidence that is never clean.

use super::*;

#[test]
fn an_unfinished_native_operation_is_an_open_operation_hazard() {
    for (operation, word) in [
        (NativeOperation::Merge, "merge"),
        (NativeOperation::Rebase, "rebase"),
        (NativeOperation::CherryPick, "cherry-pick"),
        (NativeOperation::Revert, "revert"),
        (NativeOperation::Bisect, "bisect"),
        (NativeOperation::ApplyMailbox, "mailbox"),
        (NativeOperation::Other, "operation"),
    ] {
        let report = classify_work(
            &WorkObservation {
                native_operation: Some(operation),
                ..WorkObservation::default()
            },
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty, "{operation:?}");
        assert_eq!(kinds(&report), vec![HazardKind::OpenNativeOperation]);
        assert!(
            report.hazards[0].detail.contains(word),
            "{operation:?}: {}",
            report.hazards[0].detail
        );
    }
}

#[test]
fn native_stash_entries_are_a_hazard() {
    let report = classify_work(
        &WorkObservation {
            stash_entries: 3,
            ..WorkObservation::default()
        },
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(kinds(&report), vec![HazardKind::NativeStash]);
    assert!(report.hazards[0].detail.contains('3'));
    assert_eq!(
        classify_work(&WorkObservation::default(), &GwzEvidence::default()).verdict,
        WorkVerdict::Clean
    );
}

#[test]
fn an_open_gwz_merge_and_stash_record_are_hazards() {
    let evidence = GwzEvidence {
        merge: EvidenceState::Open {
            detail: "merge/state.yml stage=apply".to_owned(),
        },
        stash: EvidenceState::Open {
            detail: "bundle b17".to_owned(),
        },
        other: Vec::new(),
    };
    let report = classify_work(&WorkObservation::default(), &evidence);
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(
        kinds(&report),
        vec![HazardKind::OpenGwzMerge, HazardKind::OpenGwzStash]
    );
    assert!(report.hazards[0].detail.contains("stage=apply"));
    assert!(report.hazards[1].detail.contains("b17"));
    assert!(report.unknown.is_empty());
}

#[test]
fn another_open_gwz_record_is_a_hazard_that_names_its_kind() {
    let evidence = GwzEvidence {
        other: vec![EvidenceItem {
            kind: "exchange".to_owned(),
            state: EvidenceState::Open {
                detail: "transfer t9".to_owned(),
            },
        }],
        ..GwzEvidence::default()
    };
    let report = classify_work(&WorkObservation::default(), &evidence);
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(kinds(&report), vec![HazardKind::OpenGwzRecord]);
    assert!(report.hazards[0].detail.contains("exchange"));
    assert!(report.hazards[0].detail.contains("t9"));
}

// --- unknown evidence ---------------------------------------------------

#[test]
fn uninterpretable_gwz_evidence_is_unknown_and_named_never_clean() {
    let evidence = GwzEvidence {
        merge: EvidenceState::Unknown {
            detail: "record version 9".to_owned(),
        },
        ..GwzEvidence::default()
    };
    let report = classify_work(&WorkObservation::default(), &evidence);
    assert_eq!(report.verdict, WorkVerdict::Unknown);
    assert_ne!(report.verdict, WorkVerdict::Clean);
    assert_eq!(
        unknown_kinds(&report),
        vec![UnknownKind::UnsupportedEvidence]
    );
    assert!(report.unknown[0].detail.contains("record version 9"));
    assert_eq!(kinds(&report), vec![HazardKind::UninterpretableEvidence]);
}

#[test]
fn external_gitfile_and_alternates_layout_evidence_is_unknown() {
    let evidence = GwzEvidence {
        other: vec![
            EvidenceItem {
                kind: "layout/gitfile".to_owned(),
                state: EvidenceState::Unknown {
                    detail: ".git is a file pointing outside the tree".to_owned(),
                },
            },
            EvidenceItem {
                kind: "layout/alternates".to_owned(),
                state: EvidenceState::Unknown {
                    detail: "objects/info/alternates present".to_owned(),
                },
            },
        ],
        ..GwzEvidence::default()
    };
    let report = classify_work(&WorkObservation::default(), &evidence);
    assert_eq!(report.verdict, WorkVerdict::Unknown);
    assert_eq!(
        unknown_kinds(&report),
        vec![
            UnknownKind::UnsupportedEvidence,
            UnknownKind::UnsupportedEvidence
        ]
    );
    let text = details(&report);
    assert!(text.contains("gitfile"), "{text}");
    assert!(text.contains("alternates"), "{text}");
    assert_eq!(
        kinds(&report),
        vec![
            HazardKind::UninterpretableEvidence,
            HazardKind::UninterpretableEvidence
        ]
    );
}

// --- status-suppression flags ------------------------------------------

#[test]
fn evidence_defaults_to_no_record() {
    assert_eq!(GwzEvidence::default().merge, EvidenceState::None);
    let unknown = EvidenceState::Unknown {
        detail: "record version 9".to_owned(),
    };
    assert_ne!(unknown, EvidenceState::None);
}
