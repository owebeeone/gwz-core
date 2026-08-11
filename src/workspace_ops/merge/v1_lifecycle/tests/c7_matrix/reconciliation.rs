use super::matrix_spec::*;
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    FreshFactClass, ReconciliationDecision as D, TransitionClass, reconcile,
};

#[test]
fn every_ordinary_row_pins_exactly_twenty_attempt_fact_cells() {
    let mut physical_rows = 0;
    let mut causal_rows = 0;

    for owner in OWNERS {
        for &handoff in legal_handoffs(owner) {
            for request in REQUESTS {
                for row in root_rows(handoff) {
                    let _identity = (owner, handoff, request, row.phase);
                    match row.class.transition() {
                        Some(TransitionClass::Physical) => {
                            assert_relation(TransitionClass::Physical, &PHYSICAL_FACTS);
                            physical_rows += 1;
                        }
                        Some(TransitionClass::CausalParent) => {
                            assert_relation(TransitionClass::CausalParent, &CAUSAL_FACTS);
                            causal_rows += 1;
                        }
                        None => {}
                    }
                }
            }
        }
        for request in REQUESTS {
            for phase in CAUSAL_RESTORE_PARENT_VARIANTS {
                let _identity = (owner, request, phase);
                assert_relation(TransitionClass::CausalParent, &CAUSAL_FACTS);
                causal_rows += 1;
            }
        }
    }
    for request in REQUESTS {
        for (_, class) in ROLLBACK_ROWS
            .into_iter()
            .chain(NON_ROOT_ROWS.map(|(_, class)| (RollbackRow::RecordNoMutationAbort, class)))
            .chain(
                CHECKED_ARTIFACT_ROWS.map(|(_, class)| (RollbackRow::RecordNoMutationAbort, class)),
            )
        {
            let _request = request;
            if let Some(class) = class.transition() {
                assert_relation(class, facts(class));
                match class {
                    TransitionClass::Physical => physical_rows += 1,
                    TransitionClass::CausalParent => causal_rows += 1,
                }
            }
        }
    }

    assert_eq!(physical_rows, 635);
    assert_eq!(causal_rows, 40);
}

#[test]
fn physical_and_causal_relations_preserve_the_frozen_diagnostic_rules() {
    use crate::workspace_ops::merge::v1_lifecycle::authority::AttemptClass as A;
    use FreshFactClass as F;

    assert_eq!(
        reconcile(TransitionClass::Physical, A::None, F::Before),
        D::ExecuteOnceThenReobserve
    );
    assert_eq!(
        reconcile(TransitionClass::Physical, A::MatchingFailed, F::After),
        D::Advance
    );
    assert_eq!(
        reconcile(TransitionClass::Physical, A::MatchingSuccess, F::Before),
        D::RetainOwner
    );
    assert_eq!(
        reconcile(
            TransitionClass::CausalParent,
            A::None,
            F::AfterNeedsDurability
        ),
        D::ExecuteOnceThenReobserve
    );
    assert_eq!(
        reconcile(
            TransitionClass::CausalParent,
            A::MatchingSuccess,
            F::AfterNeedsDurability
        ),
        D::Advance
    );
    assert_eq!(
        reconcile(
            TransitionClass::CausalParent,
            A::MatchingFailed,
            F::AfterNeedsDurability
        ),
        D::RetainOwner
    );
}

fn assert_relation(class: TransitionClass, facts: &[FreshFactClass; 4]) {
    let cells = ATTEMPTS
        .into_iter()
        .flat_map(|attempt| facts.iter().copied().map(move |fact| (attempt, fact)))
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), 20);
    for (attempt, fact) in cells {
        let result = reconcile(class, attempt, fact);
        assert_eq!(
            result,
            expected_decision(class, attempt, fact),
            "wrong decision: {class:?}/{attempt:?}/{fact:?} -> {result:?}"
        );
    }
}

fn expected_decision(
    class: TransitionClass,
    attempt: crate::workspace_ops::merge::v1_lifecycle::authority::AttemptClass,
    fact: FreshFactClass,
) -> D {
    use crate::workspace_ops::merge::v1_lifecycle::authority::AttemptClass as A;
    use FreshFactClass as F;

    match (class, attempt, fact) {
        (_, A::StaleOrMismatched | A::ConsumedSecond, _) => D::Reject,
        (_, _, F::OperationalError) => D::OperationalError,
        (_, _, F::Ambiguous) => D::Ambiguous,
        (TransitionClass::Physical, A::None, F::Before) => D::ExecuteOnceThenReobserve,
        (TransitionClass::Physical, A::None, F::After) => D::Advance,
        (TransitionClass::Physical, A::MatchingSuccess | A::MatchingFailed, F::Before) => {
            D::RetainOwner
        }
        (TransitionClass::Physical, A::MatchingSuccess | A::MatchingFailed, F::After) => D::Advance,
        (TransitionClass::CausalParent, A::None, F::Before | F::AfterNeedsDurability) => {
            D::ExecuteOnceThenReobserve
        }
        (TransitionClass::CausalParent, A::MatchingSuccess | A::MatchingFailed, F::Before) => {
            D::RetainOwner
        }
        (TransitionClass::CausalParent, A::MatchingSuccess, F::AfterNeedsDurability) => D::Advance,
        (TransitionClass::CausalParent, A::MatchingFailed, F::AfterNeedsDurability) => {
            D::RetainOwner
        }
        (TransitionClass::Physical, _, F::AfterNeedsDurability)
        | (TransitionClass::CausalParent, _, F::After) => D::Reject,
    }
}

fn facts(class: TransitionClass) -> &'static [FreshFactClass; 4] {
    match class {
        TransitionClass::Physical => &PHYSICAL_FACTS,
        TransitionClass::CausalParent => &CAUSAL_FACTS,
    }
}
