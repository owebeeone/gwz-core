//! D2 workspace diff planning acceptance cases (GwzDiffPlan.md §D2).
//!
//! Each named `#[test]` maps to one acceptance bullet in the plan. The planner
//! is pure over an in-memory manifest + snapshots + a materialization oracle, so
//! these cases exercise selection, snapshot narrowing, pathspec intersection,
//! root-first ordering, and the root delta post-filter without touching Git.

use std::collections::BTreeMap;

use crate::artifact::{
    ArtifactSourceKind, CreatedByArtifact, ManifestArtifact, ManifestMember,
    ResolvedMemberArtifact, SNAPSHOT_SCHEMA, SnapshotArtifact, WORKSPACE_SCHEMA, WorkspaceHeader,
};
use crate::diff::{
    Endpoint, ParsedComparison, PlanScope, RepoDiffComparisonKind, parse_comparison, plan_diff,
};
use crate::model::ErrorCode;
use crate::protocol::generated::DiffTargetExclusionReason;

// ---------- fixtures ----------

fn member(id: &str, path: &str, active: bool) -> ManifestMember {
    ManifestMember {
        id: id.to_owned(),
        path: path.to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: format!("src_{}", id.trim_start_matches("mem_")),
        active,
        desired: None,
        remotes: Vec::new(),
    }
}

/// Root + two active Git members `gwz-core` and `gwz-cli`.
fn manifest() -> ManifestArtifact {
    ManifestArtifact {
        schema: WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: "ws_test".to_owned(),
        },
        members: vec![
            member("mem_core", "gwz-core", true),
            member("mem_cli", "gwz-cli", true),
        ],
    }
}

/// A snapshot recording `member_ids`, each with a fixed commit oid.
fn snapshot(id: &str, members: &[(&str, &str, &str)]) -> SnapshotArtifact {
    let mut recorded: BTreeMap<String, ResolvedMemberArtifact> = BTreeMap::new();
    for (member_id, path, commit) in members {
        recorded.insert(
            (*member_id).to_owned(),
            ResolvedMemberArtifact {
                path: (*path).to_owned(),
                source_id: Some(format!("src_{}", member_id.trim_start_matches("mem_"))),
                source_kind: ArtifactSourceKind::Git,
                commit: Some((*commit).to_owned()),
                branch: None,
                detached: None,
                upstream: None,
                dirty: None,
                materialized: Some(true),
            },
        );
    }
    SnapshotArtifact {
        schema: SNAPSHOT_SCHEMA.to_owned(),
        workspace_id: "ws_test".to_owned(),
        snapshot_id: id.to_owned(),
        created_at: "2026-06-15T00:00:00Z".to_owned(),
        created_by: CreatedByArtifact {
            actor_id: "agent_test".to_owned(),
        },
        selected_members: members.iter().map(|(id, _, _)| (*id).to_owned()).collect(),
        members: recorded,
    }
}

fn selection(all: bool, targets: &[&str], exclude: &[&str]) -> crate::Selection {
    crate::Selection {
        all: all.then_some(true),
        member_ids: Vec::new(),
        paths: Vec::new(),
        targets: targets.iter().map(|t| (*t).to_owned()).collect(),
        exclude_targets: exclude.iter().map(|t| (*t).to_owned()).collect(),
    }
}

fn plain() -> ParsedComparison {
    parse_comparison(&[], false, false).unwrap()
}

/// Every member is materialized.
fn all_materialized(_: &ManifestMember) -> bool {
    true
}

/// The scope target ids of the planned targets, in plan order.
fn target_ids(plan: &crate::diff::DiffPlan) -> Vec<String> {
    plan.targets
        .iter()
        .map(|t| match &t.scope {
            PlanScope::Root => "@root".to_owned(),
            PlanScope::Member { member_id, .. } => member_id.clone(),
        })
        .collect()
}

// ---------- acceptance cases ----------

/// "No pathspec selects root plus all materialized active members."
#[test]
fn no_pathspec_selects_root_and_all_members() {
    let m = manifest();
    let plan = plan_diff(&m, None, &plain(), "", &[], &[], &all_materialized).unwrap();
    assert_eq!(target_ids(&plan), vec!["@root", "mem_core", "mem_cli"]);
    assert!(plan.excluded.is_empty());
    // Root-first ordering: the first target is the root.
    assert!(matches!(plan.targets[0].scope, PlanScope::Root));
}

