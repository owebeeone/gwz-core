use std::ffi::OsStr;

use super::authority::{CheckedArtifactAuthority, RetainedSource, authority_name};
use super::fault::{CheckedArtifactFault, fault};
use super::{CheckedArtifact, CheckedArtifactFact, ParentState, error, io_error};
use crate::model::ModelResult;

impl CheckedArtifact {
    pub(super) fn finish_replace(
        &self,
        authority: &CheckedArtifactAuthority,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<()> {
        self.finish(authority, expected, Some(goal))
    }

    pub(super) fn finish_remove(
        &self,
        authority: &CheckedArtifactAuthority,
        expected: &CheckedArtifactFact,
    ) -> ModelResult<()> {
        self.finish(authority, expected, None)
    }

    fn finish(
        &self,
        authority: &CheckedArtifactAuthority,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
    ) -> ModelResult<()> {
        let ParentState::Open {
            identity: parent_identity,
            ..
        } = &self.parent
        else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid during cleanup",
            ));
        };
        if !self.parent_is_current(parent_identity)?
            || parent_identity.durable != authority.retained_parent_identity
        {
            return Err(error(
                self.code,
                &self.label,
                "retained parent changed before checked-artifact cleanup",
            ));
        }
        let desired = goal.map_or(CheckedArtifactFact::Missing, |bytes| {
            CheckedArtifactFact::Bytes(bytes.to_vec())
        });
        if self.observe_durable()? != desired {
            return Err(error(
                self.code,
                &self.label,
                "managed destination is not the exact durable goal",
            ));
        }
        let private = self.open_private(false)?.ok_or_else(|| {
            error(
                self.code,
                &self.label,
                "private authority disappeared before cleanup",
            )
        })?;
        let residue = self.inspect_family(expected, goal)?;
        if residue.foreign || residue.authority.as_ref() != Some(authority) {
            return Err(error(
                self.code,
                &self.label,
                "family authority changed before cleanup",
            ));
        }
        if let Some(staged) = residue.goal {
            let managed = self.observe_leaf_exact_current()?;
            if managed.identity.as_ref() != Some(&staged.identity) {
                return Err(error(
                    self.code,
                    &self.label,
                    "staged goal is a different-identity duplicate",
                ));
            }
            private
                .remove_file(&staged.name)
                .map_err(|cause| io_error(self.code, &self.label, cause))?;
            super::platform::private_barrier(&private, self.code, &self.label)?;
        }
        if let Some(source) = residue.source {
            let RetainedSource::Existing(expected_identity) = &authority.retained_source else {
                return Err(error(
                    self.code,
                    &self.label,
                    "missing-source authority unexpectedly owns source residue",
                ));
            };
            if source.identity.durable != *expected_identity {
                return Err(error(
                    self.code,
                    &self.label,
                    "quarantined source identity changed before cleanup",
                ));
            }
            fault(
                CheckedArtifactFault::BeforeSourceCleanup,
                self.code,
                &self.label,
            )?;
            let rechecked = self.inspect_family(expected, goal)?;
            if rechecked.foreign
                || rechecked.authority.as_ref() != Some(authority)
                || rechecked.source.as_ref().is_none_or(|value| {
                    value.name != source.name || value.identity != source.identity
                })
                || self.observe_durable()? != desired
                || !self.parent_is_current(parent_identity)?
            {
                return Err(error(
                    self.code,
                    &self.label,
                    "cleanup evidence changed before source retirement",
                ));
            }
            private
                .remove_file(&source.name)
                .map_err(|cause| io_error(self.code, &self.label, cause))?;
            super::platform::private_barrier(&private, self.code, &self.label)?;
            fault(
                CheckedArtifactFault::AfterSourceCleanup,
                self.code,
                &self.label,
            )?;
        }
        let rechecked = self.inspect_family(expected, goal)?;
        if rechecked.foreign
            || rechecked.authority.as_ref() != Some(authority)
            || rechecked.source.is_some()
            || rechecked.goal.is_some()
            || self.observe_durable()? != desired
        {
            return Err(error(
                self.code,
                &self.label,
                "family is not closed for authority retirement",
            ));
        }
        let authority_name = authority_name(&authority.family_key, &authority.action_key);
        self.rebarrier_exact(&private, OsStr::new(&authority_name))?;
        fault(
            CheckedArtifactFault::BeforeAuthorityCleanup,
            self.code,
            &self.label,
        )?;
        private
            .remove_file(&authority_name)
            .map_err(|cause| io_error(self.code, &self.label, cause))?;
        super::platform::private_barrier(&private, self.code, &self.label)?;
        fault(
            CheckedArtifactFault::AfterAuthorityCleanup,
            self.code,
            &self.label,
        )
    }
}
