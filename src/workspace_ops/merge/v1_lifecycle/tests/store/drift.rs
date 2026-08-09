use super::{insert, seed_open};
use crate::workspace_ops::merge::model::v1::test_record as record;
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundParticipantDrift, ParticipantDriftIdentity, ParticipantDriftPayload,
    VerifiedParticipantDriftClear,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::V1MutationLease;
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;
use crate::workspace_ops::merge::v1_lifecycle::transition::{
    DriftTransition, V1Transition, prepare,
};
use crate::workspace_ops::merge::{ParticipantDrift, ParticipantDriftKind};
use crate::workspace_ops::tests::TempDir;

#[test]
fn clearing_either_duplicate_drift_preserves_the_survivors_own_unknowns() {
    for cleared in 0..2 {
        let root = TempDir::new(&format!("merge-v1-drift-clear-{cleared}"));
        let first = drift("first");
        let second = drift("second");
        let mut model = record();
        model.participants.get_mut("mem_a").unwrap().drift = vec![first, second];
        seed_open(&root, &model, |raw| {
            insert(
                &mut raw["participants"]["mem_a"]["drift"][0],
                "future_drift",
                "belongs-to-first",
            );
            insert(
                &mut raw["participants"]["mem_a"]["drift"][1],
                "future_drift",
                "belongs-to-second",
            );
        });
        let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
        let store = CheckedV1Store::default();
        let current = store.load_open(&root.path, "merge_1").unwrap();
        let removed = current.record().participants["mem_a"].drift[cleared].clone();
        let proof = VerifiedParticipantDriftClear::for_test(
            &current,
            "mem_a",
            "clear_drift",
            "verified",
            ParticipantDriftPayload {
                member_id: "mem_a".into(),
                identity: ParticipantDriftIdentity::new(&removed, cleared),
                drift: removed,
            },
        )
        .unwrap();
        let rewrite = prepare(
            &lease,
            &current,
            V1Transition::Drift(Box::new(DriftTransition::ClearParticipant(Box::new(proof)))),
        )
        .unwrap();
        let next = store.commit(&lease, &current, rewrite).unwrap();

        assert_eq!(next.record().participants["mem_a"].drift.len(), 1);
        assert_eq!(
            next.raw()["participants"]["mem_a"]["drift"][0]["future_drift"],
            if cleared == 0 {
                "belongs-to-second"
            } else {
                "belongs-to-first"
            }
        );
    }
}

#[test]
fn recording_participant_drift_replaces_only_the_exact_semantic_identity() {
    let root = TempDir::new("merge-v1-drift-record-identity");
    let existing = drift("existing");
    let mut distinct = drift("distinct");
    distinct.live_head = Some("f".repeat(40));
    let mut model = record();
    model.participants.get_mut("mem_a").unwrap().drift = vec![existing];
    seed_open(&root, &model, |raw| {
        insert(
            &mut raw["participants"]["mem_a"]["drift"][0],
            "future_drift",
            "stays-with-existing",
        );
    });
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    let fact = BoundParticipantDrift::for_test(
        &current,
        "mem_a",
        "record_drift",
        "observed",
        ParticipantDriftPayload {
            member_id: "mem_a".into(),
            identity: ParticipantDriftIdentity::new(&distinct, 0),
            drift: distinct,
        },
    )
    .unwrap();
    let rewrite = prepare(
        &lease,
        &current,
        V1Transition::Drift(Box::new(DriftTransition::RecordParticipant(Box::new(fact)))),
    )
    .unwrap();
    let next = store.commit(&lease, &current, rewrite).unwrap();

    assert_eq!(next.record().participants["mem_a"].drift.len(), 2);
    assert_eq!(
        next.raw()["participants"]["mem_a"]["drift"][0]["future_drift"],
        "stays-with-existing"
    );
}

fn drift(message: &str) -> ParticipantDrift {
    ParticipantDrift {
        kind: ParticipantDriftKind::HeadAdvanced,
        message: message.into(),
        expected_branch: Some("main".into()),
        live_branch: Some("main".into()),
        expected_head: Some("a".repeat(40)),
        live_head: Some("b".repeat(40)),
        expected_merge_head: None,
        live_merge_head: None,
    }
}
