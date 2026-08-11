#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum TransitionClass {
    Physical,
    CausalParent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum AttemptClass {
    None,
    MatchingSuccess,
    MatchingFailed,
    StaleOrMismatched,
    ConsumedSecond,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum FreshFactClass {
    Before,
    After,
    AfterNeedsDurability,
    Ambiguous,
    OperationalError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum ReconciliationDecision {
    ExecuteOnceThenReobserve,
    Advance,
    RetainOwner,
    Ambiguous,
    OperationalError,
    Reject,
}

/// The complete diagnostic/fresh-fact relation used by physical and causal
/// preservation transitions. Durable state is always decided from the fresh
/// fact; an executor result is diagnostic only.
pub(in crate::workspace_ops::merge::v1_lifecycle) fn reconcile(
    class: TransitionClass,
    attempt: AttemptClass,
    fact: FreshFactClass,
) -> ReconciliationDecision {
    use AttemptClass as A;
    use FreshFactClass as F;
    use ReconciliationDecision as D;

    if matches!(attempt, A::StaleOrMismatched | A::ConsumedSecond) {
        return D::Reject;
    }
    if fact == F::OperationalError {
        return D::OperationalError;
    }
    if fact == F::Ambiguous {
        return D::Ambiguous;
    }

    match (class, attempt, fact) {
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
        (_, A::StaleOrMismatched | A::ConsumedSecond, _) => D::Reject,
        (_, _, F::Ambiguous | F::OperationalError) => unreachable!(),
    }
}
