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
    let backend = Git2Backend::new();
    let mut found_anywhere = vec![false; tag_names.len()];
    let mut kept = Vec::new();

    for target in std::mem::take(&mut plan.targets) {
        let repo_path = repo_path_for(root, &target.scope);
        let local_tags = backend
            .tag_list(&repo_path)
            .map_err(|error| scope_error(error, &target.scope))?;
        let mut missing = Vec::new();
        for (index, tag) in tag_names.iter().enumerate() {
            if local_tags.iter().any(|candidate| candidate == tag) {
                found_anywhere[index] = true;
            } else {
                missing.push(tag.as_str());
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
                    quote_names(&missing),
                )),
            });
        }
    }

    if let Some((_, missing)) = tag_names
        .iter()
        .zip(&found_anywhere)
        .find(|(_, found)| !**found)
    {
        return Err(ModelError::new(
            ErrorCode::TagNotFound,
            format!("local tag '{missing}' was not found in any selected diff target"),
        ));
    }
    if kept.is_empty() {
        let names: Vec<&str> = tag_names.iter().map(String::as_str).collect();
        return Err(ModelError::new(
            ErrorCode::TagNotFound,
            format!(
                "no selected diff target contains all requested local tags {}",
                quote_names(&names)
            ),
        ));
    }

    plan.targets = kept;
    Ok(plan)
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
