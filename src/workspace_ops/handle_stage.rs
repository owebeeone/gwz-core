use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OpenMergeCommand, OperationRequest};

use super::*;

/// Stage pathspecs across the repos that own them — the multi-repo `git add` verb
/// (GWZAddPlan). Pathspecs are resolved cwd-relative, routed to the innermost owning repo
/// (a member, or the workspace root) by [`resolve_stage_targets`], and staged there via
/// `stage_paths`. Local only: no lock mutation, no network. A targeted member must be
/// materialized.
pub fn handle_stage<B>(
    backend: &B,
    start: &Path,
    request: crate::StageRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::StageResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Stage(request.clone()).context(operation_id.into())?;
    let _guard = acquire_workspace_mutation_guard(
        start,
        request.meta.workspace.as_ref(),
        OpenMergeCommand::StageConflictResolution,
    )?;
    let root = _guard.root().to_path_buf();
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;

    // Only active designations own operation paths. Historical rows may overlap an
    // active owner and must not steal pathspec routing from it.
    let member_paths: Vec<String> = active_members(&manifest)
        .map(|member| member.path.clone())
        .collect();
    let all = request.all.unwrap_or(false);
    // An explicit target selection scopes `-A`; bare `-A` stages the root plus every member.
    let narrowed = has_explicit_target_selection(request.meta.selection.as_ref());

    let targets = if all && narrowed {
        let selected = resolve_targets(
            &manifest,
            request.meta.selection.as_ref(),
            CommandDefaultTargets::All,
            RootSelectionPolicy::Allow,
        )?;
        selected
            .into_iter()
            .map(|target| match target {
                SelectedTarget::Root => Ok(StageTarget {
                    member_path: None,
                    pathspecs: vec![".".to_owned()],
                    explicit: true,
                }),
                SelectedTarget::Member(member) => Ok(StageTarget {
                    member_path: Some(member.path.clone()),
                    pathspecs: vec![".".to_owned()],
                    explicit: true,
                }),
            })
            .collect::<ModelResult<Vec<_>>>()?
    } else {
        resolve_stage_targets(
            &root,
            &member_paths,
            Path::new(&request.cwd),
            &request.pathspecs,
            all,
        )?
    };
    super::merge::enforce_open_merge_stage_targets(&root, &targets)?;

    // A root stage must see the current physical nested-repository boundary before
    // Git examines the worktree. Inactive checkouts remain excluded while present.
    if targets.iter().any(|target| target.member_path.is_none()) {
        let lock = artifact::read_lock(&root)?;
        ensure_workspace_exclude(backend, &root, &manifest, &lock)?;
    }

    // Stage each target repo. An unmaterialized repo is an error if a pathspec named it
    // directly, but is skipped if it was only reached by `.` / `-A` fan-out.
    for target in &targets {
        let repo_root = match &target.member_path {
            Some(path) => root.join(path),
            None => root.clone(),
        };
        if !backend.is_repository(&repo_root)? {
            if target.explicit {
                return Err(ModelError::new(
                    ErrorCode::MemberNotFound,
                    format!(
                        "member '{}' is not materialized; cannot stage",
                        target.member_path.as_deref().unwrap_or("<root>")
                    ),
                ));
            }
            continue;
        }
        let pathspecs: Vec<&str> = target.pathspecs.iter().map(String::as_str).collect();
        backend.stage_paths(&repo_root, &pathspecs)?;
    }

    Ok(crate::StageResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
    })
}
