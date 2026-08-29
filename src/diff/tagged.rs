//! Exact-local-tag target narrowing for `DiffRequest.tagged`.
//!
//! Pure diff planning establishes the selected, materialized, path-routed
//! candidate set first. This module performs the Git-backed second stage: keep
//! only candidates containing every requested local tag and explain omissions.

use std::path::{Path, PathBuf};

use crate::git::{Git2Backend, GitBackend};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::protocol::generated::DiffTargetExclusionReason;

use super::plan::{DiffPlan, ExcludedTarget, PlanScope};

pub(super) fn narrow_plan_to_exact_tags(
    root: &Path,
    mut plan: DiffPlan,
    tag_names: &[String],
) -> ModelResult<DiffPlan> {
    let mut found_anywhere = vec![false; tag_names.len()];
    let mut kept = Vec::new();

    for target in std::mem::take(&mut plan.targets) {
        let repo_path = repo_path_for(root, &target.scope);
        let missing = missing_exact_local_tags(&repo_path, tag_names)
            .map_err(|error| scope_error(error, &target.scope))?;
        for (index, tag) in tag_names.iter().enumerate() {
            if !missing.contains(tag) {
                found_anywhere[index] = true;
            }
        }

        if missing.is_empty() {
            kept.push(target);
        } else {
            let scope = target.scope.clone();
            plan.excluded.push(ExcludedTarget {
                scope,
                reason: DiffTargetExclusionReason::TagMissing,
                snapshot_id: None,
                message: Some(format!(
                    "{} does not contain local {} {}",
                    scope_label(&target.scope),
                    if missing.len() == 1 { "tag" } else { "tags" },
                    quote_names(&missing.iter().map(String::as_str).collect::<Vec<_>>()),
                )),
            });
        }
    }

    validate_exact_tag_narrowing(tag_names, &found_anywhere, kept.len(), "diff")?;

    plan.targets = kept;
    Ok(plan)
}

/// Exact local tags absent from one repository. Both diff and commit history
/// consume this primitive so a same-named branch never satisfies `--tagged`.
pub(crate) fn missing_exact_local_tags(
    repo_path: &Path,
    tag_names: &[String],
) -> ModelResult<Vec<String>> {
    let local_tags = Git2Backend::new().tag_list(repo_path)?;
    Ok(tag_names
        .iter()
        .filter(|tag| !local_tags.iter().any(|candidate| candidate == *tag))
        .cloned()
        .collect())
}

/// Shared aggregate checks for exact-tag narrowing. `target_name` keeps the
/// established diff wording while allowing another read engine to reuse the
/// same all-tags intersection rules.
pub(crate) fn validate_exact_tag_narrowing(
    tag_names: &[String],
    found_anywhere: &[bool],
    kept_count: usize,
    target_name: &str,
) -> ModelResult<()> {
    if let Some((missing, _)) = tag_names
        .iter()
        .zip(found_anywhere)
        .find(|(_, found)| !**found)
    {
        return Err(ModelError::new(
            ErrorCode::TagNotFound,
            format!("local tag '{missing}' was not found in any selected {target_name} target"),
        ));
    }
    if kept_count == 0 {
        let names: Vec<&str> = tag_names.iter().map(String::as_str).collect();
        return Err(ModelError::new(
            ErrorCode::TagNotFound,
            format!(
                "no selected {target_name} target contains all requested local tags {}",
                quote_names(&names)
            ),
        ));
    }
    Ok(())
}

fn repo_path_for(root: &Path, scope: &PlanScope) -> PathBuf {
    match scope {
        PlanScope::Root => root.to_path_buf(),
        PlanScope::Member { member_path, .. } => root.join(member_path),
    }
}

fn scope_error(error: ModelError, scope: &PlanScope) -> ModelError {
    match scope {
        PlanScope::Root => error.with_member("@root", "."),
        PlanScope::Member {
            member_id,
            member_path,
            ..
        } => error.with_member(member_id, member_path),
    }
}

fn scope_label(scope: &PlanScope) -> String {
    match scope {
        PlanScope::Root => "workspace root '@root' at '.'".to_owned(),
        PlanScope::Member {
            member_id,
            member_path,
            ..
        } => format!("member '{member_id}' at '{member_path}'"),
    }
}

fn quote_names(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ")
}
