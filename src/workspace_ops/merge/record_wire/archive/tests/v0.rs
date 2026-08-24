use std::collections::BTreeSet;

use super::super::super::super::model::archive_projection::*;
use super::super::{decode_archived, v0};
use super::fixtures::{
    MERGE_ID, Shape, add_unselected_member, bytes, digest, oid, rewrite_candidate_lock, v0_record,
};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::{MergeExecutionMode, OperationState, PublicationStep};

fn project(shape: Shape) -> ArchivedMergeProjection {
    let record = v0_record(shape);
    decode_archived(&bytes(&record), MERGE_ID)
        .unwrap()
        .projection
}

#[test]
fn av0_b_through_g_have_orthogonal_outcomes_and_acceptance() {
    let completed_candidate = project(Shape::CompletedCandidate);
    assert_eq!(
        completed_candidate.terminal_outcome,
        ArchivedTerminalOutcome::Completed
    );
    assert!(matches!(
        completed_candidate.acceptance,
        ArchivedAcceptanceProjection::LegacyComplete {
            source: LegacyAcceptanceSource::Candidate,
            ..
        }
    ));

    let no_publication_record = v0_record(Shape::CompletedNoPublication);
    let exact_baseline = no_publication_record.baseline.lock_yaml.clone().unwrap();
    let no_publication = v0::project(&no_publication_record).unwrap();
    assert!(matches!(
        no_publication.acceptance,
        ArchivedAcceptanceProjection::LegacyComplete {
            workspace: ArchivedAcceptedWorkspace { ref lock_yaml, .. },
            source: LegacyAcceptanceSource::BaselineNoPublication,
            ..
        } if lock_yaml == &exact_baseline
    ));

    let mut evidence_gap = v0_record(Shape::CompletedNoPublication);
    evidence_gap.baseline.lock_yaml = None;
    let projected = v0::project(&evidence_gap).unwrap();
    assert!(matches!(
        projected.acceptance,
        ArchivedAcceptanceProjection::LegacyUnavailable { .. }
    ));

    assert!(matches!(
        project(Shape::AbortedPreAcceptance).acceptance,
        ArchivedAcceptanceProjection::NotAccepted
    ));
    let aborted_complete = project(Shape::AbortedCompleteCandidate);
    assert_eq!(
        aborted_complete.terminal_outcome,
        ArchivedTerminalOutcome::Aborted
    );
    assert!(matches!(
        aborted_complete.acceptance,
        ArchivedAcceptanceProjection::LegacyComplete {
            source: LegacyAcceptanceSource::Candidate,
            ..
        }
    ));
    assert!(matches!(
        project(Shape::AbortedPartialCandidate).acceptance,
        ArchivedAcceptanceProjection::LegacyUnavailable { ref missing, .. }
            if missing == &BTreeSet::from([LegacyAcceptanceGap::PublicationEvidence])
    ));
}

#[test]
fn candidate_projection_adopts_exact_bytes_and_collects_every_gap() {
    let mut record = v0_record(Shape::AbortedPartialCandidate);
    let publication = record.publication.as_mut().unwrap();
    let exact_candidate = publication.candidate.as_ref().unwrap().lock_yaml.clone();
    publication.candidate_lock_sha256 = None;
    record.baseline.manifest_yaml = None;
    record.baseline.root_head = None;
    record.baseline.root_branch = None;

    let projected = v0::project(&record).unwrap();
    let ArchivedAcceptanceProjection::LegacyUnavailable { available, missing } =
        projected.acceptance
    else {
        panic!("expected unavailable legacy acceptance")
    };
    assert_eq!(
        available.lock_yaml.as_deref(),
        Some(exact_candidate.as_str())
    );
    assert_eq!(
        missing,
        BTreeSet::from([
            LegacyAcceptanceGap::ExactLockBytes,
            LegacyAcceptanceGap::CompleteMemberAudit,
            LegacyAcceptanceGap::AcceptedRootInput,
            LegacyAcceptanceGap::PublicationEvidence,
        ])
    );
}

#[test]
fn missing_evidence_is_a_gap_but_present_wrong_evidence_is_unreadable() {
    let mut missing = v0_record(Shape::CompletedNoPublication);
    missing.baseline.lock_yaml = None;
    assert!(matches!(
        v0::project(&missing).unwrap().acceptance,
        ArchivedAcceptanceProjection::LegacyUnavailable { .. }
    ));

    let mut contradictory = v0_record(Shape::CompletedNoPublication);
    contradictory.baseline.lock_yaml = Some("wrong: bytes\n".to_owned());
    let error = decode_archived(&bytes(&contradictory), MERGE_ID).unwrap_err();
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);

    let mut missing_manifest_but_wrong_lock = v0_record(Shape::CompletedCandidate);
    missing_manifest_but_wrong_lock.baseline.manifest_yaml = None;
    rewrite_candidate_lock(&mut missing_manifest_but_wrong_lock, |lock| {
        lock.members.get_mut("mem_a").unwrap().path = "members/wrong".to_owned();
    });
    assert!(v0::project(&missing_manifest_but_wrong_lock).is_err());
}

