use super::super::acceptance::{
    AcceptedRootBase, accepted_root_checkout, accepted_root_checkout_with_observation,
    construct_complete_lock, publication_required,
};
use super::super::{MergeTargetKind, ParticipantState};
use super::fixtures::{manifest_and_lock, participant, record};

#[test]
fn complete_lock_updates_only_selected_member_result_fields() {
    let record = record();
    let (manifest, lock) = manifest_and_lock();
    let baseline = lock.members["mem_one"].clone();

    let complete = construct_complete_lock(&record, &manifest, lock).unwrap();
    let selected = &complete.members["mem_one"];

    assert_eq!(selected.path, baseline.path);
    assert_eq!(selected.source_id, baseline.source_id);
    assert_eq!(selected.source_kind, baseline.source_kind);
    assert_eq!(selected.upstream, baseline.upstream);
    assert_eq!(selected.commit.as_deref(), Some("after"));
    assert_eq!(selected.branch.as_deref(), Some("main"));
    assert_eq!(selected.detached, Some(false));
    assert_eq!(selected.dirty, Some(false));
    assert_eq!(selected.materialized, Some(true));
}

#[test]
fn complete_lock_rejects_missing_active_selected_member_with_context() {
    let record = record();
    let (mut manifest, lock) = manifest_and_lock();
    manifest.members[0].active = false;

    let error = construct_complete_lock(&record, &manifest, lock).unwrap_err();

    assert_eq!(error.error.code, crate::model::ErrorCode::ManifestInvalid);
    assert_eq!(error.error.member_id.as_deref(), Some("mem_one"));
    assert_eq!(error.error.member_path.as_deref(), Some("member"));
}

#[test]
fn accepted_root_checkout_covers_selected_and_frozen_baseline_forms() {
    let attached = accepted_root_checkout(&record()).unwrap();
    assert_eq!(
        attached.base,
        AcceptedRootBase::BornAttached {
            commit: "root-before".to_owned(),
            symbolic_branch: "main".to_owned(),
        }
    );
    assert!(!attached.root_selected);

    let mut detached_record = record();
    detached_record.baseline.root_branch = None;
    assert_eq!(
        accepted_root_checkout(&detached_record).unwrap().base,
        AcceptedRootBase::BornDetached {
            commit: "root-before".to_owned(),
        }
    );

    let mut unborn_record = record();
    unborn_record.baseline.root_head = None;
    assert_eq!(
        accepted_root_checkout(&unborn_record).unwrap().base,
        AcceptedRootBase::UnbornAttached {
            symbolic_branch: "main".to_owned(),
        }
    );

    let mut selected_record = record();
    let mut root = participant(ParticipantState::Merged, "root-before", Some("root-after"));
    root.path = ".".to_owned();
    root.target_kind = MergeTargetKind::Root;
    selected_record.selected_targets.push("@root".to_owned());
    selected_record
        .participants
        .insert("@root".to_owned(), root);
    let selected = accepted_root_checkout(&selected_record).unwrap();
    assert!(selected.root_selected);
    assert_eq!(selected.evidence_parent(), Some("root-after"));
    assert_eq!(selected.publication_branch(), Some("main"));
}

#[test]
fn accepted_root_checkout_preserves_the_v0_live_branch_fallbacks() {
    let mut legacy = record();
    legacy.baseline.root_head = None;
    legacy.baseline.root_branch = None;
    let observation = crate::git::GitHeadState {
        branch: Some("main".to_owned()),
        commit: None,
        is_detached: false,
    };
    assert_eq!(
        accepted_root_checkout_with_observation(&legacy, Some(&observation))
            .unwrap()
            .base,
        AcceptedRootBase::UnbornAttached {
            symbolic_branch: "main".to_owned(),
        }
    );
    assert!(accepted_root_checkout(&legacy).is_err());

    legacy.baseline.root_head = Some("root-before".to_owned());
    let attached = crate::git::GitHeadState {
        branch: Some("recovered".to_owned()),
        commit: Some("root-before".to_owned()),
        is_detached: false,
    };
    assert_eq!(
        accepted_root_checkout_with_observation(&legacy, Some(&attached))
            .unwrap()
            .base,
        AcceptedRootBase::BornAttached {
            commit: "root-before".to_owned(),
            symbolic_branch: "recovered".to_owned(),
        }
    );
}

#[test]
fn publication_required_uses_the_shared_result_semantics() {
    let mut changed = record();
    assert!(publication_required(&changed));
    changed.participants.get_mut("mem_one").unwrap().state = ParticipantState::UpToDate;
    changed
        .participants
        .get_mut("mem_one")
        .unwrap()
        .resulting_commit = Some("before".to_owned());
    assert!(!publication_required(&changed));
}
