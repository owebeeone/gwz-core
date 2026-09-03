//! Target-bound advisory leases for checked catalog mutation.

#[cfg(test)]
use std::cell::Cell;
use std::ffi::OsStr;

use super::paths::{open_or_create_file, revalidate_file};
use super::{BOOTSTRAP_GUARD_NAME, WorkspaceRuntimeLease, try_advisory_lock};
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};

mod alias;
mod association;
mod target;
mod witness;

use alias::reject_equivalent_alias;
#[cfg(test)]
use alias::{
    MAX_CATALOG_ALIAS_PARENT_ENTRIES_V1, native_name_charge_for_test,
    reject_equivalent_alias_with_mode_for_test,
};
use association::CatalogGitAssociationBindingV1;
pub(in crate::checked_artifact) use target::CatalogLeaseTargetRequestV1;
#[cfg(test)]
use target::GIT_CATALOG_MUTATOR_LOCK_NAME;
/// DR-1 ship (1) W3's read-only admission probe
/// (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §2, 2026-09-03): the decision
/// point's door into the same `dir_identity` calls `finish` makes.
pub(in crate::checked_artifact) use target::probe_workspace_admission;
use target::{CatalogTargetBindingV1, HeldCatalogTargetV1, RetainedCatalogTargetV1};
pub(crate) use witness::CatalogLeaseTargetWitnessV1;

struct PreparedCatalogRequestV1 {
    request: CatalogLeaseTargetRequestV1,
    git_association: Option<CatalogGitAssociationBindingV1>,
}

struct PreparedCatalogTargetV1 {
    requests: Vec<PreparedCatalogRequestV1>,
    binding: CatalogTargetBindingV1,
}

struct RetainedCatalogTargetGroupV1 {
    primary: RetainedCatalogTargetV1,
    associated_targets: Vec<RetainedCatalogTargetV1>,
}

pub(in crate::checked_artifact) const MAX_CATALOG_LEASE_TARGETS_V1: usize = 4_096;

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_CATALOG_BATCH_ALLOCATION: Cell<bool> = const { Cell::new(false) };
}

pub(in crate::checked_artifact) struct CatalogLeaseTargetBatchV1 {
    requests: Vec<CatalogLeaseTargetRequestV1>,
}

impl CatalogLeaseTargetBatchV1 {
    pub(in crate::checked_artifact) fn try_new(
        requests: impl IntoIterator<Item = CatalogLeaseTargetRequestV1>,
    ) -> Result<Self, CheckedFsError> {
        let mut bounded = Vec::new();
        for request in requests.into_iter().take(MAX_CATALOG_LEASE_TARGETS_V1 + 1) {
            if bounded.len() == MAX_CATALOG_LEASE_TARGETS_V1 {
                return Err(CheckedFsError::ambiguous(
                    "catalog lease target capacity",
                    "target batch exceeds 4,096 entries",
                ));
            }
            try_reserve_batch(&mut bounded, 1)?;
            bounded.push(request);
        }
        if bounded.is_empty() {
            return Err(CheckedFsError::ambiguous(
                "catalog lease target capacity",
                "target batch is empty",
            ));
        }
        Ok(Self { requests: bounded })
    }
}

pub(in crate::checked_artifact) struct CatalogLeaseSetV1 {
    held: Vec<HeldCatalogTargetV1>,
}