#[test]
fn unavailable_projection_retains_unselected_rows_and_still_rejects_their_mutation() {
    let mut record = v0_record(Shape::CompletedCandidate);
    add_unselected_member(&mut record);
    record.baseline.manifest_yaml = None;

    let ArchivedAcceptanceProjection::LegacyUnavailable { available, missing } =
        v0::project(&record).unwrap().acceptance
    else {
        panic!("expected unavailable legacy acceptance")
    };
    assert!(missing.contains(&LegacyAcceptanceGap::CompleteMemberAudit));
    let unselected = available
        .members
        .iter()
        .find(|member| member.member_id == "mem_b")
        .unwrap();
    assert!(!unselected.selected);
    assert!(unselected.state.is_none());
    assert!(unselected.integration.is_none());
    assert!(unselected.lock_member.is_some());

    rewrite_candidate_lock(&mut record, |lock| {
        lock.members.get_mut("mem_b").unwrap().commit = Some(oid('f'));
    });
    assert!(v0::project(&record).is_err());
}

#[test]
fn exact_composition_with_missing_optional_marker_path_is_a_publication_gap() {
    let mut record = v0_record(Shape::CompletedCandidate);
    record.publication.as_mut().unwrap().candidate_marker_path = None;
    assert!(matches!(
        v0::project(&record).unwrap().acceptance,
        ArchivedAcceptanceProjection::LegacyUnavailable { ref missing, .. }
            if missing == &BTreeSet::from([LegacyAcceptanceGap::PublicationEvidence])
    ));
}

#[test]
fn no_publication_requires_provably_unchanged_participants() {
    let mut record = v0_record(Shape::CompletedNoPublication);
    let participant = record.participants.get_mut("mem_a").unwrap();
    participant.state = crate::workspace_ops::merge::ParticipantState::Merged;
    participant.resulting_commit = None;
    assert!(v0::project(&record).is_err());
}

#[test]
fn missing_legacy_result_is_a_gap_but_cannot_hide_marker_lock_contradiction() {
    let mut record = v0_record(Shape::CompletedCandidate);
    record
        .participants
        .get_mut("mem_a")
        .unwrap()
        .resulting_commit = None;
    assert!(matches!(
        v0::project(&record).unwrap().acceptance,
        ArchivedAcceptanceProjection::LegacyUnavailable { ref missing, .. }
            if missing.contains(&LegacyAcceptanceGap::CompleteMemberAudit)
    ));

    let publication = record.publication.as_mut().unwrap();
    let candidate = publication.candidate.as_mut().unwrap();
    let mut marker = crate::artifact::MarkerArtifact::from_yaml(&candidate.marker_yaml).unwrap();
    marker
        .merge
        .as_mut()
        .unwrap()
        .participants
        .get_mut("mem_a")
        .unwrap()
        .resulting_commit = oid('e');
    candidate.marker_yaml = marker.to_yaml().unwrap();
    candidate.marker_sha256 = digest(&candidate.marker_yaml);
    publication.candidate_hashes[1].sha256 = candidate.marker_sha256.clone();
    assert!(v0::project(&record).is_err());
}

