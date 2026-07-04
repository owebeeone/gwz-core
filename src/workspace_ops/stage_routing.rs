use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::pathspec_routing::{join_cwd, lexical_normalize, route_pathspec};

/// A repo to stage into plus its repo-relative pathspecs. `member_path == None` is the
/// workspace root repo; `Some(path)` is the member at `root/<path>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StageTarget {
    pub member_path: Option<String>,
    pub pathspecs: Vec<String>,
    /// `true` when a pathspec named this repo directly — an unmaterialized such target is an
    /// error. `false` for fan-out targets reached via `.` / `-A` across member boundaries —
    /// an unmaterialized fan-out target is silently skipped.
    pub explicit: bool,
}

/// Route raw `gwz add` pathspecs to the repos that own them (GWZAddPlan §2). The workspace
/// is nested repos (root + members); each path is owned by the innermost repo containing
/// it. Pathspecs are resolved cwd-relative (like `git add`), then mapped to that repo via
/// the shared [`route_pathspec`] primitive. A directory pathspec at/above member boundaries
/// fans out into each contained member, so `gwz add .` at the root spans every repo (D2).
/// `all` ignores pathspecs and targets the root repo plus every member (all fan-out). Pure —
/// no filesystem access. Targets are returned root-first then by member path; pathspecs are
/// sorted and de-duplicated.
pub(crate) fn resolve_stage_targets(
    root: &Path,
    member_paths: &[String],
    cwd: &Path,
    pathspecs: &[String],
    all: bool,
) -> ModelResult<Vec<StageTarget>> {
    // member_path (None == root) -> (repo-relative pathspecs, explicit?)
    let mut groups: BTreeMap<Option<String>, (BTreeSet<String>, bool)> = BTreeMap::new();

    if all {
        add(&mut groups, None, ".".to_owned(), false);
        for member in member_paths {
            add(&mut groups, Some(member.clone()), ".".to_owned(), false);
        }
        return Ok(into_targets(groups));
    }

    if pathspecs.is_empty() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "nothing specified to stage; pass pathspecs or --all",
        ));
    }

    for spec in pathspecs {
        let routed = route_pathspec(root, member_paths, cwd, spec)?;
        match routed.member_path {
            // The pathspec names this member directly → explicit.
            Some(member) => add(&mut groups, Some(member), routed.pathspec, true),
            None => {
                // Root-territory path: stage it in the root repo (explicit), and fan out
                // into every member contained within this pathspec (D2, fan-out — members
                // are excluded from the root, so a root-side `.` would never reach them).
                let rel = lexical_normalize(&join_cwd(cwd, spec));
                let rel = rel.strip_prefix(root).unwrap_or(&rel);
                add(&mut groups, None, routed.pathspec, true);
                for member in member_paths {
                    if rel.as_os_str().is_empty() || Path::new(member).starts_with(rel) {
                        add(&mut groups, Some(member.clone()), ".".to_owned(), false);
                    }
                }
            }
        }
    }

    Ok(into_targets(groups))
}

/// Merge a repo-relative pathspec into `groups`; a target is explicit if any contributing
/// pathspec named it directly.
fn add(
    groups: &mut BTreeMap<Option<String>, (BTreeSet<String>, bool)>,
    key: Option<String>,
    spec: String,
    explicit: bool,
) {
    let entry = groups
        .entry(key)
        .or_insert_with(|| (BTreeSet::new(), false));
    entry.0.insert(spec);
    entry.1 |= explicit;
}

fn into_targets(groups: BTreeMap<Option<String>, (BTreeSet<String>, bool)>) -> Vec<StageTarget> {
    groups
        .into_iter()
        .map(|(member_path, (pathspecs, explicit))| StageTarget {
            member_path,
            pathspecs: pathspecs.into_iter().collect(),
            explicit,
        })
        .collect()
}
