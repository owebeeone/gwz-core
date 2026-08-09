use super::super::authority::ParticipantDriftIdentity;
use super::super::transition::{EffectKind, RetiredContainer, TransitionEffect};
use crate::workspace_ops::merge::{ParticipantDrift, ParticipantDriftKind};

#[test]
fn retirement_is_variant_and_identity_specific() {
    let outcome =
        TransitionEffect::participant_for_test(EffectKind::RecordParticipantOutcome, "mem_a");
    assert_eq!(
        outcome.retired().unwrap(),
        [
            RetiredContainer::ParticipantPendingAction("mem_a".into()),
            RetiredContainer::ParticipantConflictEvidence("mem_a".into()),
            RetiredContainer::ParticipantError("mem_a".into()),
        ]
    );
    let drift = participant_drift(ParticipantDriftKind::HeadRewound);
    let effect = TransitionEffect::participant_drift_for_test(
        EffectKind::ClearParticipantDrift,
        "mem_a",
        drift.kind,
    );
    assert_eq!(
        effect.retired().unwrap(),
        [RetiredContainer::ParticipantDrift {
            member_id: "mem_a".into(),
            identity: ParticipantDriftIdentity::new(&drift, 0),
        }]
    );
}

fn participant_drift(kind: ParticipantDriftKind) -> ParticipantDrift {
    ParticipantDrift {
        kind,
        message: String::new(),
        expected_branch: None,
        live_branch: None,
        expected_head: None,
        live_head: None,
        expected_merge_head: None,
        live_merge_head: None,
    }
}
