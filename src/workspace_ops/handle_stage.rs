use std::collections::BTreeSet;
use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OpenMergeCommand, OperationRequest};

use super::merge::MergeStore;
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
    if let Some(record) = merge::FileMergeStore.discover_open(&root)? {
        return handle_open_merge_stage(backend, &root, &record, &request, context);
    }
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
        // Pathspec routing is selection-blind, so an explicit `--target` must constrain the
        // routed targets: a root-territory pathspec fans out across every member and would
        // otherwise stage outside the requested scope.
        let routed = resolve_stage_targets(
            &root,
            &member_paths,
            Path::new(&request.cwd),
            &request.pathspecs,
            all,
        )?;
        if narrowed {
            SelectionScope::resolve(&manifest, request.meta.selection.as_ref())?
                .constrain(routed)?
        } else {
            routed
        }
    };
    super::merge::enforce_open_merge_stage_targets(&root, &targets)?;

    let dry_run = request.meta.dry_run.unwrap_or(false);

    // A root stage must see the current physical nested-repository boundary before
    // Git examines the worktree. Inactive checkouts remain excluded while present.
    // `.git/info/exclude` is a mutation, so a dry run never refreshes it.
    if !dry_run && targets.iter().any(|target| target.member_path.is_none()) {
        let lock = artifact::read_lock(&root)?;
        ensure_workspace_exclude(backend, &root, &manifest, &lock)?;
    }

    // Stage each target repo. An unmaterialized repo is an error if a pathspec named it
    // directly, but is skipped if it was only reached by `.` / `-A` fan-out. A dry run
    // still validates, but stages nothing.
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
        if dry_run {
            continue;
        }
        let pathspecs: Vec<&str> = target.pathspecs.iter().map(String::as_str).collect();
        backend.stage_paths(&repo_root, &pathspecs)?;
    }

    Ok(crate::StageResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
    })
}

fn handle_open_merge_stage<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &merge::MergeOperationRecord,
    request: &crate::StageRequest,
    context: crate::operation::OperationContext,
) -> ModelResult<crate::StageResponse> {
    let member_paths = merge::root::open_merge_stage_member_paths(backend, root, record)?;
    let all = request.all.unwrap_or(false);
    let narrowed = has_explicit_target_selection(request.meta.selection.as_ref());
    let targets = if all && narrowed {
        selected_open_merge_targets(record, request.meta.selection.as_ref().unwrap())?
    } else {
        // Same rule as the ordinary pathspec branch: an explicit selection constrains where
        // routed pathspecs may stage.
        let routed = resolve_stage_targets(
            root,
            &member_paths,
            Path::new(&request.cwd),
            &request.pathspecs,
            all,
        )?;
        if narrowed {
            let manifest = artifact::read_manifest(root)?;
            SelectionScope::resolve(&manifest, request.meta.selection.as_ref())?
                .constrain(routed)?
        } else {
            routed
        }
    };
    merge::enforce_open_merge_stage_targets(root, &targets)?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    for target in &targets {
        let repo_root = target
            .member_path
            .as_ref()
            .map_or_else(|| root.to_path_buf(), |path| root.join(path));
        if !backend.is_repository(&repo_root)? {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!(
                    "merge participant '{}' is not materialized",
                    target.member_path.as_deref().unwrap_or("@root")
                ),
            ));
        }
        if dry_run {
            continue;
        }
        let pathspecs = target
            .pathspecs
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        backend.stage_paths_allowing_other_conflicts(&repo_root, &pathspecs)?;
    }
    Ok(crate::StageResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
    })
}

/// The repos an explicit `--target` selection admits, in the shape routed stage targets use:
/// the workspace root plus a set of member paths.
struct SelectionScope {
    root: bool,
    member_paths: BTreeSet<String>,
    label: String,
}

impl SelectionScope {
    fn resolve(
        manifest: &artifact::ManifestArtifact,
        selection: Option<&crate::Selection>,
    ) -> ModelResult<Self> {
        let selected = resolve_targets(
            manifest,
            selection,
            CommandDefaultTargets::All,
            RootSelectionPolicy::Allow,
        )?;
        let mut root = false;
        let mut member_paths = BTreeSet::new();
        let mut labels = Vec::with_capacity(selected.len());
        for target in selected {
            match target {
                SelectedTarget::Root => {
                    root = true;
                    labels.push("@root".to_owned());
                }
                SelectedTarget::Member(member) => {
                    member_paths.insert(member.path.clone());
                    labels.push(member.id.clone());
                }
            }
        }
        Ok(Self {
            root,
            member_paths,
            label: labels.join(", "),
        })
    }

    fn admits(&self, member_path: Option<&str>) -> bool {
        member_path.map_or(self.root, |path| self.member_paths.contains(path))
    }

