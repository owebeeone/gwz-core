//! Ordinary worktree dirt: edits, renames, deletions, ignored data and
//! conflict stages.

use super::*;

#[test]
fn an_empty_known_observation_with_no_evidence_is_clean() {
    let report = classify_work(&WorkObservation::default(), &GwzEvidence::default());
    assert_eq!(report.verdict, WorkVerdict::Clean);
    assert!(report.hazards.is_empty());
    assert!(report.unknown.is_empty());
    assert!(!report.truncated);
    assert_eq!(report, WorkReport::clean());
}

// --- on-disk work -------------------------------------------------------

#[test]
fn staged_and_unstaged_versions_of_the_same_file_are_two_hazards() {
    let report = classify_work(
        &observation(vec![
            entry("src/main.rs", WorkKind::Staged),
            entry("src/main.rs", WorkKind::Unstaged),
        ]),
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(
        kinds(&report),
        vec![
            HazardKind::Work(WorkKind::Staged),
            HazardKind::Work(WorkKind::Unstaged),
        ]
    );
    assert!(
        report
            .hazards
            .iter()
            .all(|hazard| hazard.path.as_deref() == Some(b"src/main.rs".as_slice()))
    );
    assert!(report.unknown.is_empty());
    assert!(!report.truncated);
}

#[test]
fn a_binary_edit_is_named_and_unknown_binaryness_claims_nothing() {
    let mut binary = entry("assets/logo.png", WorkKind::Unstaged);
    binary.binary = Some(true);
    let mut text = entry("README.md", WorkKind::Unstaged);
    text.binary = Some(false);
    let undetermined = entry("data.bin", WorkKind::Untracked);
    let report = classify_work(
        &observation(vec![binary, text, undetermined]),
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert!(report.hazards[0].detail.contains("binary"));
    assert!(report.hazards[1].detail.contains("text"));
    assert!(!report.hazards[2].detail.contains("binary"));
    assert!(!report.hazards[2].detail.contains("text"));
}

#[test]
fn renames_deletions_and_mode_and_link_changes_are_each_hazards() {
    let report = classify_work(
        &observation(vec![
            entry("old.rs", WorkKind::Renamed),
            entry("gone.rs", WorkKind::Deleted),
            entry("run.sh", WorkKind::ModeChange),
            entry("link", WorkKind::LinkChange),
        ]),
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(
        kinds(&report),
        vec![
            HazardKind::Work(WorkKind::Renamed),
            HazardKind::Work(WorkKind::Deleted),
            HazardKind::Work(WorkKind::ModeChange),
            HazardKind::Work(WorkKind::LinkChange),
        ]
    );
    assert!(report.unknown.is_empty());
}

#[test]
fn ignored_user_data_is_a_hazard_because_ignored_does_not_mean_disposable() {
    let report = classify_work(
        &observation(vec![entry("data/user.sqlite", WorkKind::Ignored)]),
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(kinds(&report), vec![HazardKind::Work(WorkKind::Ignored)]);
    assert!(report.hazards[0].detail.contains("ignored"));
}

#[test]
fn an_ignored_nested_git_directory_is_a_hazard_with_its_bytes_intact() {
    // Not UTF-8: paths are bytes.
    let nested = WorkEntry {
        path: b"vendor/n\xffsted/.git".to_vec(),
        kind: WorkKind::Ignored,
        binary: None,
    };
    let report = classify_work(&observation(vec![nested]), &GwzEvidence::default());
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(
        report.hazards[0].path.as_deref(),
        Some(b"vendor/n\xffsted/.git".as_slice())
    );
}

#[test]
fn conflict_stages_are_hazards() {
    let report = classify_work(
        &observation(vec![entry("merge.rs", WorkKind::Conflict)]),
        &GwzEvidence::default(),
    );
    assert_eq!(report.verdict, WorkVerdict::Dirty);
    assert_eq!(kinds(&report), vec![HazardKind::Work(WorkKind::Conflict)]);
    assert!(report.hazards[0].detail.contains("conflict"));
}

// --- open operations ----------------------------------------------------
