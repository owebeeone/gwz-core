//! Suppressed index flags: assume-unchanged, skip-worktree and the sparse
//! records that make an absence clean.

use super::*;

#[test]
fn an_edited_assume_unchanged_path_is_dirty() {
    let report = classify_work(
        &suppressed_observation(vec![suppressed(
            "config.toml",
            SuppressionFlag::AssumeUnchanged,
            PhysicalState::Differs,
        )]),
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(kinds(&report), vec![HazardKind::Suppressed]);
    assert_eq!(report.hazards[0].path, Some(path("config.toml")));
    assert!(report.hazards[0].detail.contains("assume-unchanged"));
    assert!(report.unknown.is_empty());
}

#[test]
fn a_present_skip_worktree_path_that_differs_is_dirty() {
    let report = classify_work(
        &suppressed_observation(vec![suppressed(
            "sparse/file.rs",
            SuppressionFlag::SkipWorktree,
            PhysicalState::Differs,
        )]),
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(kinds(&report), vec![HazardKind::Suppressed]);
    assert!(report.hazards[0].detail.contains("skip-worktree"));
}

#[test]
fn a_suppressed_path_without_a_physical_observation_is_unknown_never_clean() {
    for flag in [
        SuppressionFlag::AssumeUnchanged,
        SuppressionFlag::SkipWorktree,
        SuppressionFlag::Other,
    ] {
        let report = classify_work(
            &suppressed_observation(vec![suppressed(
                "opaque",
                flag,
                PhysicalState::Unobservable,
            )]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Unknown, "{flag:?}");
        assert_ne!(report.verdict, WorkVerdict::Clean, "{flag:?}");
        assert_eq!(unknown_kinds(&report), vec![UnknownKind::Unreadable]);
        assert_eq!(report.unknown[0].path, Some(path("opaque")));
    }
}

#[test]
fn a_suppressed_path_physically_matching_the_index_is_clean() {
    for flag in [
        SuppressionFlag::AssumeUnchanged,
        SuppressionFlag::SkipWorktree,
    ] {
        let report = classify_work(
            &suppressed_observation(vec![suppressed(
                "vendored.lock",
                flag,
                PhysicalState::MatchesIndex,
            )]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Clean, "{flag:?}");
        assert!(report.hazards.is_empty());
        assert!(report.unknown.is_empty());
    }
}

#[test]
fn an_assume_unchanged_path_absent_without_a_sparse_record_is_dirty() {
    let report = classify_work(
        &suppressed_observation(vec![suppressed(
            "deleted.txt",
            SuppressionFlag::AssumeUnchanged,
            PhysicalState::Absent,
        )]),
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(kinds(&report), vec![HazardKind::Suppressed]);
    assert!(report.hazards[0].detail.contains("absent"));
}

#[test]
fn skip_worktree_absence_is_clean_only_with_a_recorded_valid_sparse_absence() {
    let unjustified = classify_work(
        &suppressed_observation(vec![suppressed(
            "sparse/absent.rs",
            SuppressionFlag::SkipWorktree,
            PhysicalState::Absent,
        )]),
        &GwzEvidence::default(),
    );
    assert_eq!(unjustified.verdict, WorkVerdict::Unknown);
    assert_eq!(
        unknown_kinds(&unjustified),
        vec![UnknownKind::UnsupportedIndexFlag]
    );
    assert_eq!(unjustified.unknown[0].path, Some(path("sparse/absent.rs")));

    let justified = classify_work(
        &WorkObservation {
            suppressed: vec![suppressed(
                "sparse/absent.rs",
                SuppressionFlag::SkipWorktree,
                PhysicalState::Absent,
            )],
            sparse_absent: vec![path("sparse/absent.rs")],
            ..WorkObservation::default()
        },
        &GwzEvidence::default(),
    );
    assert_eq!(justified.verdict, WorkVerdict::Clean);
    assert!(justified.hazards.is_empty());
    assert!(justified.unknown.is_empty());
}

#[test]
fn an_uninterpreted_index_flag_is_unknown_unless_the_bytes_differ() {
    let differs = classify_work(
        &suppressed_observation(vec![suppressed(
            "flagged",
            SuppressionFlag::Other,
            PhysicalState::Differs,
        )]),
        &GwzEvidence::default(),
    );
    assert_eq!(differs.verdict, WorkVerdict::Dirty);
    assert_eq!(kinds(&differs), vec![HazardKind::Suppressed]);
    assert!(differs.unknown.is_empty());

    for physical in [PhysicalState::MatchesIndex, PhysicalState::Absent] {
        let report = classify_work(
            &WorkObservation {
                suppressed: vec![suppressed("flagged", SuppressionFlag::Other, physical)],
                sparse_absent: vec![path("flagged")],
                ..WorkObservation::default()
            },
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Unknown, "{physical:?}");
        assert_eq!(
            unknown_kinds(&report),
            vec![UnknownKind::UnsupportedIndexFlag]
        );
    }
}

#[test]
fn valid_sparse_absence_on_its_own_is_not_dirt() {
    let report = classify_work(
        &WorkObservation {
            sparse_absent: vec![path("sparse/a.rs"), path("sparse/b.rs")],
            ..WorkObservation::default()
        },
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Clean);
    assert!(report.hazards.is_empty());
    assert!(report.unknown.is_empty());
}

// --- truncation ---------------------------------------------------------
