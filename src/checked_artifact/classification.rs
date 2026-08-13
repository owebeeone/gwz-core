use super::authority::{ArtifactOperation, RetainedSource};
use super::observation::observe_leaf_exact;
use super::{CheckedArtifact, CheckedArtifactFact, CheckedArtifactTransition, ParentState, error};
use crate::model::ModelResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExactTransition {
    ProofOnly,
    Before,
    BeforeBound,
    RecoverableStaged,
    RecoverableDetached,
    RecoverablePublished,
    RecoverableDuplicateSource,
    RecoverableDuplicateGoal,
    BoundAfter,
    After,
    Ambiguous,
}

impl ExactTransition {
    fn public(self) -> CheckedArtifactTransition {
        match self {
            Self::ProofOnly | Self::After => CheckedArtifactTransition::After,
            Self::Before => CheckedArtifactTransition::Before,
            Self::BeforeBound
            | Self::RecoverableStaged
            | Self::RecoverableDetached
            | Self::RecoverablePublished
            | Self::RecoverableDuplicateSource
            | Self::RecoverableDuplicateGoal
            | Self::BoundAfter => CheckedArtifactTransition::Recoverable,
            Self::Ambiguous => CheckedArtifactTransition::Ambiguous,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationTable {
    ExistingReplace,
    MissingReplace,
    ExistingRemove,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryForm {
    Absent,
    Exact,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagedForm {
    Missing,
    ExactSource,
    ExactGoal,
    SameSourceAlias,
    SameGoalAlias,
    Other,
}

impl CheckedArtifact {
    pub(super) fn classify_replace(
        &self,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<CheckedArtifactTransition> {
        Ok(self.classify_replace_exact(expected, goal)?.public())
    }

    pub(super) fn classify_remove(
        &self,
        expected: &CheckedArtifactFact,
    ) -> ModelResult<CheckedArtifactTransition> {
        Ok(self.classify_remove_exact(expected)?.public())
    }

    pub(super) fn classify_replace_exact(
        &self,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<ExactTransition> {
        require_source(expected, self.code, &self.label)?;
        if expected == &CheckedArtifactFact::Bytes(goal.to_vec()) {
            return self.classify_proof_only(expected, goal);
        }
        self.classify_exact(expected, Some(goal))
    }

    pub(super) fn classify_remove_exact(
        &self,
        expected: &CheckedArtifactFact,
    ) -> ModelResult<ExactTransition> {
        if !matches!(expected, CheckedArtifactFact::Bytes(_)) {
            return Err(error(
                self.code,
                &self.label,
                "checked removal requires exact existing source bytes",
            ));
        }
        self.classify_exact(expected, None)
    }

    fn classify_proof_only(
        &self,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<ExactTransition> {
        let residue = self.inspect_family(expected, Some(goal))?;
        if residue.foreign
            || residue.authority.is_some()
            || residue.source.is_some()
            || residue.goal.is_some()
        {
            return Ok(ExactTransition::Ambiguous);
        }
        Ok(
            if self.observe_durable()? == CheckedArtifactFact::Bytes(goal.to_vec()) {
                ExactTransition::ProofOnly
            } else {
                ExactTransition::Ambiguous
            },
        )
    }

    fn classify_exact(
        &self,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
    ) -> ModelResult<ExactTransition> {
        let ParentState::Open {
            dir,
            identity: parent_identity,
        } = &self.parent
        else {
            return Ok(ExactTransition::Ambiguous);
        };
        if !self.parent_is_current(parent_identity)? {
            return Ok(ExactTransition::Ambiguous);
        }
        let residue = self.inspect_family(expected, goal)?;
        if residue.foreign {
            return Ok(ExactTransition::Ambiguous);
        }
        let leaf = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
        let goal_fact = goal.map_or(CheckedArtifactFact::Missing, |bytes| {
            CheckedArtifactFact::Bytes(bytes.to_vec())
        });
        let operation = match (expected, goal) {
            (CheckedArtifactFact::Bytes(_), Some(_)) => OperationTable::ExistingReplace,
            (CheckedArtifactFact::Missing, Some(_)) => OperationTable::MissingReplace,
            (CheckedArtifactFact::Bytes(_), None) => OperationTable::ExistingRemove,
            (CheckedArtifactFact::Missing | CheckedArtifactFact::Invalid, None)
            | (CheckedArtifactFact::Invalid, Some(_)) => return Ok(ExactTransition::Ambiguous),
        };
        let authority_current = residue.authority.as_ref().is_some_and(|authority| {
            authority.retained_parent_identity == parent_identity.durable
                && authority.matches_request(self, expected, goal)
                && matches!(
                    (&authority.retained_source, authority.operation, operation),
                    (
                        RetainedSource::Existing(_),
                        ArtifactOperation::Replace,
                        OperationTable::ExistingReplace
                    ) | (
                        RetainedSource::Missing,
                        ArtifactOperation::Replace,
                        OperationTable::MissingReplace
                    ) | (
                        RetainedSource::Existing(_),
                        ArtifactOperation::Remove,
                        OperationTable::ExistingRemove
                    )
                )
        });
        if residue.authority.is_some() && !authority_current {
            return Ok(ExactTransition::Ambiguous);
        }
        if let Some(authority) = &residue.authority
            && let RetainedSource::Existing(expected_identity) = &authority.retained_source
            && residue
                .source
                .as_ref()
                .is_some_and(|source| source.identity.durable != *expected_identity)
        {
            return Ok(ExactTransition::Ambiguous);
        }
        let source = residue
            .source
            .as_ref()
            .map_or(EntryForm::Absent, |_| EntryForm::Exact);
        let staged_goal = residue
            .goal
            .as_ref()
            .map_or(EntryForm::Absent, |_| EntryForm::Exact);
        let managed = managed_form(&leaf, expected, &goal_fact, &residue);
        let transition = classify_table(operation, authority_current, source, staged_goal, managed);
        if transition == ExactTransition::After && self.observe_durable()? != goal_fact {
            return Ok(ExactTransition::Ambiguous);
        }
        Ok(transition)
    }
}

fn managed_form(
    leaf: &super::observation::LeafObservation,
    expected: &CheckedArtifactFact,
    goal: &CheckedArtifactFact,
    residue: &super::residue::FamilyResidue,
) -> ManagedForm {
    if residue
        .source
        .as_ref()
        .is_some_and(|source| leaf.identity.as_ref() == Some(&source.identity))
    {
        return ManagedForm::SameSourceAlias;
    }
    if residue
        .goal
        .as_ref()
        .is_some_and(|staged| leaf.identity.as_ref() == Some(&staged.identity))
    {
        return ManagedForm::SameGoalAlias;
    }
    if leaf.fact == CheckedArtifactFact::Missing {
        ManagedForm::Missing
    } else if leaf.fact == *expected {
        ManagedForm::ExactSource
    } else if leaf.fact == *goal {
        ManagedForm::ExactGoal
    } else {
        ManagedForm::Other
    }
}

fn classify_table(
    operation: OperationTable,
    authority: bool,
    source: EntryForm,
    goal: EntryForm,
    managed: ManagedForm,
) -> ExactTransition {
    use EntryForm::{Absent, Exact};
    use ExactTransition::*;
    use ManagedForm::*;

    match (operation, authority, source, goal, managed) {
        (OperationTable::ExistingReplace, false, Absent, Absent, ExactSource)
        | (OperationTable::MissingReplace, false, Absent, Absent, Missing)
        | (OperationTable::ExistingRemove, false, Absent, Absent, ExactSource) => Before,

        (OperationTable::ExistingReplace, false, Absent, Absent, ExactGoal)
        | (OperationTable::MissingReplace, false, Absent, Absent, ExactGoal)
        | (OperationTable::ExistingRemove, false, Absent, Absent, Missing) => After,

        (OperationTable::ExistingReplace, true, Absent, Absent | Exact, ExactSource)
        | (OperationTable::MissingReplace, true, Absent, Absent, Missing)
        | (OperationTable::ExistingRemove, true, Absent, Absent, ExactSource) => BeforeBound,

        (OperationTable::MissingReplace, true, Absent, Exact, Missing) => RecoverableStaged,
        (OperationTable::ExistingReplace, true, Exact, Exact, Missing)
        | (OperationTable::ExistingRemove, true, Exact, Absent, Missing) => RecoverableDetached,
        (OperationTable::ExistingReplace, true, Exact, Absent, ExactGoal) => RecoverablePublished,
        (OperationTable::ExistingReplace, true, Exact, Absent | Exact, SameSourceAlias)
        | (OperationTable::ExistingRemove, true, Exact, Absent, SameSourceAlias) => {
            RecoverableDuplicateSource
        }
        (OperationTable::ExistingReplace, true, Exact, Exact, SameGoalAlias)
        | (OperationTable::MissingReplace, true, Absent, Exact, SameGoalAlias) => {
            RecoverableDuplicateGoal
        }
        (OperationTable::ExistingReplace, true, Absent, Absent, ExactGoal)
        | (OperationTable::MissingReplace, true, Absent, Absent, ExactGoal)
        | (OperationTable::ExistingRemove, true, Absent, Absent, Missing) => BoundAfter,

        _ => Ambiguous,
    }
}

fn require_source(
    expected: &CheckedArtifactFact,
    code: crate::model::ErrorCode,
    label: &str,
) -> ModelResult<()> {
    if matches!(
        expected,
        CheckedArtifactFact::Missing | CheckedArtifactFact::Bytes(_)
    ) {
        Ok(())
    } else {
        Err(error(
            code,
            label,
            "invalid source cannot authorize mutation",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_tables_accept_only_the_declared_rows() {
        use EntryForm::{Absent, Exact};
        use ExactTransition::*;
        use ManagedForm::*;

        let declared = [
            (
                OperationTable::ExistingReplace,
                false,
                Absent,
                Absent,
                ExactSource,
                Before,
            ),
            (
                OperationTable::ExistingReplace,
                false,
                Absent,
                Absent,
                ExactGoal,
                After,
            ),
            (
                OperationTable::ExistingReplace,
                true,
                Absent,
                Absent,
                ExactSource,
                BeforeBound,
            ),
            (
                OperationTable::ExistingReplace,
                true,
                Absent,
                Exact,
                ExactSource,
                BeforeBound,
            ),
            (
                OperationTable::ExistingReplace,
                true,
                Exact,
                Exact,
                Missing,
                RecoverableDetached,
            ),
            (
                OperationTable::ExistingReplace,
                true,
                Exact,
                Absent,
                ExactGoal,
                RecoverablePublished,
            ),
            (
                OperationTable::ExistingReplace,
                true,
                Exact,
                Absent,
                SameSourceAlias,
                RecoverableDuplicateSource,
            ),
            (
                OperationTable::ExistingReplace,
                true,
                Exact,
                Exact,
                SameSourceAlias,
                RecoverableDuplicateSource,
            ),
            (
                OperationTable::ExistingReplace,
                true,
                Exact,
                Exact,
                SameGoalAlias,
                RecoverableDuplicateGoal,
            ),
            (
                OperationTable::ExistingReplace,
                true,
                Absent,
                Absent,
                ExactGoal,
                BoundAfter,
            ),
            (
                OperationTable::MissingReplace,
                false,
                Absent,
                Absent,
                Missing,
                Before,
            ),
            (
                OperationTable::MissingReplace,
                false,
                Absent,
                Absent,
                ExactGoal,
                After,
            ),
            (
                OperationTable::MissingReplace,
                true,
                Absent,
                Absent,
                Missing,
                BeforeBound,
            ),
            (
                OperationTable::MissingReplace,
                true,
                Absent,
                Exact,
                Missing,
                RecoverableStaged,
            ),
            (
                OperationTable::MissingReplace,
                true,
                Absent,
                Exact,
                SameGoalAlias,
                RecoverableDuplicateGoal,
            ),
            (
                OperationTable::MissingReplace,
                true,
                Absent,
                Absent,
                ExactGoal,
                BoundAfter,
            ),
            (
                OperationTable::ExistingRemove,
                false,
                Absent,
                Absent,
                ExactSource,
                Before,
            ),
            (
                OperationTable::ExistingRemove,
                false,
                Absent,
                Absent,
                Missing,
                After,
            ),
            (
                OperationTable::ExistingRemove,
                true,
                Absent,
                Absent,
                ExactSource,
                BeforeBound,
            ),
            (
                OperationTable::ExistingRemove,
                true,
                Exact,
                Absent,
                Missing,
                RecoverableDetached,
            ),
            (
                OperationTable::ExistingRemove,
                true,
                Exact,
                Absent,
                SameSourceAlias,
                RecoverableDuplicateSource,
            ),
            (
                OperationTable::ExistingRemove,
                true,
                Absent,
                Absent,
                Missing,
                BoundAfter,
            ),
        ];
        for &(operation, authority, source, goal, managed, expected) in &declared {
            assert_eq!(
                classify_table(operation, authority, source, goal, managed),
                expected,
                "{operation:?}/{authority}/{source:?}/{goal:?}/{managed:?}",
            );
        }

        let operations = [
            OperationTable::ExistingReplace,
            OperationTable::MissingReplace,
            OperationTable::ExistingRemove,
        ];
        let entries = [Absent, Exact];
        let managed_forms = [
            Missing,
            ExactSource,
            ExactGoal,
            SameSourceAlias,
            SameGoalAlias,
            Other,
        ];
        let mut accepted = 0;
        for operation in operations {
            for authority in [false, true] {
                for source in entries {
                    for goal in entries {
                        for managed in managed_forms {
                            let actual =
                                classify_table(operation, authority, source, goal, managed);
                            if actual != Ambiguous {
                                accepted += 1;
                                assert!(
                                    declared.iter().any(|row| {
                                        (row.0, row.1, row.2, row.3, row.4)
                                            == (operation, authority, source, goal, managed)
                                            && row.5 == actual
                                    }),
                                    "undeclared accepted row: {operation:?}/{authority}/{source:?}/{goal:?}/{managed:?} -> {actual:?}",
                                );
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(accepted, declared.len());
    }
}