impl CatalogLeaseSetV1 {
    pub(in crate::checked_artifact) fn try_acquire(
        batch: CatalogLeaseTargetBatchV1,
    ) -> Result<Option<Self>, CheckedFsError> {
        let mut prepared = Vec::new();
        try_reserve_batch(&mut prepared, batch.requests.len())?;
        for request in batch.requests {
            let target = RetainedCatalogTargetV1::retain(&request)?;
            let (binding, git_association) = target.into_prepared_bindings();
            let mut requests = Vec::new();
            try_reserve_batch(&mut requests, 1)?;
            requests.push(PreparedCatalogRequestV1 {
                request,
                git_association,
            });
            prepared.push(PreparedCatalogTargetV1 { requests, binding });
        }
        #[cfg(test)]
        super::fault::run(super::fault::RuntimeBootstrapFault::CatalogInitialRetentionComplete);
        prepared = deduplicate_exact_locations(prepared)?;
        prepared
            .sort_unstable_by(|left, right| left.binding.order_key.cmp(&right.binding.order_key));

        // Phase one may converge only the fixed runtime lock grammar. Every
        // transient guard is released before the next target is visited.
        for expected in &prepared {
            let target = RetainedCatalogTargetGroupV1::retain(expected)?;
            reject_equivalent_alias(
                target.guard_parent(),
                OsStr::new(BOOTSTRAP_GUARD_NAME),
                "runtime bootstrap guard",
            )?;
            let guard_file = open_or_create_file(
                target.guard_parent(),
                OsStr::new(BOOTSTRAP_GUARD_NAME),
                "runtime bootstrap guard",
            )?;
            let Some(guard) = try_advisory_lock(guard_file)? else {
                return Ok(None);
            };
            target.revalidate()?;
            target.prepare_final_slot()?;
            target.revalidate()?;
            revalidate_file(
                target.guard_parent(),
                OsStr::new(BOOTSTRAP_GUARD_NAME),
                guard.file(),
                "runtime bootstrap guard",
            )?;
            drop(guard);
        }
        #[cfg(test)]
        super::fault::run(super::fault::RuntimeBootstrapFault::CatalogPreparation);

        // Phase two performs no preparation mutation. It opens the prepared
        // slots in canonical order and drops the whole prefix on any failure.
        let mut held = Vec::new();
        try_reserve_batch(&mut held, prepared.len())?;
        for expected in &prepared {
            let target = match RetainedCatalogTargetGroupV1::retain(expected) {
                Ok(target) => target,
                Err(error) => {
                    release_reverse(&mut held);
                    return Err(error);
                }
            };
            match target.acquire_final() {
                Ok(Some(lease)) => held.push(lease),
                Ok(None) => {
                    release_reverse(&mut held);
                    return Ok(None);
                }
                Err(error) => {
                    release_reverse(&mut held);
                    return Err(error);
                }
            }
        }
        for (expected, lease) in prepared.iter().zip(&held) {
            if let Err(error) = require_same_binding(&expected.binding, &lease.target.binding)
                .and_then(|()| lease.revalidate_held())
            {
                release_reverse(&mut held);
                return Err(error);
            }
        }
        Ok(Some(Self { held }))
    }

    pub(in crate::checked_artifact) fn len(&self) -> usize {
        self.held.len()
    }

    pub(in crate::checked_artifact) fn leases(
        &self,
    ) -> impl ExactSizeIterator<Item = CatalogMutationLeaseV1<'_>> {
        self.held.iter().map(CatalogMutationLeaseV1::from_held)
    }
}

impl Drop for CatalogLeaseSetV1 {
    fn drop(&mut self) {
        release_reverse(&mut self.held);
    }
}

pub(crate) struct CatalogMutationLeaseV1<'lease> {
    source: CatalogMutationLeaseSourceV1<'lease>,
}

