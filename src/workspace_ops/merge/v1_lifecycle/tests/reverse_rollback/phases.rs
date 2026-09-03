use super::*;
use crate::workspace_ops::merge::v1_rollback::{
    V1EvidenceRollbackObservation as E, V1ParticipantRollbackObservation as O,
    execute_v1_evidence_rollback, observe_v1_evidence_rollback, observe_v1_participant_rollback,
};
use crate::workspace_ops::merge::model::v1::{EvidenceRollbackStepV1, ParticipantRollbackKindV1};

#[test]
fn integrated_participant_classifies_exact_before_after_and_ambiguous() {
    let fixture = integrated_fixture("v1-rollback-participant-phases");
    let row = &fixture.model.participants["mem_a"];
    assert_eq!(
        observe_v1_participant_rollback(
            &fixture.backend,
            &fixture.root.path,
            &fixture.model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::ResetIntegrated,
        )
        .unwrap(),
        O::Before
    );
    fixture
        .backend
        .set_branch_target_checked(&fixture.member, "main", &fixture.result, &fixture.before)
        .unwrap();
    assert_eq!(
        observe_v1_participant_rollback(
            &fixture.backend,
            &fixture.root.path,
            &fixture.model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::ResetIntegrated,
        )
        .unwrap(),
        O::After
    );
    std::fs::write(fixture.member.join("untracked"), "drift\n").unwrap();
    assert_eq!(
        observe_v1_participant_rollback(
            &fixture.backend,
            &fixture.root.path,
            &fixture.model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::ResetIntegrated,
        )
        .unwrap(),
        O::Ambiguous
    );
}

#[test]
fn integrated_rollback_remains_exact_under_repo_local_autocrlf() {
    let fixture = integrated_fixture("v1-rollback-participant-autocrlf");
    // Windows runners resolve system-level `core.autocrlf=true`; libgit2 honors
    // the same key at repository scope, so pinning it repo-locally here (after
    // the fixture commits, which are LF blob + LF worktree with no checkout in
    // between) reproduces the Windows platform semantics on any host.
    git2::Repository::open(&fixture.member)
        .unwrap()
        .config()
        .unwrap()
        .set_bool("core.autocrlf", true)
        .unwrap();
    let row = &fixture.model.participants["mem_a"];
    // The config flip alone rewrites nothing: the live worktree is still the
    // fixture's own LF bytes, so the exact before state must still hold.
    assert_eq!(
        observe_v1_participant_rollback(
            &fixture.backend,
            &fixture.root.path,
            &fixture.model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::ResetIntegrated,
        )
        .unwrap(),
        O::Before
    );
    fixture
        .backend
        .set_branch_target_checked(&fixture.member, "main", &fixture.result, &fixture.before)
        .unwrap();
    // Recovery-grade rollback must land blob-exact bytes even under an active
    // CRLF filter, so recovery-time re-verification classifies the exact
    // after state instead of Ambiguous.
    assert_eq!(
        observe_v1_participant_rollback(
            &fixture.backend,
            &fixture.root.path,
            &fixture.model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::ResetIntegrated,
        )
        .unwrap(),
        O::After
    );
    assert_eq!(
        std::fs::read(fixture.member.join("README.md")).unwrap(),
        b"before\n"
    );
}

#[test]
fn evidence_rollback_steps_accept_only_their_exact_before_and_after_states() {
    let fixture = staged_evidence_fixture("v1-rollback-evidence-phases", true, true);
    let steps = [
        EvidenceRollbackStepV1::EvidenceCommit,
        EvidenceRollbackStepV1::Boundary,
        EvidenceRollbackStepV1::Lock,
        EvidenceRollbackStepV1::Marker,
        EvidenceRollbackStepV1::Index,
    ];
    for step in steps {
        assert_eq!(
            observe_v1_evidence_rollback(
                &fixture.backend,
                &fixture.root.path,
                &fixture.model,
                step,
            )
            .unwrap(),
            E::Before,
            "{step:?} before",
        );
        execute_v1_evidence_rollback(&fixture.backend, &fixture.root.path, &fixture.model, step)
            .unwrap();
        assert_eq!(
            observe_v1_evidence_rollback(
                &fixture.backend,
                &fixture.root.path,
                &fixture.model,
                step,
            )
            .unwrap(),
            E::After,
            "{step:?} after",
        );
    }
    assert_eq!(
        observe_v1_evidence_rollback(
            &fixture.backend,
            &fixture.root.path,
            &fixture.model,
            EvidenceRollbackStepV1::Complete,
        )
        .unwrap(),
        E::After,
    );
}

