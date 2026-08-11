use super::matrix_spec::*;
use crate::workspace_ops::merge::model::v1::{
    PreservationRefResetPhaseV1 as R, PreservationStashPhaseV1 as S,
};

#[test]
fn root_domain_is_explicit_and_cardinality_closed() {
    assert_eq!(REQUESTS.len(), 5);
    assert_eq!(OWNERS.len(), 2);
    assert_eq!(HANDOFFS.len(), 8);
    assert_eq!(
        HANDOFFS.iter().filter(|form| form.has_candidate()).count(),
        6
    );
    assert_eq!(legal_handoffs(RootOwner::PublicationRoot).len(), 6);
    assert_eq!(legal_handoffs(RootOwner::SelectedRoot).len(), 7);
    assert!(
        legal_handoffs(RootOwner::PublicationRoot)
            .iter()
            .all(|handoff| handoff.has_candidate())
    );
    assert!(
        OWNERS
            .iter()
            .all(|owner| !legal_handoffs(*owner).contains(&HandoffShape::EvidencePending))
    );

    for handoff in HANDOFFS {
        let rows = root_rows(handoff);
        assert_eq!(
            rows.len(),
            match handoff {
                HandoffShape::NoCandidate => 6,
                HandoffShape::EvidencePending => 3,
                _ => 22,
            }
        );
        assert_eq!(
            rows.iter()
                .enumerate()
                .filter(|(index, row)| rows[..*index].iter().all(|seen| seen.phase != row.phase))
                .count(),
            rows.len(),
            "duplicate durable phase for {handoff:?}"
        );
    }

    let candidate = root_rows(HandoffShape::BoundaryStaged);
    assert!(candidate.contains(&RootRow {
        phase: RootPhase::Stash(S::RestoreParent),
        class: RowClass::ProofOnly,
    }));

    assert_eq!(
        [
            HandoffShape::BaselinePre,
            HandoffShape::MarkerPre,
            HandoffShape::LockPre,
            HandoffShape::BoundaryPre,
            HandoffShape::MarkerStagedDegenerate,
            HandoffShape::BoundaryStaged,
        ]
        .map(|handoff| canonical_physical_root_phases(handoff).len()),
        [18, 12, 8, 8, 4, 4]
    );
    assert!(candidate.contains(&RootRow {
        phase: RootPhase::Reset(R::RestoreParent),
        class: RowClass::ProofOnly,
    }));
    assert!(candidate.contains(&RootRow {
        phase: RootPhase::Stash(S::Complete),
        class: RowClass::ActionFree,
    }));
    assert_eq!(
        CAUSAL_RESTORE_PARENT_VARIANTS,
        [
            RootPhase::Stash(S::RestoreParent),
            RootPhase::Reset(R::RestoreParent),
        ]
    );
    assert!(candidate.contains(&RootRow {
        phase: RootPhase::Reset(R::Complete),
        class: RowClass::ActionFree,
    }));

    let generated = OWNERS
        .into_iter()
        .flat_map(|owner| {
            legal_handoffs(owner)
                .iter()
                .copied()
                .flat_map(move |handoff| {
                    REQUESTS.into_iter().flat_map(move |request| {
                        root_rows(handoff)
                            .into_iter()
                            .map(move |row| (owner, handoff, request, row))
                    })
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 1_350);
}

#[test]
fn absent_handoffs_use_only_their_legal_short_graphs() {
    assert_eq!(
        root_rows(HandoffShape::NoCandidate),
        vec![
            RootRow {
                phase: RootPhase::BackupRef,
                class: RowClass::Physical,
            },
            RootRow {
                phase: RootPhase::Stash(S::CreateStash),
                class: RowClass::Physical,
            },
            RootRow {
                phase: RootPhase::Stash(S::WriteBundle),
                class: RowClass::Physical,
            },
            RootRow {
                phase: RootPhase::Stash(S::Complete),
                class: RowClass::ActionFree,
            },
            RootRow {
                phase: RootPhase::Reset(R::ResetRef),
                class: RowClass::Physical,
            },
            RootRow {
                phase: RootPhase::Reset(R::Complete),
                class: RowClass::ActionFree,
            },
        ]
    );
    assert_eq!(
        root_rows(HandoffShape::EvidencePending),
        vec![
            RootRow {
                phase: RootPhase::Stash(S::CreateStash),
                class: RowClass::Physical,
            },
            RootRow {
                phase: RootPhase::Stash(S::WriteBundle),
                class: RowClass::Physical,
            },
            RootRow {
                phase: RootPhase::Stash(S::Complete),
                class: RowClass::ActionFree,
            },
        ]
    );
}