enum CatalogMutationLeaseSourceV1<'lease> {
    WorkspaceRuntime(&'lease WorkspaceRuntimeLease),
    LeaseSet(&'lease HeldCatalogTargetV1),
}

impl<'lease> CatalogMutationLeaseV1<'lease> {
    pub(super) fn from_workspace_runtime(runtime: &'lease WorkspaceRuntimeLease) -> Self {
        Self {
            source: CatalogMutationLeaseSourceV1::WorkspaceRuntime(runtime),
        }
    }

    fn from_held(held: &'lease HeldCatalogTargetV1) -> Self {
        Self {
            source: CatalogMutationLeaseSourceV1::LeaseSet(held),
        }
    }

    pub(in crate::checked_artifact) fn begin_preflight(
        self,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        CatalogLeaseTargetWitnessV1::try_new(self)
    }

    #[cfg(test)]
    fn canonical_order_key_for_test(&self) -> &[u8] {
        match self.source {
            CatalogMutationLeaseSourceV1::WorkspaceRuntime(_) => &[],
            CatalogMutationLeaseSourceV1::LeaseSet(held) => &held.target.binding.order_key,
        }
    }
}

fn deduplicate_exact_locations(
    mut targets: Vec<PreparedCatalogTargetV1>,
) -> Result<Vec<PreparedCatalogTargetV1>, CheckedFsError> {
    targets.sort_unstable_by(|left, right| {
        left.binding
            .canonical_path
            .cmp(&right.binding.canonical_path)
    });
    let mut unique: Vec<PreparedCatalogTargetV1> = Vec::new();
    try_reserve_batch(&mut unique, targets.len())?;
    for mut target in targets {
        if let Some(previous) = unique.last_mut()
            && previous.binding.canonical_path == target.binding.canonical_path
        {
            require_same_binding(&previous.binding, &target.binding)?;
            merge_prepared_requests(&mut previous.requests, &mut target.requests)?;
            continue;
        }
        unique.push(target);
    }
    Ok(unique)
}

impl RetainedCatalogTargetGroupV1 {
    fn retain(expected: &PreparedCatalogTargetV1) -> Result<Self, CheckedFsError> {
        let mut requests = expected.requests.iter();
        let first = requests.next().expect("prepared target has a request");
        let primary = retain_prepared_request(&expected.binding, first)?;
        let mut associated_targets = Vec::new();
        try_reserve_batch(
            &mut associated_targets,
            expected.requests.len().saturating_sub(1),
        )?;
        for request in requests {
            associated_targets.push(retain_prepared_request(&expected.binding, request)?);
        }
        let group = Self {
            primary,
            associated_targets,
        };
        group.revalidate()?;
        Ok(group)
    }

    fn revalidate(&self) -> Result<(), CheckedFsError> {
        self.primary.revalidate()?;
        for target in &self.associated_targets {
            target.revalidate()?;
        }
        Ok(())
    }

    fn guard_parent(&self) -> &cap_std::fs::Dir {
        self.primary.guard_parent()
    }

    fn prepare_final_slot(&self) -> Result<(), CheckedFsError> {
        self.primary.prepare_final_slot()
    }

    fn acquire_final(self) -> Result<Option<HeldCatalogTargetV1>, CheckedFsError> {
        self.primary.acquire_final(self.associated_targets)
    }
}

fn retain_prepared_request(
    binding: &CatalogTargetBindingV1,
    expected: &PreparedCatalogRequestV1,
) -> Result<RetainedCatalogTargetV1, CheckedFsError> {
    let target = RetainedCatalogTargetV1::retain(&expected.request)?;
    require_same_binding(binding, &target.binding)?;
    if expected.git_association.as_ref() != target.git_association_binding() {
        return Err(CheckedFsError::ambiguous(
            "catalog repository/worktree membership",
            "request association changed after initial retention",
        ));
    }
    Ok(target)
}

fn merge_prepared_requests(
    existing: &mut Vec<PreparedCatalogRequestV1>,
    incoming: &mut Vec<PreparedCatalogRequestV1>,
) -> Result<(), CheckedFsError> {
    for request in incoming.drain(..) {
        let duplicate = existing
            .iter()
            .find(|candidate| request_location(candidate) == request_location(&request));
        if let Some(previous) = duplicate {
            if previous.git_association != request.git_association {
                return Err(CheckedFsError::ambiguous(
                    "catalog repository/worktree membership",
                    "one request location has different retained membership",
                ));
            }
            continue;
        }
        try_reserve_batch(existing, 1)?;
        existing.push(request);
    }
    Ok(())
}

fn request_location(request: &PreparedCatalogRequestV1) -> Option<&std::path::Path> {
    request
        .git_association
        .as_ref()
        .map(CatalogGitAssociationBindingV1::request_path)
}

fn batch_allocation_error(_: std::collections::TryReserveError) -> CheckedFsError {
    CheckedFsError::unsupported(
        PlatformCapability::RuntimeAdvisoryLock,
        "catalog lease batch allocation failed",
    )
}

fn try_reserve_batch<T>(values: &mut Vec<T>, additional: usize) -> Result<(), CheckedFsError> {
    #[cfg(test)]
    if FAIL_NEXT_CATALOG_BATCH_ALLOCATION.with(|failure| failure.replace(false)) {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::RuntimeAdvisoryLock,
            "injected catalog lease batch allocation failure",
        ));
    }
    values
        .try_reserve_exact(additional)
        .map_err(batch_allocation_error)
}

#[cfg(test)]
fn fail_next_catalog_batch_allocation_for_test() {
    FAIL_NEXT_CATALOG_BATCH_ALLOCATION.with(|failure| failure.set(true));
}

fn require_same_binding(
    expected: &CatalogTargetBindingV1,
    actual: &CatalogTargetBindingV1,
) -> Result<(), CheckedFsError> {
    if expected != actual {
        return Err(CheckedFsError::ambiguous(
            "catalog lease target",
            "target identity, path, repository relationship, or ordering key changed",
        ));
    }
    Ok(())
}

fn release_reverse(held: &mut Vec<HeldCatalogTargetV1>) {
    while held.pop().is_some() {}
}

#[cfg(test)]
mod tests;
