use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::artifact::{self, ArtifactSourceKind, ManifestArtifact, ManifestMember};
use crate::git::{
    GitBackend, GitStashPushOptions, GitStashRestoreOptions, GitStashTarget, GitStatus,
    GitStatusOptions,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OpenMergeCommand, OperationRequest};
use crate::stash::{
    self, STASH_BUNDLE_SCHEMA, StashBundle, StashBundleMember, StashDirtySummary, StashDrift,
    StashErrorDetail, StashParticipation, StashPushLifecycle, StashRestoreState, StashWarning,
};

use super::*;

mod commands;
mod projection;
mod shared;

use commands::*;
use projection::*;
use shared::*;

pub fn handle_stash<B>(
    backend: &B,
    start: &Path,
    request: crate::StashRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::StashResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Stash(request.clone()).context(operation_id.into())?;
    let (_guard, root) = if request.op == crate::StashOp::List {
        (
            None,
            resolve_workspace_root(start, request.meta.workspace.as_ref())?,
        )
    } else {
        guarded_workspace_root(
            start,
            request.meta.workspace.as_ref(),
            OpenMergeCommand::StashMutate,
            request.op == crate::StashOp::Push && request.meta.dry_run.unwrap_or(false),
        )?
    };
    if _guard.is_some() {
        assert_conf_unmodified_for(
            backend,
            &root,
            OpenMergeCommand::StashMutate,
            reconcile_authority(_guard.as_ref(), request.meta.dry_run.unwrap_or(false)),
        )?;
    }
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;

    match request.op {
        crate::StashOp::Push => handle_stash_push(backend, root, manifest, request, context),
        crate::StashOp::List => handle_stash_list(backend, root, manifest, &lock, request, context),
        crate::StashOp::Apply | crate::StashOp::Pop | crate::StashOp::Drop => {
            handle_stash_restore(backend, root, manifest, &lock, request, context)
        }
    }
}
