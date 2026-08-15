#[cfg(test)]
use std::sync::atomic::AtomicU64;

use super::authority::{ArtifactOperation, CheckedArtifactAuthority, RetainedSource};
use super::classification::ExactTransition;
use super::fault::{CheckedArtifactFault, fault};
use super::observation::{io_op_error, observe_leaf_exact};
use super::{CheckedArtifact, CheckedArtifactFact, CheckedArtifactTransition, ParentState, error};
use crate::model::ModelResult;

#[cfg(test)]
pub(super) static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl CheckedArtifact {
    pub(super) fn observe_durable(&self) -> ModelResult<CheckedArtifactFact> {
        let ParentState::Open { dir, identity } = &self.parent else {
            return self.observe();
        };
        if !self.parent_is_current(identity)? {
            return Ok(CheckedArtifactFact::Invalid);
        }
        let before = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
        self.sync_dir(
            dir,
            CheckedArtifactFault::BeforeDurability,
            CheckedArtifactFault::AfterDurability,
        )?;
        if !self.parent_is_current(identity)? {
            return Ok(CheckedArtifactFact::Invalid);
        }
        let after = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
        if before.fact != after.fact || before.identity != after.identity {
            return Ok(CheckedArtifactFact::Invalid);
        }
        Ok(after.fact)
    }

    pub(super) fn replace_exact(
        &self,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<()> {
        match self.classify_replace_exact(expected, goal)? {
            ExactTransition::ProofOnly | ExactTransition::After => return Ok(()),
            ExactTransition::Ambiguous => {
                return Err(error(
                    self.code,
                    &self.label,
                    "replacement evidence is ambiguous",
                ));
            }
            ExactTransition::Before
            | ExactTransition::BeforeBound
            | ExactTransition::RecoverableStaged
            | ExactTransition::RecoverableDetached
            | ExactTransition::RecoverablePublished
            | ExactTransition::RecoverableDuplicateSource
            | ExactTransition::RecoverableDuplicateGoal
            | ExactTransition::BoundAfter => {}
        }
        let ParentState::Open { .. } = &self.parent else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid",
            ));
        };
        fault(
            CheckedArtifactFault::BeforeFinalCheck,
            self.code,
            &self.label,
        )?;
        let source = self.observe_leaf_exact_current()?;
        fault(
            CheckedArtifactFault::AfterFinalProof,
            self.code,
            &self.label,
        )?;
        let authority = self.ensure_authority(expected, Some(goal), &source)?;
        if authority.operation != ArtifactOperation::Replace {
            return Err(error(
                self.code,
                &self.label,
                "replacement authority has the wrong operation",
            ));
        }
        let managed = self.observe_leaf_exact_current()?;
        if managed.fact != CheckedArtifactFact::Bytes(goal.to_vec()) {
            self.ensure_goal(&authority, expected, goal)?;
        }
        if matches!(authority.retained_source, RetainedSource::Existing(_)) {
            self.detach_existing(&authority, expected, Some(goal))?;
        } else {
            let managed = self.observe_leaf_exact_current()?;
            if managed.fact != CheckedArtifactFact::Missing
                && managed.fact != CheckedArtifactFact::Bytes(goal.to_vec())
            {
                return Err(error(
                    self.code,
                    &self.label,
                    "missing-source replacement destination changed before publication",
                ));
            }
        }
        self.publish_goal(&authority, expected, goal)?;
        self.finish_replace(&authority, expected, goal)?;
        (self.classify_replace(expected, goal)? == CheckedArtifactTransition::After)
            .then_some(())
            .ok_or_else(|| {
                error(
                    self.code,
                    &self.label,
                    "replacement failed exact durable verification",
                )
            })
    }

    pub(super) fn remove_exact(&self, expected: &CheckedArtifactFact) -> ModelResult<()> {
        match self.classify_remove_exact(expected)? {
            ExactTransition::After => return Ok(()),
            ExactTransition::Ambiguous => {
                return Err(error(
                    self.code,
                    &self.label,
                    "removal evidence is ambiguous",
                ));
            }
            ExactTransition::Before
            | ExactTransition::BeforeBound
            | ExactTransition::RecoverableDetached
            | ExactTransition::RecoverableDuplicateSource
            | ExactTransition::BoundAfter => {}
            ExactTransition::ProofOnly
            | ExactTransition::RecoverableStaged
            | ExactTransition::RecoverablePublished
            | ExactTransition::RecoverableDuplicateGoal => {
                return Err(error(
                    self.code,
                    &self.label,
                    "removal classifier returned an invalid operation state",
                ));
            }
        }
        fault(
            CheckedArtifactFault::BeforeFinalCheck,
            self.code,
            &self.label,
        )?;
        let source = self.observe_leaf_exact_current()?;
        fault(
            CheckedArtifactFault::AfterFinalProof,
            self.code,
            &self.label,
        )?;
        let authority = self.ensure_authority(expected, None, &source)?;
        if authority.operation != ArtifactOperation::Remove
            || !matches!(authority.retained_source, RetainedSource::Existing(_))
        {
            return Err(error(
                self.code,
                &self.label,
                "removal authority has invalid source semantics",
            ));
        }
        self.detach_existing(&authority, expected, None)?;
        fault(CheckedArtifactFault::AfterMutation, self.code, &self.label)?;
        self.finish_remove(&authority, expected)?;
        (self.classify_remove(expected)? == CheckedArtifactTransition::After)
            .then_some(())
            .ok_or_else(|| {
                error(
                    self.code,
                    &self.label,
                    "removal failed exact durable verification",
                )
            })
    }

    fn detach_existing(
        &self,
        authority: &CheckedArtifactAuthority,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
    ) -> ModelResult<()> {
        let ParentState::Open {
            dir,
            identity: parent_identity,
        } = &self.parent
        else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is unavailable",
            ));
        };
        let RetainedSource::Existing(expected_identity) = &authority.retained_source else {
            return Err(error(
                self.code,
                &self.label,
                "authority has no existing source",
            ));
        };
        let private = self.open_private(false)?.ok_or_else(|| {
            error(
                self.code,
                &self.label,
                "private authority directory disappeared",
            )
        })?;
        let residue = self.inspect_family(expected, goal)?;
        if residue.foreign || residue.authority.as_ref() != Some(authority) {
            return Err(error(
                self.code,
                &self.label,
                "source authority changed before detach",
            ));
        }
        let leaf = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
        if let Some(source) = residue.source {
            if source.identity.durable != *expected_identity {
                return Err(error(
                    self.code,
                    &self.label,
                    "quarantined source identity changed",
                ));
            }
            if leaf.fact == CheckedArtifactFact::Missing {
                return Ok(());
            }
            if goal.is_some_and(|bytes| leaf.fact == CheckedArtifactFact::Bytes(bytes.to_vec())) {
                return Ok(());
            }
            if leaf.fact == *expected && leaf.identity.as_ref() == Some(&source.identity) {
                dir.remove_file(&self.leaf).map_err(|cause| {
                    io_op_error(self.code, &self.label, "remove managed source leaf", cause)
                })?;
                self.sync_dir(
                    dir,
                    CheckedArtifactFault::BeforeSourceRetirement,
                    CheckedArtifactFault::AfterSourceRetirement,
                )?;
                return Ok(());
            }
            return Err(error(
                self.code,
                &self.label,
                "managed source conflicts with quarantined source",
            ));
        }
        if goal.is_some_and(|bytes| leaf.fact == CheckedArtifactFact::Bytes(bytes.to_vec()))
            || (goal.is_none() && leaf.fact == CheckedArtifactFact::Missing)
        {
            return Ok(());
        }
        if leaf.fact != *expected
            || leaf
                .identity
                .as_ref()
                .is_none_or(|identity| identity.durable != *expected_identity)
            || !self.parent_is_current(parent_identity)?
            || parent_identity.durable != authority.retained_parent_identity
        {
            return Err(error(
                self.code,
                &self.label,
                "source or retained parent changed before detach",
            ));
        }
        let source_identity = leaf.identity.expect("exact existing source has identity");
        let source_name = super::authority::source_name(
            &authority.family_key,
            &authority.action_key,
            &source_identity.name_digest(),
        );
        super::platform::rename_relative(
            dir,
            &self.leaf,
            &private,
            source_name.as_ref(),
            false,
            self.code,
            &self.label,
        )?;
        fault(CheckedArtifactFault::AfterDetach, self.code, &self.label)?;
        self.sync_private(
            &private,
            CheckedArtifactFault::BeforeDestinationDurability,
            CheckedArtifactFault::AfterDestinationDurability,
        )?;
        self.sync_dir(
            dir,
            CheckedArtifactFault::BeforeSourceRetirement,
            CheckedArtifactFault::AfterSourceRetirement,
        )?;
        let moved = observe_leaf_exact(&private, source_name.as_ref(), self.code, &self.label)?;
        if moved.fact != *expected
            || moved.identity.as_ref() != Some(&source_identity)
            || observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?.fact
                != CheckedArtifactFact::Missing
            || !self.parent_is_current(parent_identity)?
        {
            return Err(error(
                self.code,
                &self.label,
                "source detach failed exact durable reobservation",
            ));
        }
        Ok(())
    }

    fn publish_goal(
        &self,
        authority: &CheckedArtifactAuthority,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<()> {
        let ParentState::Open { dir, identity } = &self.parent else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is unavailable",
            ));
        };
        let private = self.open_private(false)?.ok_or_else(|| {
            error(
                self.code,
                &self.label,
                "private authority directory disappeared",
            )
        })?;
        let residue = self.inspect_family(expected, Some(goal))?;
        if residue.foreign || residue.authority.as_ref() != Some(authority) {
            return Err(error(
                self.code,
                &self.label,
                "goal authority changed before publication",
            ));
        }
        let leaf = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
        if leaf.fact == CheckedArtifactFact::Bytes(goal.to_vec()) {
            if residue
                .goal
                .as_ref()
                .is_some_and(|staged| leaf.identity.as_ref() != Some(&staged.identity))
            {
                return Err(error(
                    self.code,
                    &self.label,
                    "managed goal is a different-identity duplicate",
                ));
            }
            return Ok(());
        }
        if leaf.fact != CheckedArtifactFact::Missing || !self.parent_is_current(identity)? {
            return Err(error(
                self.code,
                &self.label,
                "replacement destination changed before goal publication",
            ));
        }
        let staged = residue.goal.ok_or_else(|| {
            error(
                self.code,
                &self.label,
                "checked replacement lost its staged goal",
            )
        })?;
        super::platform::rename_relative(
            &private,
            &staged.name,
            dir,
            &self.leaf,
            false,
            self.code,
            &self.label,
        )?;
        fault(CheckedArtifactFault::AfterMutation, self.code, &self.label)?;
        self.sync_dir(
            dir,
            CheckedArtifactFault::BeforeManagedDestinationDurability,
            CheckedArtifactFault::AfterManagedDestinationDurability,
        )?;
        self.sync_private(
            &private,
            CheckedArtifactFault::BeforeQuarantineSourceRetirement,
            CheckedArtifactFault::AfterQuarantineSourceRetirement,
        )?;
        let managed = observe_leaf_exact(dir, &self.leaf, self.code, &self.label)?;
        if managed.fact != CheckedArtifactFact::Bytes(goal.to_vec())
            || managed.identity.as_ref() != Some(&staged.identity)
            || !self.parent_is_current(identity)?
        {
            return Err(error(
                self.code,
                &self.label,
                "managed goal failed exact post-publication verification",
            ));
        }
        Ok(())
    }

    fn sync_dir(
        &self,
        dir: &cap_std::fs::Dir,
        before: CheckedArtifactFault,
        after: CheckedArtifactFault,
    ) -> ModelResult<()> {
        fault(before, self.code, &self.label)?;
        super::platform::sync_parent(dir).map_err(|cause| {
            io_op_error(
                self.code,
                &self.label,
                "sync managed parent directory",
                cause,
            )
        })?;
        fault(after, self.code, &self.label)
    }

    fn sync_private(
        &self,
        dir: &cap_std::fs::Dir,
        before: CheckedArtifactFault,
        after: CheckedArtifactFault,
    ) -> ModelResult<()> {
        fault(before, self.code, &self.label)?;
        super::platform::private_barrier(dir, self.code, &self.label)?;
        fault(after, self.code, &self.label)
    }
}