/// "`--target @root` selects only the root repository."
#[test]
fn target_root_selects_only_root() {
    let m = manifest();
    let sel = selection(false, &["@root"], &[]);
    let plan = plan_diff(&m, Some(&sel), &plain(), "", &[], &[], &all_materialized).unwrap();
    assert_eq!(target_ids(&plan), vec!["@root"]);
}

/// "`--all --no-target @root` selects all materialized active members and
/// excludes root."
#[test]
fn all_minus_root_selects_members_only() {
    let m = manifest();
    let sel = selection(true, &[], &["@root"]);
    let plan = plan_diff(&m, Some(&sel), &plain(), "", &[], &[], &all_materialized).unwrap();
    assert_eq!(target_ids(&plan), vec!["mem_core", "mem_cli"]);
}

/// "`gwz diff +snap` omits root, diffs only members recorded in `snap`, and
/// reports root plus any members missing from the snapshot in
/// `excluded_targets`."
#[test]
fn snapshot_omits_root_and_reports_excluded() {
    let m = manifest();
    // Snapshot records only mem_core; mem_cli was added after capture.
    let snap = snapshot("start", &[("mem_core", "gwz-core", "aaaa1111")]);
    let comparison = parse_comparison(&["+start".to_owned()], false, false).unwrap();
    let plan = plan_diff(
        &m,
        None,
        &comparison,
        "",
        &[],
        std::slice::from_ref(&snap),
        &all_materialized,
    )
    .unwrap();

    // Only the snapshotted member is diffed.
    assert_eq!(target_ids(&plan), vec!["mem_core"]);

    // The snapshot commit was lowered onto the member's comparison old side.
    let core = &plan.targets[0];
    assert_eq!(core.spec.kind, RepoDiffComparisonKind::WorktreeVsTree);
    assert_eq!(core.spec.left.as_deref(), Some("aaaa1111"));
    assert_eq!(core.left_snapshot_id.as_deref(), Some("start"));

    // Root is excluded with root_not_in_snapshot; mem_cli with snapshot_missing.
    let mut excluded: Vec<(String, DiffTargetExclusionReason)> = plan
        .excluded
        .iter()
        .map(|e| {
            let key = match &e.scope {
                PlanScope::Root => "@root".to_owned(),
                PlanScope::Member { member_id, .. } => member_id.clone(),
            };
            (key, e.reason)
        })
        .collect();
    excluded.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        excluded,
        vec![
            (
                "@root".to_owned(),
                DiffTargetExclusionReason::RootNotInSnapshot
            ),
            (
                "mem_cli".to_owned(),
                DiffTargetExclusionReason::SnapshotMissing
            ),
        ]
    );
}