    /// Keep only the routed targets the selection admits. A repo a pathspec named directly
    /// (`explicit`) but the selection excludes is a hard error — the request asked to stage
    /// outside its own scope. A target reached only by `.` / `-A` fan-out is dropped, the
    /// same way an unmaterialized fan-out target is skipped.
    fn constrain(&self, targets: Vec<StageTarget>) -> ModelResult<Vec<StageTarget>> {
        let mut kept = Vec::with_capacity(targets.len());
        for target in targets {
            if self.admits(target.member_path.as_deref()) {
                kept.push(target);
                continue;
            }
            if target.explicit {
                let owner = target
                    .member_path
                    .as_deref()
                    .map_or_else(|| "@root".to_owned(), |path| format!("member '{path}'"));
                return Err(ModelError::new(
                    ErrorCode::InvalidRequest,
                    format!(
                        "pathspec routes to {owner} which is outside the selected targets [{}]",
                        self.label
                    ),
                ));
            }
        }
        Ok(kept)
    }
}

fn selected_open_merge_targets(
    record: &merge::MergeOperationRecord,
    selection: &crate::Selection,
) -> ModelResult<Vec<StageTarget>> {
    let included = selection
        .member_ids
        .iter()
        .chain(&selection.paths)
        .chain(&selection.targets)
        .collect::<Vec<_>>();
    let excluded = selection.exclude_targets.iter().collect::<Vec<_>>();
    let token_matches = |target_id: &str, token: &str| {
        matches!(token, "@all" | "@default")
            || target_id == token
            || record
                .participants
                .get(target_id)
                .is_some_and(|participant| {
                    participant.target_kind == merge::MergeTargetKind::Member
                        && participant.path == token
                })
    };
    let known = |token: &str| {
        matches!(token, "@all" | "@default")
            || record
                .selected_targets
                .iter()
                .any(|target_id| token_matches(target_id, token))
    };
    for token in included.iter().chain(&excluded) {
        if !known(token) {
            return Err(ModelError::new(
                ErrorCode::OpenOperation,
                format!(
                    "merge '{}' is open; selected add target '{}' is not a frozen merge participant",
                    record.merge_id, token
                ),
            ));
        }
    }
    let include_all = selection.all.unwrap_or(false)
        || included.is_empty()
        || included
            .iter()
            .any(|target| matches!(target.as_str(), "@all" | "@default"));
    Ok(record
        .selected_targets
        .iter()
        .filter_map(|target_id| {
            let participant = record.participants.get(target_id)?;
            let selected = include_all
                || included
                    .iter()
                    .any(|target| token_matches(target_id, target));
            let rejected = excluded
                .iter()
                .any(|target| token_matches(target_id, target));
            (selected && !rejected).then(|| StageTarget {
                member_path: match participant.target_kind {
                    merge::MergeTargetKind::Member => Some(participant.path.clone()),
                    merge::MergeTargetKind::Root => None,
                },
                pathspecs: vec![".".to_owned()],
                explicit: true,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_record() -> merge::MergeOperationRecord {
        serde_yaml::from_str(
            r#"
schema: gwz.merge-operation/v0
record_schema_version: 0
writer_version: test
workspace_id: ws_test
merge_id: merge_stage
operation_id: op_stage
state: awaiting_resolution
source_ref: feature/x
created_at: now
baseline: {lock_sha256: lock, manifest_sha256: manifest}
selected_targets: [mem_app, "@root"]
participants:
  mem_app:
    path: app
    target_kind: member
    target_branch: main
    before_commit: before
    source_commit: source
    commit_message: merge
    state: conflicted
  "@root":
    path: "."
    target_kind: root
    target_branch: main
    before_commit: before
    source_commit: source
    commit_message: merge
    state: conflicted
"#,
        )
        .unwrap()
    }

    fn selected(targets: &[&str]) -> crate::Selection {
        crate::Selection {
            targets: targets.iter().map(|target| (*target).to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn open_merge_selection_rejects_unknown_unselected_and_mixed_tokens() {
        let record = open_record();
        for targets in [
            vec!["@rot"],
            vec!["mem_not_selected"],
            vec!["@root", "mem_not_selected"],
        ] {
            let error = selected_open_merge_targets(&record, &selected(&targets)).unwrap_err();
            assert_eq!(error.code, ErrorCode::OpenOperation);
            assert!(error.message.contains(targets.last().unwrap()));
        }
    }

    #[test]
    fn open_merge_selection_keeps_frozen_order_and_supports_exclusion_only() {
        let record = open_record();
        let root = selected_open_merge_targets(&record, &selected(&["@root"])).unwrap();
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].member_path, None);

        let without_root = selected_open_merge_targets(
            &record,
            &crate::Selection {
                exclude_targets: vec!["@root".to_owned()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            without_root
                .iter()
                .map(|target| target.member_path.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("app")]
        );
    }

    #[test]
    fn open_merge_selection_expands_default_and_requires_literal_root_selector() {
        let record = open_record();
        let all = selected_open_merge_targets(&record, &selected(&["@default"])).unwrap();
        assert_eq!(
            all.iter()
                .map(|target| target.member_path.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("app"), None]
        );

        let none = selected_open_merge_targets(
            &record,
            &crate::Selection {
                exclude_targets: vec!["@default".to_owned()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(none.is_empty());

        let error = selected_open_merge_targets(&record, &selected(&["."])).unwrap_err();
        assert_eq!(error.code, ErrorCode::OpenOperation);
    }
}