#[test]
fn root_history_is_archive_derived_and_selected_root_never_borrows_baseline_audit() {
    for (head, branch, expected) in [
        (
            Some(oid('c')),
            Some("main".to_owned()),
            AcceptedRootKind::BornAttached,
        ),
        (Some(oid('c')), None, AcceptedRootKind::BornDetached),
        (
            None,
            Some("main".to_owned()),
            AcceptedRootKind::UnbornAttached,
        ),
    ] {
        let mut record = v0_record(Shape::CompletedNoPublication);
        record.baseline.root_head = head;
        record.baseline.root_branch = branch;
        let ArchivedAcceptanceProjection::LegacyComplete { workspace, .. } =
            v0::project(&record).unwrap().acceptance
        else {
            panic!("expected complete baseline projection")
        };
        assert_eq!(workspace.root.kind, expected);
    }

    let mut selected = v0_record(Shape::CompletedCandidate);
    selected.baseline.lock_commit_sha256 = Some(digest("root-lock"));
    selected.baseline.manifest_commit_sha256 = Some(digest("root-manifest"));
    selected.selected_targets.push("@root".to_owned());
    let mut root = selected.participants["mem_a"].clone();
    root.path = ".".to_owned();
    root.target_kind = crate::workspace_ops::merge::MergeTargetKind::Root;
    root.before_commit = oid('c');
    root.source_commit = oid('b');
    root.resulting_commit = Some(oid('d'));
    selected.participants.insert("@root".to_owned(), root);
    let publication = selected.publication.as_mut().unwrap();
    publication.root_merge_commit = Some(oid('d'));
    let marker = publication.candidate.as_mut().unwrap();
    let mut raw: crate::artifact::MarkerArtifact =
        crate::artifact::MarkerArtifact::from_yaml(&marker.marker_yaml).unwrap();
    raw.selected_targets.push("@root".to_owned());
    raw.root.before_commit = Some(oid('d'));
    let merge = raw.merge.as_mut().unwrap();
    merge.selected_targets.push("@root".to_owned());
    merge.participants.insert(
        "@root".to_owned(),
        crate::artifact::MarkerMergeParticipantArtifact {
            target_kind: crate::artifact::MarkerMergeTargetKind::Root,
            target_branch: "main".to_owned(),
            before_commit: oid('c'),
            source_commit: oid('b'),
            resulting_commit: oid('d'),
        },
    );
    merge.root_merge_commit = Some(oid('d'));
    marker.marker_yaml = raw.to_yaml().unwrap();
    marker.marker_sha256 = digest(&marker.marker_yaml);
    publication.candidate_hashes[1].sha256 = marker.marker_sha256.clone();

    assert!(matches!(
        v0::project(&selected).unwrap().acceptance,
        ArchivedAcceptanceProjection::LegacyUnavailable { ref missing, .. }
            if missing.contains(&LegacyAcceptanceGap::CompleteMemberAudit)
    ));

    let mut missing_root_result = selected.clone();
    missing_root_result
        .participants
        .get_mut("@root")
        .unwrap()
        .resulting_commit = None;
    assert!(matches!(
        v0::project(&missing_root_result).unwrap().acceptance,
        ArchivedAcceptanceProjection::LegacyUnavailable { ref missing, .. }
            if missing == &BTreeSet::from([
                LegacyAcceptanceGap::CompleteMemberAudit,
                LegacyAcceptanceGap::AcceptedRootInput,
            ])
    ));

    let mut wrong_selected_lock = selected.clone();
    rewrite_candidate_lock(&mut wrong_selected_lock, |lock| {
        lock.members.get_mut("mem_a").unwrap().dirty = Some(true);
    });
    assert!(v0::project(&wrong_selected_lock).is_err());

    let mut wrong_metadata_base = selected.clone();
    let candidate = wrong_metadata_base
        .publication
        .as_mut()
        .unwrap()
        .candidate
        .as_mut()
        .unwrap();
    let mut base = crate::artifact::LockArtifact::from_yaml(&candidate.baseline_lock_yaml).unwrap();
    base.members.get_mut("mem_a").unwrap().path = "members/wrong".to_owned();
    candidate.baseline_lock_yaml = base.to_yaml().unwrap();
    assert!(v0::project(&wrong_metadata_base).is_err());

    let mut wrong_root_base = selected;
    wrong_root_base.baseline.root_head = Some(oid('f'));
    assert!(v0::project(&wrong_root_base).is_err());
}

#[test]
fn terminal_no_ff_is_historical_and_nonterminal_or_contradictory_rows_fail() {
    for shape in [Shape::CompletedCandidate, Shape::AbortedCompleteCandidate] {
        let mut record = v0_record(shape);
        record.mode = MergeExecutionMode::NoFf;
        assert!(v0::project(&record).is_ok());
    }

    for state in [
        OperationState::Executing,
        OperationState::AwaitingResolution,
        OperationState::Halted,
        OperationState::Finalizing,
        OperationState::Preserving,
        OperationState::RollingBack,
        OperationState::RecoveryRequired,
    ] {
        let mut nonterminal = v0_record(Shape::CompletedCandidate);
        nonterminal.state = state;
        assert!(v0::project(&nonterminal).is_err(), "{state:?}");
    }

    let mut contradictory = v0_record(Shape::AbortedCompleteCandidate);
    contradictory
        .publication
        .as_mut()
        .unwrap()
        .evidence_rolled_back = false;
    assert!(v0::project(&contradictory).is_err());

    let mut partial_triad = v0_record(Shape::CompletedCandidate);
    let publication = partial_triad.publication.as_mut().unwrap();
    publication.composition_tree = None;
    assert!(v0::project(&partial_triad).is_err());

    let mut impossible_phase = v0_record(Shape::AbortedPartialCandidate);
    impossible_phase.publication.as_mut().unwrap().step = PublicationStep::Complete;
    assert!(v0::project(&impossible_phase).is_err());
}

#[test]
fn unknown_fields_survive_because_projection_never_rewrites_archive_bytes() {
    let mut raw = serde_yaml::to_value(v0_record(Shape::CompletedCandidate)).unwrap();
    raw["future_record"] = serde_yaml::Value::String("retained".to_owned());
    raw["baseline"]["future_baseline"] = serde_yaml::Value::Bool(true);
    raw["participants"]["mem_a"]["future_participant"] =
        serde_yaml::Value::String("retained".to_owned());
    raw["publication"]["future_publication"] = serde_yaml::Value::Bool(true);
    raw["publication"]["candidate"]["future_candidate"] =
        serde_yaml::Value::String("retained".to_owned());
    let bytes = serde_yaml::to_string(&raw).unwrap().into_bytes();
    let before = bytes.clone();
    decode_archived(&bytes, MERGE_ID).unwrap();
    assert_eq!(bytes, before);
}
