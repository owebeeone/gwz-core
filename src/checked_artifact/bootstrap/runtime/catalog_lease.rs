//! Target-bound advisory leases for checked catalog mutation.

use std::ffi::OsStr;
use std::path::Path;

use super::paths::{open_or_create_file, revalidate_file};
use super::{BOOTSTRAP_GUARD_NAME, WorkspaceRuntimeLease, try_advisory_lock};
use crate::checked_artifact::capability::{CheckedFsError, PreCatalogRootKindV1};

mod target;

pub(in crate::checked_artifact) use target::CatalogLeaseTargetRequestV1;
#[cfg(test)]
use target::GIT_CATALOG_MUTATOR_LOCK_NAME;
use target::{
    CatalogTargetBindingV1, HeldCatalogTargetV1, RetainedCatalogTargetV1, reject_equivalent_alias,
};

struct PreparedCatalogTargetV1 {
    request: CatalogLeaseTargetRequestV1,
    binding: CatalogTargetBindingV1,
}

pub(in crate::checked_artifact) struct CatalogLeaseSetV1 {
    held: Vec<HeldCatalogTargetV1>,
}

impl CatalogLeaseSetV1 {
    pub(in crate::checked_artifact) fn try_acquire(
        requests: impl IntoIterator<Item = CatalogLeaseTargetRequestV1>,
    ) -> Result<Option<Self>, CheckedFsError> {
        let mut prepared = Vec::new();
        for request in requests {
            let target = RetainedCatalogTargetV1::retain(&request)?;
            prepared.push(PreparedCatalogTargetV1 {
                request,
                binding: target.binding,
            });
        }
        prepared.sort_by(|left, right| left.binding.order_key.cmp(&right.binding.order_key));
        prepared = deduplicate_exact_targets(prepared)?;

        // Phase one may converge only the fixed runtime lock grammar. Every
        // transient guard is released before the next target is visited.
        for expected in &prepared {
            let target = RetainedCatalogTargetV1::retain(&expected.request)?;
            require_same_binding(&expected.binding, &target.binding)?;
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
        let mut held = Vec::with_capacity(prepared.len());
        for expected in &prepared {
            let target = RetainedCatalogTargetV1::retain(&expected.request)?;
            if let Err(error) = require_same_binding(&expected.binding, &target.binding) {
                release_reverse(&mut held);
                return Err(error);
            }
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
                .and_then(|()| lease.target.revalidate())
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

    pub(in crate::checked_artifact) fn root_kind(&self) -> PreCatalogRootKindV1 {
        match self.source {
            CatalogMutationLeaseSourceV1::WorkspaceRuntime(_) => PreCatalogRootKindV1::Workspace,
            CatalogMutationLeaseSourceV1::LeaseSet(held) => held.target.binding.root_kind,
        }
    }

    pub(in crate::checked_artifact) fn canonical_target_path(&self) -> &Path {
        match self.source {
            CatalogMutationLeaseSourceV1::WorkspaceRuntime(runtime) => {
                runtime.workspace_root_path()
            }
            CatalogMutationLeaseSourceV1::LeaseSet(held) => &held.target.binding.canonical_path,
        }
    }

    #[cfg(test)]
    fn root_kind_for_test(&self) -> PreCatalogRootKindV1 {
        self.root_kind()
    }

    #[cfg(test)]
    fn canonical_target_path_for_test(&self) -> &Path {
        self.canonical_target_path()
    }

    #[cfg(test)]
    fn canonical_order_key_for_test(&self) -> &[u8] {
        match self.source {
            CatalogMutationLeaseSourceV1::WorkspaceRuntime(_) => &[],
            CatalogMutationLeaseSourceV1::LeaseSet(held) => &held.target.binding.order_key,
        }
    }
}

fn deduplicate_exact_targets(
    targets: Vec<PreparedCatalogTargetV1>,
) -> Result<Vec<PreparedCatalogTargetV1>, CheckedFsError> {
    let mut unique: Vec<PreparedCatalogTargetV1> = Vec::with_capacity(targets.len());
    for target in targets {
        if let Some(previous) = unique.last()
            && previous.binding.order_key == target.binding.order_key
        {
            require_same_binding(&previous.binding, &target.binding)?;
            continue;
        }
        unique.push(target);
    }
    Ok(unique)
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