#[test]
fn every_evidence_phase_rejects_a_third_state() {
    use crate::artifact::LOCK_PATH;
    const PRIOR: &[EvidenceRollbackStepV1] = &[
        EvidenceRollbackStepV1::EvidenceCommit,
        EvidenceRollbackStepV1::Boundary,
        EvidenceRollbackStepV1::Lock,
        EvidenceRollbackStepV1::Marker,
        EvidenceRollbackStepV1::Index,
    ];
    let advance = |fixture: &EvidenceFixture, count| {
        for step in &PRIOR[..count] {
            execute_v1_evidence_rollback(
                &fixture.backend,
                &fixture.root.path,
                &fixture.model,
                *step,
            )
            .unwrap();
        }
    };
    let assert_ambiguous = |fixture: &EvidenceFixture, step| {
        assert_eq!(
            observe_v1_evidence_rollback(
                &fixture.backend,
                &fixture.root.path,
                &fixture.model,
                step,
            )
            .unwrap(),
            E::Ambiguous,
            "{step:?} third state",
        );
    };
    let marker_path = |fixture: &EvidenceFixture| {
        fixture
            .model
            .publication
            .as_ref()
            .unwrap()
            .candidate_marker_path
            .as_deref()
            .unwrap()
            .to_owned()
    };

    let fixture = staged_evidence_fixture("v1-rollback-evidence-third-head", true, true);
    std::fs::write(fixture.root.path.join(marker_path(&fixture)), "foreign\n").unwrap();
    assert_ambiguous(&fixture, EvidenceRollbackStepV1::EvidenceCommit);

    let fixture = staged_evidence_fixture("v1-rollback-evidence-third-boundary", true, true);
    advance(&fixture, 1);
    std::fs::remove_file(fixture.root.path.join(marker_path(&fixture))).unwrap();
    assert_ambiguous(&fixture, EvidenceRollbackStepV1::Boundary);

    let fixture = staged_evidence_fixture("v1-rollback-evidence-third-lock", true, true);
    advance(&fixture, 2);
    let candidate = fixture
        .model
        .publication
        .as_ref()
        .unwrap()
        .candidate
        .as_ref()
        .unwrap();
    crate::workspace_ops::publish_workspace_exclude_candidate(
        &fixture.root.path,
        &candidate.boundary_text,
    )
    .unwrap();
    assert_ambiguous(&fixture, EvidenceRollbackStepV1::Lock);

    let fixture = staged_evidence_fixture("v1-rollback-evidence-third-marker", true, true);
    advance(&fixture, 3);
    let candidate = fixture
        .model
        .publication
        .as_ref()
        .unwrap()
        .candidate
        .as_ref()
        .unwrap();
    std::fs::write(fixture.root.path.join(LOCK_PATH), &candidate.lock_yaml).unwrap();
    assert_ambiguous(&fixture, EvidenceRollbackStepV1::Marker);

    let fixture = staged_evidence_fixture("v1-rollback-evidence-third-index", true, true);
    advance(&fixture, 4);
    let marker = &fixture
        .model
        .publication
        .as_ref()
        .unwrap()
        .candidate
        .as_ref()
        .unwrap()
        .marker_yaml;
    std::fs::write(fixture.root.path.join(marker_path(&fixture)), marker).unwrap();
    assert_ambiguous(&fixture, EvidenceRollbackStepV1::Index);

    let fixture = staged_evidence_fixture("v1-rollback-evidence-third-complete", true, true);
    advance(&fixture, 5);
    std::fs::write(fixture.root.path.join(LOCK_PATH), "foreign\n").unwrap();
    assert_ambiguous(&fixture, EvidenceRollbackStepV1::Complete);
}

#[test]
fn evidence_rollback_skips_artifact_steps_that_are_already_baseline() {
    let fixture = staged_evidence_fixture("v1-rollback-evidence-noops", false, false);
    execute_v1_evidence_rollback(
        &fixture.backend,
        &fixture.root.path,
        &fixture.model,
        EvidenceRollbackStepV1::EvidenceCommit,
    )
    .unwrap();
    for step in [
        EvidenceRollbackStepV1::Boundary,
        EvidenceRollbackStepV1::Lock,
    ] {
        assert_eq!(
            observe_v1_evidence_rollback(
                &fixture.backend,
                &fixture.root.path,
                &fixture.model,
                step,
            )
            .unwrap(),
            E::After,
            "{step:?} no-op",
        );
    }
}