/// "`gwz --target @root diff +snap` is a typed error because v0 snapshots do not
/// record a root commit."
#[test]
fn root_target_with_snapshot_is_error() {
    let m = manifest();
    let snap = snapshot("start", &[("mem_core", "gwz-core", "aaaa1111")]);
    let sel = selection(false, &["@root"], &[]);
    let comparison = parse_comparison(&["+start".to_owned()], false, false).unwrap();
    let err = plan_diff(
        &m,
        Some(&sel),
        &comparison,
        "",
        &[],
        std::slice::from_ref(&snap),
        &all_materialized,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
}

/// "`gwz diff -- gwz-core/src/lib.rs` targets only `gwz-core` with `src/lib.rs`."
#[test]
fn member_pathspec_targets_only_that_member() {
    let m = manifest();
    let pathspecs = vec!["gwz-core/src/lib.rs".to_owned()];
    let plan = plan_diff(&m, None, &plain(), "", &pathspecs, &[], &all_materialized).unwrap();
    assert_eq!(target_ids(&plan), vec!["mem_core"]);
    assert_eq!(plan.targets[0].pathspecs, vec!["src/lib.rs".to_owned()]);
}

/// "`gwz diff -- gwz.conf/gwz.yml` targets only root."
#[test]
fn root_pathspec_targets_only_root() {
    let m = manifest();
    let pathspecs = vec!["gwz.conf/gwz.yml".to_owned()];
    let plan = plan_diff(&m, None, &plain(), "", &pathspecs, &[], &all_materialized).unwrap();
    assert_eq!(target_ids(&plan), vec!["@root"]);
    assert_eq!(
        plan.targets[0].pathspecs,
        vec!["gwz.conf/gwz.yml".to_owned()]
    );
}

/// "`gwz diff -- .` at workspace root targets root plus members."
#[test]
fn dot_at_root_targets_root_and_members() {
    let m = manifest();
    let pathspecs = vec![".".to_owned()];
    let plan = plan_diff(&m, None, &plain(), "", &pathspecs, &[], &all_materialized).unwrap();
    assert_eq!(target_ids(&plan), vec!["@root", "mem_core", "mem_cli"]);
    // Each repo gets whole-repo scope.
    for target in &plan.targets {
        assert_eq!(target.pathspecs, vec![".".to_owned()]);
    }
}

/// "Parent-relative pathspecs from subdirectories behave like Git."
///
/// From `gwz-core/src`, `../../gwz-cli/a.rs` routes to `gwz-cli` with `a.rs`.
#[test]
fn parent_relative_pathspec_from_subdir() {
    let m = manifest();
    let pathspecs = vec!["../../gwz-cli/a.rs".to_owned()];
    let plan = plan_diff(
        &m,
        None,
        &plain(),
        "gwz-core/src",
        &pathspecs,
        &[],
        &all_materialized,
    )
    .unwrap();
    assert_eq!(target_ids(&plan), vec!["mem_cli"]);
    assert_eq!(plan.targets[0].pathspecs, vec!["a.rs".to_owned()]);
}

/// "Explicit member `A` plus pathspec `B/file` returns a clean empty result when
/// both names are valid but non-overlapping."
#[test]
fn member_a_with_pathspec_b_is_empty() {
    let m = manifest();
    // Select only mem_core, but pathspec into gwz-cli territory.
    let sel = crate::Selection {
        all: None,
        member_ids: vec!["mem_core".to_owned()],
        paths: Vec::new(),
        targets: Vec::new(),
        exclude_targets: Vec::new(),
    };
    let pathspecs = vec!["gwz-cli/file.rs".to_owned()];
    let plan = plan_diff(
        &m,
        Some(&sel),
        &plain(),
        "",
        &pathspecs,
        &[],
        &all_materialized,
    )
    .unwrap();
    // Non-overlapping: the pathspec cannot add gwz-cli (excluded by selection)
    // and does not touch gwz-core → clean empty, not an error.
    assert!(plan.targets.is_empty(), "expected empty intersection");
    assert!(plan.excluded.is_empty());
}

/// "Root diff output does not include active member paths, `.gwz/`, or
/// `gwz.conf/.tmp/` even when `.git/info/exclude` is stale."
///
/// The planner attaches these workspace-relative prefixes to the root target's
/// `root_exclude`; D3 drops any root delta under them after libgit2 runs.
#[test]
fn root_exclude_prefixes_cover_members_and_runtime() {
    let m = manifest();
    let plan = plan_diff(&m, None, &plain(), "", &[], &[], &all_materialized).unwrap();
    let root = plan
        .targets
        .iter()
        .find(|t| matches!(t.scope, PlanScope::Root))
        .expect("root target present");
    // Active member dirs + the fixed runtime/temp prefixes.
    assert!(root.root_exclude.contains(&"gwz-core".to_owned()));
    assert!(root.root_exclude.contains(&"gwz-cli".to_owned()));
    assert!(root.root_exclude.contains(&".gwz".to_owned()));
    assert!(root.root_exclude.contains(&"gwz.conf/.tmp".to_owned()));
}

// ---------- supporting behavior beyond the bulleted list ----------

/// An explicitly-selected member missing from a referenced snapshot is a
/// member-scoped typed error (not a silent exclusion), honoring D0 §7.2.
#[test]
fn explicit_member_missing_from_snapshot_errors() {
    let m = manifest();
    // Snapshot lacks mem_cli, which we select explicitly.
    let snap = snapshot("start", &[("mem_core", "gwz-core", "aaaa1111")]);
    let sel = crate::Selection {
        all: None,
        member_ids: vec!["mem_cli".to_owned()],
        paths: Vec::new(),
        targets: Vec::new(),
        exclude_targets: Vec::new(),
    };
    let comparison = parse_comparison(&["+start".to_owned()], false, false).unwrap();
    let err = plan_diff(
        &m,
        Some(&sel),
        &comparison,
        "",
        &[],
        std::slice::from_ref(&snap),
        &all_materialized,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::MemberNotFound);
}

/// `--cached` lowers to index-vs-tree with a `None` (HEAD/empty) old side.
#[test]
fn cached_lowers_to_index_vs_tree() {
    let comparison = parse_comparison(&[], true, false).unwrap();
    assert_eq!(comparison.kind, RepoDiffComparisonKind::IndexVsTree);
    assert!(comparison.left.is_none());
}

/// `A...B` lowers to tree-vs-tree with `merge_base`; endpoints are parsed as two
/// operands even when both are snapshots.
#[test]
fn three_dot_range_lowers_to_merge_base() {
    let comparison = parse_comparison(&["+base...+tip".to_owned()], false, false).unwrap();
    assert_eq!(comparison.kind, RepoDiffComparisonKind::TreeVsTree);
    assert!(comparison.merge_base);
    assert_eq!(comparison.left, Some(Endpoint::Snapshot("base".to_owned())));
    assert_eq!(comparison.right, Some(Endpoint::Snapshot("tip".to_owned())));
}

/// An unmaterialized member reached implicitly (default selection) is silently
/// skipped from the diff, not errored.
#[test]
fn unmaterialized_member_skipped_when_implicit() {
    let m = manifest();
    // mem_cli has no live worktree.
    let oracle = |member: &ManifestMember| member.id != "mem_cli";
    let plan = plan_diff(&m, None, &plain(), "", &[], &[], &oracle).unwrap();
    assert_eq!(target_ids(&plan), vec!["@root", "mem_core"]);
}

/// A pathspec that names an unmaterialized member directly is a typed error.
#[test]
fn explicit_pathspec_into_unmaterialized_member_errors() {
    let m = manifest();
    let oracle = |member: &ManifestMember| member.id != "mem_cli";
    let pathspecs = vec!["gwz-cli/a.rs".to_owned()];
    let err = plan_diff(&m, None, &plain(), "", &pathspecs, &[], &oracle).unwrap_err();
    assert_eq!(err.code, ErrorCode::MemberNotFound);
}

/// A pathspec that names an inactive member directly is a typed error.
#[test]
fn explicit_pathspec_into_inactive_member_errors() {
    let mut m = manifest();
    m.members.push(member("mem_old", "legacy", false));
    let pathspecs = vec!["legacy/a.rs".to_owned()];
    let err = plan_diff(&m, None, &plain(), "", &pathspecs, &[], &all_materialized).unwrap_err();
    assert_eq!(err.code, ErrorCode::MemberInactive);
}

/// The wire projection (D1 to_wire bridge pattern) carries scope, comparison,
/// pathspecs, and snapshot provenance; oids stay `None` (D3 resolves them).
#[test]
fn wire_projection_matches_plan() {
    let m = manifest();
    let snap = snapshot("start", &[("mem_core", "gwz-core", "aaaa1111")]);
    let comparison = parse_comparison(&["+start".to_owned()], false, false).unwrap();
    let plan = plan_diff(
        &m,
        None,
        &comparison,
        "",
        &[],
        std::slice::from_ref(&snap),
        &all_materialized,
    )
    .unwrap();

    let parsed = plan.parsed_targets();
    assert_eq!(parsed.len(), 1);
    let core = &parsed[0];
    assert_eq!(core.target_id, "mem_core");
    assert_eq!(core.scope.member_id.as_deref(), Some("mem_core"));
    assert_eq!(core.scope.member_path.as_deref(), Some("gwz-core"));
    assert_eq!(core.comparison.left.as_deref(), Some("aaaa1111"));
    assert_eq!(core.left_snapshot_id.as_deref(), Some("start"));
    // Planning leaves oids unresolved — D3's Git access fills them.
    assert!(core.left_oid.is_none());
    assert!(core.right_oid.is_none());

    let excluded = plan.excluded_targets();
    // Root + mem_cli excluded → two wire records.
    assert_eq!(excluded.len(), 2);
    assert!(
        excluded.iter().any(|e| e.scope.root == Some(true)
            && e.reason == DiffTargetExclusionReason::RootNotInSnapshot)
    );
    assert!(
        excluded
            .iter()
            .any(|e| e.scope.member_id.as_deref() == Some("mem_cli")
                && e.reason == DiffTargetExclusionReason::SnapshotMissing)
    );
}

/// A pathspec outside the workspace is a path-escape error (routed through the
/// shared primitive).
#[test]
fn pathspec_outside_workspace_is_escape() {
    let m = manifest();
    let pathspecs = vec!["../outside.rs".to_owned()];
    let err = plan_diff(&m, None, &plain(), "", &pathspecs, &[], &all_materialized).unwrap_err();
    assert_eq!(err.code, ErrorCode::PathEscape);
}
