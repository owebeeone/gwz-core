#[cfg(test)]
use std::sync::atomic::AtomicU64;

use cap_std::fs::Dir;
use sha2::{Digest, Sha256};

use super::fault::{CheckedArtifactFault, fault};
use super::observation::{observe_leaf, observe_leaf_exact};
use super::residue::{goal_name, source_name};
use super::{
    CheckedArtifact, CheckedArtifactFact, CheckedArtifactTransition, ParentState, error, io_error,
};
use crate::model::ModelResult;

#[cfg(test)]
pub(super) static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl CheckedArtifact {
    pub(crate) fn observe_durable(&self) -> ModelResult<CheckedArtifactFact> {
        let ParentState::Open { dir, identity } = &self.parent else {
            return self.observe();
        };
        if !self.parent_is_current(*identity)? {
            return Ok(CheckedArtifactFact::Invalid);
        }
        let before = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
        self.sync_parent(dir)?;
        if !self.parent_is_current(*identity)? {
            return Ok(CheckedArtifactFact::Invalid);
        }
        let after = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
        if before.fact != after.fact || before.identity != after.identity {
            return Ok(CheckedArtifactFact::Invalid);
        }
        Ok(after.fact)
    }

    pub(crate) fn classify_replace(
        &self,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<CheckedArtifactTransition> {
        require_source(expected, self.code, &self.label)?;
        self.classify(expected, Some(goal))
    }

    pub(crate) fn classify_remove(
        &self,
        expected: &CheckedArtifactFact,
    ) -> ModelResult<CheckedArtifactTransition> {
        if !matches!(expected, CheckedArtifactFact::Bytes(_)) {
            return Err(error(
                self.code,
                &self.label,
                "checked removal requires exact existing source bytes",
            ));
        }
        self.classify(expected, None)
    }

    pub(crate) fn replace_exact(
        &self,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<()> {
        require_source(expected, self.code, &self.label)?;
        match self.classify_replace(expected, goal)? {
            CheckedArtifactTransition::After => return Ok(()),
            CheckedArtifactTransition::Ambiguous => {
                return Err(error(
                    self.code,
                    &self.label,
                    "replacement evidence is ambiguous",
                ));
            }
            CheckedArtifactTransition::Before | CheckedArtifactTransition::Recoverable => {}
        }
        let ParentState::Open { dir, identity } = &self.parent else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid",
            ));
        };
        let key = self.action_key(expected, Some(goal));
        let quarantine = self.open_quarantine(true)?.expect("created quarantine");
        let prior = self.inspect_residue(&key, expected, Some(goal))?;
        if prior.foreign {
            return Err(error(
                self.code,
                &self.label,
                "foreign checked-artifact recovery residue",
            ));
        }
        if prior.source.is_some()
            && observe_leaf(dir, &self.leaf, self.code, &self.label)?
                == CheckedArtifactFact::Bytes(goal.to_vec())
        {
            self.sync_parent(dir)?;
            self.cleanup_source(&quarantine, &key, expected, Some(goal))?;
            return (self.observe_durable()? == CheckedArtifactFact::Bytes(goal.to_vec()))
                .then_some(())
                .ok_or_else(|| {
                    error(
                        self.code,
                        &self.label,
                        "replacement failed exact durable verification",
                    )
                });
        }
        self.stage_goal(&quarantine, &key, goal)?;
        let mut residue = self.inspect_residue(&key, expected, Some(goal))?;
        if residue.foreign {
            return Err(error(
                self.code,
                &self.label,
                "foreign checked-artifact recovery residue",
            ));
        }

        if matches!(expected, CheckedArtifactFact::Bytes(_)) && residue.source.is_none() {
            fault(
                CheckedArtifactFault::BeforeFinalCheck,
                self.code,
                &self.label,
            )?;
            let source = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
            if source.fact != *expected || !self.parent_is_current(*identity)? {
                return Err(error(
                    self.code,
                    &self.label,
                    "source changed before checked replacement",
                ));
            }
            let source_identity = source.identity.expect("exact bytes have an identity");
            let source_name = source_name(&key, *identity);
            fault(
                CheckedArtifactFault::AfterFinalProof,
                self.code,
                &self.label,
            )?;
            super::platform::rename_relative(
                dir,
                &self.leaf,
                &quarantine,
                &source_name,
                false,
                self.code,
                &self.label,
            )?;
            fault(CheckedArtifactFault::AfterDetach, self.code, &self.label)?;
            let moved = observe_leaf_exact(&quarantine, &source_name, self.code, &self.label)?;
            if moved.fact != *expected
                || moved.identity != Some(source_identity)
                || !self.parent_is_current(*identity)?
            {
                self.restore_source(&quarantine, &source_name, dir)?;
                return Err(error(
                    self.code,
                    &self.label,
                    "source identity or parent changed at checked replacement",
                ));
            }
            residue = self.inspect_residue(&key, expected, Some(goal))?;
        } else if matches!(expected, CheckedArtifactFact::Missing) {
            fault(
                CheckedArtifactFault::BeforeFinalCheck,
                self.code,
                &self.label,
            )?;
            fault(
                CheckedArtifactFault::AfterFinalProof,
                self.code,
                &self.label,
            )?;
            if observe_leaf(dir, &self.leaf, self.code, &self.label)?
                != CheckedArtifactFact::Missing
                || !self.parent_is_current(*identity)?
            {
                return Err(error(
                    self.code,
                    &self.label,
                    "source changed before checked replacement",
                ));
            }
        }

        let leaf = observe_leaf(dir, &self.leaf, self.code, &self.label)?;
        if leaf == CheckedArtifactFact::Missing {
            let goal_name = goal_name(&key);
            if !residue.goal_staged {
                return Err(error(
                    self.code,
                    &self.label,
                    "checked replacement lost its staged goal",
                ));
            }
            super::platform::rename_relative(
                &quarantine,
                &goal_name,
                dir,
                &self.leaf,
                false,
                self.code,
                &self.label,
            )?;
            fault(CheckedArtifactFault::AfterMutation, self.code, &self.label)?;
        } else if leaf != CheckedArtifactFact::Bytes(goal.to_vec()) {
            return Err(error(
                self.code,
                &self.label,
                "replacement destination is not the exact goal",
            ));
        }

        self.sync_parent(dir)?;
        self.cleanup_source(&quarantine, &key, expected, Some(goal))?;
        if self.observe_durable()? != CheckedArtifactFact::Bytes(goal.to_vec()) {
            return Err(error(
                self.code,
                &self.label,
                "replacement failed exact durable verification",
            ));
        }
        Ok(())
    }

    pub(crate) fn remove_exact(&self, expected: &CheckedArtifactFact) -> ModelResult<()> {
        if self.classify_remove(expected)? == CheckedArtifactTransition::After {
            return Ok(());
        }
        let ParentState::Open { dir, identity } = &self.parent else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid",
            ));
        };
        let key = self.action_key(expected, None);
        let quarantine = self.open_quarantine(true)?.expect("created quarantine");
        let residue = self.inspect_residue(&key, expected, None)?;
        if residue.foreign || residue.goal_staged {
            return Err(error(
                self.code,
                &self.label,
                "removal evidence is ambiguous",
            ));
        }
        if residue.source.is_none() {
            fault(
                CheckedArtifactFault::BeforeFinalCheck,
                self.code,
                &self.label,
            )?;
            let source = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
            if source.fact != *expected || !self.parent_is_current(*identity)? {
                return Err(error(
                    self.code,
                    &self.label,
                    "source changed before checked removal",
                ));
            }
            let source_identity = source.identity.expect("exact bytes have an identity");
            let source_name = source_name(&key, *identity);
            fault(
                CheckedArtifactFault::AfterFinalProof,
                self.code,
                &self.label,
            )?;
            super::platform::rename_relative(
                dir,
                &self.leaf,
                &quarantine,
                &source_name,
                false,
                self.code,
                &self.label,
            )?;
            fault(CheckedArtifactFault::AfterDetach, self.code, &self.label)?;
            let moved = observe_leaf_exact(&quarantine, &source_name, self.code, &self.label)?;
            if moved.fact != *expected
                || moved.identity != Some(source_identity)
                || !self.parent_is_current(*identity)?
            {
                self.restore_source(&quarantine, &source_name, dir)?;
                return Err(error(
                    self.code,
                    &self.label,
                    "source identity or parent changed at checked removal",
                ));
            }
            fault(CheckedArtifactFault::AfterMutation, self.code, &self.label)?;
        }
        self.sync_parent(dir)?;
        self.cleanup_source(&quarantine, &key, expected, None)?;
        if self.observe_durable()? != CheckedArtifactFact::Missing {
            return Err(error(
                self.code,
                &self.label,
                "removal failed exact durable verification",
            ));
        }
        Ok(())
    }

    fn classify(
        &self,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
    ) -> ModelResult<CheckedArtifactTransition> {
        let ParentState::Open { dir, identity } = &self.parent else {
            return Ok(CheckedArtifactTransition::Ambiguous);
        };
        if !self.parent_is_current(*identity)? {
            return Ok(CheckedArtifactTransition::Ambiguous);
        }
        let key = self.action_key(expected, goal);
        let residue = self.inspect_residue(&key, expected, goal)?;
        if residue.foreign
            || residue.source.as_ref().is_some_and(|source| {
                source.parent_identity != *identity || source.identity == (0, 0)
            })
        {
            return Ok(CheckedArtifactTransition::Ambiguous);
        }
        let leaf = observe_leaf(dir, &self.leaf, self.code, &self.label)?;
        let goal_fact = goal.map_or(CheckedArtifactFact::Missing, |bytes| {
            CheckedArtifactFact::Bytes(bytes.to_vec())
        });
        if residue.source.is_none() && !residue.goal_staged && leaf == goal_fact {
            return self.durable_goal(&goal_fact).map(|exact| {
                if exact {
                    CheckedArtifactTransition::After
                } else {
                    CheckedArtifactTransition::Ambiguous
                }
            });
        }
        if residue.source.is_none() && !residue.goal_staged && leaf == *expected {
            return Ok(CheckedArtifactTransition::Before);
        }
        let staged_is_legal = goal.is_some() && residue.goal_staged;
        let source_is_legal =
            residue.source.is_some() && matches!(expected, CheckedArtifactFact::Bytes(_));
        if (staged_is_legal || source_is_legal)
            && (leaf == *expected || leaf == goal_fact || leaf == CheckedArtifactFact::Missing)
        {
            return Ok(CheckedArtifactTransition::Recoverable);
        }
        Ok(CheckedArtifactTransition::Ambiguous)
    }

    fn durable_goal(&self, goal: &CheckedArtifactFact) -> ModelResult<bool> {
        Ok(self.observe_durable()? == *goal)
    }

    fn action_key(&self, expected: &CheckedArtifactFact, goal: Option<&[u8]>) -> String {
        let mut bytes = b"gwz.checked-artifact/v2\0".to_vec();
        bytes.extend(self.relative.to_string_lossy().as_bytes());
        bytes.push(0);
        match expected {
            CheckedArtifactFact::Missing => bytes.push(0),
            CheckedArtifactFact::Bytes(value) => {
                bytes.push(1);
                bytes.extend(Sha256::digest(value));
            }
            CheckedArtifactFact::Invalid => bytes.push(2),
        }
        match goal {
            Some(value) => {
                bytes.push(1);
                bytes.extend(Sha256::digest(value));
            }
            None => bytes.push(0),
        }
        format!("{:x}", Sha256::digest(bytes))
    }

    fn sync_parent(&self, dir: &Dir) -> ModelResult<()> {
        fault(
            CheckedArtifactFault::BeforeDurability,
            self.code,
            &self.label,
        )?;
        super::platform::sync_parent(dir)
            .map_err(|cause| io_error(self.code, &self.label, cause))?;
        fault(
            CheckedArtifactFault::AfterDurability,
            self.code,
            &self.label,
        )
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
