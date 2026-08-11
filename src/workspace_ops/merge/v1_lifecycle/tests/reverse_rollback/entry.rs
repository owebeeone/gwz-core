use super::*;
use crate::workspace_ops::merge::abort::{
    preflight_v1_rollback, verify_v1_no_mutation_participant,
};

#[test]
fn global_preflight_failure_mutates_no_earlier_participant() {
    let mut fixture = integrated_fixture("v1-rollback-entry-global");
    fixture.model.state = OperationState::Halted;
    let member_b = fixture.root.path.join("members/b");
    fixture.backend.create_repo(&member_b).unwrap();
    let before_b = commit_file(&member_b, "README.md", "before b\n", "before", &[]).unwrap();
    let result_b = commit_file(
        &member_b,
        "README.md",
        "result b\n",
        "result",
        &[before_b.parse().unwrap()],
    )
    .unwrap();
    let mut row_b = fixture.model.participants["mem_a"].clone();
    row_b.path = "members/b".into();
    row_b.before_commit = before_b;
    row_b.source_commit = result_b.clone();
    row_b.resulting_commit = Some(result_b);
    fixture.model.selected_targets.push("mem_b".into());
    fixture.model.participants.insert("mem_b".into(), row_b);
    std::fs::write(member_b.join("untracked"), "drift\n").unwrap();

    let a_before = fixture.backend.head(&fixture.member).unwrap();
    let error =
        preflight_v1_rollback(&fixture.backend, &fixture.root.path, &fixture.model).unwrap_err();
    assert_eq!(error.member_id.as_deref(), Some("mem_b"));
    assert_eq!(fixture.backend.head(&fixture.member).unwrap(), a_before);
    assert_eq!(
        fixture.backend.head(&member_b).unwrap().commit.as_deref(),
        fixture.model.participants["mem_b"]
            .resulting_commit
            .as_deref()
    );
}

#[test]
fn no_mutation_participants_require_the_exact_clean_before_checkout() {
    for state in [
        ParticipantState::Planned,
        ParticipantState::UpToDate,
        ParticipantState::Failed,
        ParticipantState::Unattempted,
    ] {
        let mut fixture = integrated_fixture(&format!("v1-rollback-no-mutation-{state:?}"));
        fixture
            .backend
            .set_branch_target_checked(&fixture.member, "main", &fixture.result, &fixture.before)
            .unwrap();
        fixture.model.participants.get_mut("mem_a").unwrap().state = state;
        let row = &fixture.model.participants["mem_a"];
        verify_v1_no_mutation_participant(&fixture.backend, &fixture.root.path, "mem_a", row)
            .unwrap();
        std::fs::write(fixture.member.join("untracked"), "drift\n").unwrap();
        assert!(
            verify_v1_no_mutation_participant(&fixture.backend, &fixture.root.path, "mem_a", row,)
                .is_err(),
            "{state:?}",
        );
    }
}

#[test]
fn selected_root_publication_handoff_rejects_all_unrelated_dirt_before_entry() {
    for kind in ["staged", "unstaged", "untracked"] {
        let fixture =
            selected_root_evidence_fixture(&format!("v1-rollback-root-publication-dirt-{kind}"));
        match kind {
            "staged" => {
                std::fs::write(fixture.root.path.join("unrelated.txt"), "staged\n").unwrap();
                fixture
                    .backend
                    .stage_paths(&fixture.root.path, &["unrelated.txt"])
                    .unwrap();
            }
            "unstaged" => {
                std::fs::write(
                    fixture.root.path.join(crate::workspace::WORKSPACE_MANIFEST),
                    "unrelated edit\n",
                )
                .unwrap();
            }
            "untracked" => {
                std::fs::write(fixture.root.path.join("unrelated.txt"), "untracked\n").unwrap();
            }
            _ => unreachable!(),
        }
        let head_before = fixture.backend.head(&fixture.root.path).unwrap();
        let lock_before =
            std::fs::read(fixture.root.path.join(crate::artifact::LOCK_PATH)).unwrap();
        let marker = fixture
            .model
            .publication
            .as_ref()
            .unwrap()
            .candidate_marker_path
            .as_ref()
            .unwrap();
        let marker_before = std::fs::read(fixture.root.path.join(marker)).unwrap();
        let error = preflight_v1_rollback(&fixture.backend, &fixture.root.path, &fixture.model)
            .unwrap_err();
        assert_eq!(error.member_id.as_deref(), Some("@root"), "{kind}");
        assert_eq!(
            fixture.backend.head(&fixture.root.path).unwrap(),
            head_before,
            "{kind}"
        );
        assert_eq!(
            std::fs::read(fixture.root.path.join(crate::artifact::LOCK_PATH)).unwrap(),
            lock_before,
            "{kind}"
        );
        assert_eq!(
            std::fs::read(fixture.root.path.join(marker)).unwrap(),
            marker_before,
            "{kind}"
        );
    }
}

fn selected_root_evidence_fixture(name: &str) -> EvidenceFixture {
    use crate::workspace_ops::merge::model::v1::AcceptedRootBaseV1;

    let mut fixture = staged_evidence_fixture(name, true, true);
    let result = match &fixture.model.accepted_workspace.as_ref().unwrap().root.base {
        AcceptedRootBaseV1::BornAttached { commit, .. } => commit.clone(),
        _ => panic!("fixture must have an attached root base"),
    };
    let mut row = fixture.model.participants.remove("mem_a").unwrap();
    row.path = ".".into();
    row.target_kind = MergeTargetKind::Root;
    row.target_branch = "main".into();
    row.state = ParticipantState::Merged;
    row.resulting_commit = Some(result.clone());
    row.source_commit = result;
    fixture.model.selected_targets = vec!["@root".into()];
    fixture.model.participants.clear();
    fixture.model.participants.insert("@root".into(), row);
    preflight_v1_rollback(&fixture.backend, &fixture.root.path, &fixture.model).unwrap();
    fixture
}
