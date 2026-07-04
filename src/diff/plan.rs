//! D2 workspace diff planning.
//!
//! Given the manifest, the referenced GWZ snapshots, the parsed comparison, the
//! GWZ selection, and the request pathspecs, [`plan_diff`] produces the per-repo
//! [`PlannedTarget`] set that D3 executes plus the [`ExcludedTarget`] records
//! that explain candidates dropped by snapshot narrowing. It owns:
//!
//! - **Candidate selection** — the shared target resolver
//!   (`CommandDefaultTargets::All`, `RootSelectionPolicy::Allow`); root inclusion
//!   is decided here, not via a diff-only flag (AD7).
//! - **Snapshot narrowing** — when a `+snap` operand is present and selection is
//!   implicit, the candidate set narrows to materialized active Git members that
//!   every referenced snapshot covers; root and snapshotless members become typed
//!   [`ExcludedTarget`] records. Explicit `--target @root` with a snapshot operand
//!   is a typed error (D0 §7.2).
//! - **Pathspec intersection** — pathspecs narrow candidates via the shared
//!   [`route_pathspec`](crate::workspace_ops::route_pathspec) primitive; they
//!   never add back a target that selection excluded (plan §"Pathspec routing").
//! - **Root delta post-filter** — the workspace-relative prefixes a root diff must
//!   drop (member dirs, `.gwz/`, `gwz.conf/.tmp/`, AD11) travel on the root's
//!   [`PlannedTarget`] for D3 to apply after libgit2 runs.
//!
//! Output ordering is **root first, then members in manifest order** — never the
//! `BTreeMap` order stage routing uses (plan §"Pathspec routing").
//!
//! This module is pure over its inputs (manifest + snapshots + a materialization
//! predicate); the handler (D3) supplies the filesystem-backed predicate. That
//! keeps every acceptance case testable from an in-memory manifest.

use crate::artifact::{ArtifactSourceKind, ManifestArtifact, ManifestMember, SnapshotArtifact};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::protocol::generated::{
    DiffComparison, DiffExcludedTarget, DiffParsedTarget, DiffRepoScope, DiffTargetExclusionReason,
    SourceKind,
};
use crate::workspace_ops::{
    CommandDefaultTargets, RootSelectionPolicy, SelectedTarget, has_explicit_target_selection,
    join_cwd, lexical_normalize, resolve_targets, route_pathspec,
};

use super::operands::{Endpoint, ParsedComparison};
use super::{ComparisonSpec, RepoDiffComparisonKind};

/// Workspace-relative prefixes a root diff must never surface (AD11 / plan
/// §"Root and member ordering"): `.gwz/` runtime state and `gwz.conf/.tmp/`
/// temp files. Active member directories are added per-plan.
pub const ROOT_EXCLUDE_FIXED: [&str; 2] = [".gwz", "gwz.conf/.tmp"];

/// One repo the diff will actually run over: its scope, the per-repo comparison,
/// and the repo-relative pathspecs (member prefix already stripped). For the
/// root target, `root_exclude` carries the workspace-relative prefixes D3 drops
/// from the root delta list after libgit2 runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedTarget {
    pub scope: PlanScope,
    /// The per-repo comparison spec D3 hands to
    /// [`resolve_comparison`](super::resolve_comparison). Snapshot endpoints are
    /// already lowered to the recorded member commit.
    pub spec: ComparisonSpec,
    /// Repo-relative pathspecs; empty means the whole repo.
    pub pathspecs: Vec<String>,
    /// Snapshot ids that sourced the old/new side, if any (provenance carried
    /// through to `DiffParsedTarget`).
    pub left_snapshot_id: Option<String>,
    pub right_snapshot_id: Option<String>,
    /// Root-only: workspace-relative prefixes to drop from the root delta list.
    /// Empty for member targets.
    pub root_exclude: Vec<String>,
}

/// Which repo a planned/excluded target addresses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanScope {
    Root,
    Member {
        member_id: String,
        member_path: String,
        source_kind: ArtifactSourceKind,
    },
}

impl PlanScope {
    /// The wire [`DiffRepoScope`] projection (D1 to_wire bridge pattern).
    pub fn to_wire(&self) -> DiffRepoScope {
        match self {
            PlanScope::Root => DiffRepoScope {
                root: Some(true),
                member_id: None,
                member_path: None,
                source_kind: None,
            },
            PlanScope::Member {
                member_id,
                member_path,
                source_kind,
            } => DiffRepoScope {
                root: None,
                member_id: Some(member_id.clone()),
                member_path: Some(member_path.clone()),
                source_kind: Some(source_kind_to_wire(*source_kind)),
            },
        }
    }

    /// Stable target id for `DiffParsedTarget.target_id` — `@root` or the member
    /// id, matching the target-selection key vocabulary.
    fn target_id(&self) -> String {
        match self {
            PlanScope::Root => "@root".to_owned(),
            PlanScope::Member { member_id, .. } => member_id.clone(),
        }
    }
}

/// A candidate intentionally excluded before diff execution, with the typed
/// reason and the snapshot id that caused it.
#[derive(Clone, Debug, PartialEq)]
pub struct ExcludedTarget {
    pub scope: PlanScope,
    pub reason: DiffTargetExclusionReason,
    pub snapshot_id: Option<String>,
    pub message: Option<String>,
}

impl ExcludedTarget {
    pub fn to_wire(&self) -> DiffExcludedTarget {
        DiffExcludedTarget {
            scope: self.scope.to_wire(),
            reason: self.reason,
            snapshot_id: self.snapshot_id.clone(),
            message: self.message.clone(),
        }
    }
}

/// The planning result: the repos to diff (root first, then manifest order) and
/// the candidates excluded by snapshot narrowing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiffPlan {
    pub targets: Vec<PlannedTarget>,
    pub excluded: Vec<ExcludedTarget>,
}

impl DiffPlan {
    /// Project the planned targets onto the wire `DiffParsedTarget` list. The
    /// resolved oids are left `None` here — D3 fills them when it resolves the
    /// comparison per repo — matching D1's split between planning and Git access.
    pub fn parsed_targets(&self) -> Vec<DiffParsedTarget> {
        self.targets.iter().map(planned_to_wire).collect()
    }

    pub fn excluded_targets(&self) -> Vec<DiffExcludedTarget> {
        self.excluded.iter().map(ExcludedTarget::to_wire).collect()
    }
}

/// Materialization predicate: does this member have a live Git worktree? D3
/// passes a filesystem-backed check; tests pass an in-memory set. A member that
/// is inactive or non-Git is never materialized for diff purposes.
pub trait MaterializationOracle {
    fn is_materialized(&self, member: &ManifestMember) -> bool;
}

impl<F> MaterializationOracle for F
where
    F: Fn(&ManifestMember) -> bool,
{
    fn is_materialized(&self, member: &ManifestMember) -> bool {
        self(member)
    }
}

/// Plan the workspace diff (D2). See the module docs for the ordered rules.
///
/// `cwd_rel` is the workspace-relative logical cwd (`""`, `gwz-core`, …) that
/// request pathspecs resolve against (AD10). `pathspecs` are the combined
/// positional-`--`-separated pathspecs already extracted by the caller.
pub fn plan_diff(
    manifest: &ManifestArtifact,
    selection: Option<&crate::Selection>,
    comparison: &ParsedComparison,
    cwd_rel: &str,
    pathspecs: &[String],
    snapshots: &[SnapshotArtifact],
    oracle: &dyn MaterializationOracle,
) -> ModelResult<DiffPlan> {
    let explicit_selection = has_explicit_target_selection(selection);
    let has_snapshot = comparison.has_snapshot();

    // (1) Candidate set from GWZ selection (AD7): root + active members by
    // default; explicit selectors honored literally.
    let selected = resolve_targets(
        manifest,
        selection,
        CommandDefaultTargets::All,
        RootSelectionPolicy::Allow,
    )?;
    let mut include_root = selected.iter().any(|t| matches!(t, SelectedTarget::Root));
    let candidate_members: Vec<&ManifestMember> = selected
        .iter()
        .filter_map(|t| match t {
            SelectedTarget::Root => None,
            SelectedTarget::Member(member) => Some(*member),
        })
        .collect();

    let mut excluded: Vec<ExcludedTarget> = Vec::new();

    // (2) Snapshot narrowing (D0 §7.2). Snapshots do not record the root.
    if has_snapshot {
        if include_root && explicit_selection && selection_names_root(selection) {
            // Explicit `--target @root` with a snapshot operand is a typed error.
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "cannot diff @root against a snapshot: v0 snapshots do not record a workspace-root commit",
            ));
        }
        if include_root {
            // Implicit selection: drop root, report why.
            include_root = false;
            excluded.push(ExcludedTarget {
                scope: PlanScope::Root,
                reason: DiffTargetExclusionReason::RootNotInSnapshot,
                snapshot_id: comparison.snapshot_ids().first().map(|s| s.to_string()),
                message: Some("workspace root is not recorded in v0 snapshots".to_owned()),
            });
        }
    }

    // (3) Resolve each candidate member into a planned target (or an excluded /
    // errored one under snapshot narrowing), in manifest order.
    let mut member_targets: Vec<PlannedTarget> = Vec::new();
    for member in &candidate_members {
        // Non-Git members cannot be diffed by the libgit2 backend.
        if member.source_kind != ArtifactSourceKind::Git {
            if explicit_selection {
                return Err(ModelError::new(
                    ErrorCode::UnsupportedSourceKind,
                    format!(
                        "member '{}' is not a Git member; diff supports Git members only",
                        member.id
                    ),
                ));
            }
            continue;
        }

        // An active Git member with no live worktree cannot be diffed. Reached
        // implicitly (default/`@all` fan-out) it is silently skipped; explicitly
        // selected it is a member-scoped error (plan §D2).
        if !oracle.is_materialized(member) {
            if explicit_selection {
                return Err(ModelError::new(
                    ErrorCode::MemberNotFound,
                    format!("member '{}' is not materialized; cannot diff", member.id),
                ));
            }
            continue;
        }

        match resolve_member_comparison(member, comparison, snapshots, explicit_selection)? {
            MemberOutcome::Planned {
                spec,
                left_snapshot_id,
                right_snapshot_id,
            } => member_targets.push(PlannedTarget {
                scope: member_scope(member),
                spec,
                pathspecs: Vec::new(),
                left_snapshot_id,
                right_snapshot_id,
                root_exclude: Vec::new(),
            }),
            MemberOutcome::Excluded(record) => excluded.push(record),
        }
    }

    // (4) Assemble targets root-first, then members in manifest order.
    let mut targets: Vec<PlannedTarget> = Vec::new();
    if include_root {
        targets.push(PlannedTarget {
            scope: PlanScope::Root,
            spec: root_spec(comparison),
            pathspecs: Vec::new(),
            left_snapshot_id: None,
            right_snapshot_id: None,
            root_exclude: root_exclude_prefixes(manifest),
        });
    }
    targets.extend(member_targets);

    // (5) Pathspec intersection: narrow candidates and attach repo-relative
    // pathspecs. Pathspecs never add an excluded target back.
    if !pathspecs.is_empty() {
        targets = intersect_pathspecs(manifest, cwd_rel, pathspecs, targets, oracle)?;
    }

    Ok(DiffPlan { targets, excluded })
}

/// Outcome of resolving one member under the parsed comparison.
enum MemberOutcome {
    Planned {
        spec: ComparisonSpec,
        left_snapshot_id: Option<String>,
        right_snapshot_id: Option<String>,
    },
    Excluded(ExcludedTarget),
}

/// Lower the workspace comparison to this member's [`ComparisonSpec`], resolving
/// snapshot endpoints to the member's recorded commit. A member absent from a
/// referenced snapshot is an [`ExcludedTarget`] when reached implicitly, or a
/// member-scoped typed error when explicitly selected (D0 §7.2).
fn resolve_member_comparison(
    member: &ManifestMember,
    comparison: &ParsedComparison,
    snapshots: &[SnapshotArtifact],
    explicit_selection: bool,
) -> ModelResult<MemberOutcome> {
    let left = resolve_endpoint(
        member,
        comparison.left.as_ref(),
        snapshots,
        explicit_selection,
    )?;
    let left = match left {
        EndpointOutcome::Token(token) => token,
        EndpointOutcome::Excluded(record) => return Ok(MemberOutcome::Excluded(record)),
    };
    let right = resolve_endpoint(
        member,
        comparison.right.as_ref(),
        snapshots,
        explicit_selection,
    )?;
    let right = match right {
        EndpointOutcome::Token(token) => token,
        EndpointOutcome::Excluded(record) => return Ok(MemberOutcome::Excluded(record)),
    };

    let spec = ComparisonSpec {
        kind: comparison.kind,
        left: left.token,
        right: right.token,
        merge_base: comparison.merge_base,
    };
    Ok(MemberOutcome::Planned {
        spec,
        left_snapshot_id: left.snapshot_id,
        right_snapshot_id: right.snapshot_id,
    })
}

/// A resolved endpoint token plus the snapshot id it came from, if any.
struct ResolvedEndpoint {
    token: Option<String>,
    snapshot_id: Option<String>,
}

enum EndpointOutcome {
    Token(ResolvedEndpoint),
    Excluded(ExcludedTarget),
}

fn resolve_endpoint(
    member: &ManifestMember,
    endpoint: Option<&Endpoint>,
    snapshots: &[SnapshotArtifact],
    explicit_selection: bool,
) -> ModelResult<EndpointOutcome> {
    match endpoint {
        None => Ok(EndpointOutcome::Token(ResolvedEndpoint {
            token: None,
            snapshot_id: None,
        })),
        Some(Endpoint::Revision(token)) => Ok(EndpointOutcome::Token(ResolvedEndpoint {
            token: Some(token.clone()),
            snapshot_id: None,
        })),
        Some(Endpoint::Snapshot(id)) => {
            resolve_snapshot_endpoint(member, id, snapshots, explicit_selection)
        }
    }
}

/// Resolve a `+snap` endpoint to `members.<member_id>.commit`. The member must
/// appear in the snapshot with a Git source and a recorded commit.
fn resolve_snapshot_endpoint(
    member: &ManifestMember,
    snapshot_id: &str,
    snapshots: &[SnapshotArtifact],
    explicit_selection: bool,
) -> ModelResult<EndpointOutcome> {
    let snapshot = snapshots
        .iter()
        .find(|s| s.snapshot_id == snapshot_id)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::SnapshotNotFound,
                format!("snapshot '{snapshot_id}' not found"),
            )
        })?;

    match snapshot.members.get(&member.id) {
        None => exclude_or_error(
            member,
            snapshot_id,
            DiffTargetExclusionReason::SnapshotMissing,
            format!(
                "member '{}' is not recorded in snapshot '{snapshot_id}'",
                member.id
            ),
            explicit_selection,
        ),
        Some(record) if record.source_kind != ArtifactSourceKind::Git => exclude_or_error(
            member,
            snapshot_id,
            DiffTargetExclusionReason::SnapshotMissingCommit,
            format!(
                "member '{}' in snapshot '{snapshot_id}' is not a Git source",
                member.id
            ),
            explicit_selection,
        ),
        Some(record) => match &record.commit {
            Some(commit) => Ok(EndpointOutcome::Token(ResolvedEndpoint {
                token: Some(commit.clone()),
                snapshot_id: Some(snapshot_id.to_owned()),
            })),
            None => exclude_or_error(
                member,
                snapshot_id,
                DiffTargetExclusionReason::SnapshotMissingCommit,
                format!(
                    "member '{}' in snapshot '{snapshot_id}' has no recorded commit",
                    member.id
                ),
                explicit_selection,
            ),
        },
    }
}

/// A member missing from a snapshot is an [`ExcludedTarget`] when reached
/// implicitly, or a member-scoped typed error when explicitly selected.
fn exclude_or_error(
    member: &ManifestMember,
    snapshot_id: &str,
    reason: DiffTargetExclusionReason,
    message: String,
    explicit_selection: bool,
) -> ModelResult<EndpointOutcome> {
    if explicit_selection {
        return Err(ModelError::new(ErrorCode::MemberNotFound, message));
    }
    Ok(EndpointOutcome::Excluded(ExcludedTarget {
        scope: member_scope(member),
        reason,
        snapshot_id: Some(snapshot_id.to_owned()),
        message: Some(message),
    }))
}

/// The root repo's comparison spec. Snapshot endpoints never reach root (root is
/// excluded under snapshot narrowing), so a snapshot endpoint here would be a
/// bug; treat it as no token (defensive) rather than passing `+…` to Git.
fn root_spec(comparison: &ParsedComparison) -> ComparisonSpec {
    ComparisonSpec {
        kind: comparison.kind,
        left: revision_token(comparison.left.as_ref()),
        right: revision_token(comparison.right.as_ref()),
        merge_base: comparison.merge_base,
    }
}

fn revision_token(endpoint: Option<&Endpoint>) -> Option<String> {
    match endpoint {
        Some(Endpoint::Revision(token)) => Some(token.clone()),
        _ => None,
    }
}

/// Workspace-relative prefixes the root diff drops: every *active* member path,
/// plus the fixed `.gwz/` and `gwz.conf/.tmp/` (AD11). Only active members are
/// excluded — an inactive member's directory is legitimately root content.
fn root_exclude_prefixes(manifest: &ManifestArtifact) -> Vec<String> {
    let mut prefixes: Vec<String> = manifest
        .members
        .iter()
        .filter(|member| member.active)
        .map(|member| member.path.clone())
        .collect();
    prefixes.extend(ROOT_EXCLUDE_FIXED.iter().map(|s| (*s).to_owned()));
    prefixes
}

/// Intersect the request pathspecs with the planned targets (plan §"Pathspec
/// routing"): a member pathspec keeps only that member; a root-owned pathspec
/// keeps root only; a pathspec at/above a member boundary fans out to the
/// candidate members it contains. Pathspecs never add an excluded target back.
/// An empty-but-valid intersection is a clean no-diff result, not an error.
fn intersect_pathspecs(
    manifest: &ManifestArtifact,
    cwd_rel: &str,
    pathspecs: &[String],
    targets: Vec<PlannedTarget>,
    oracle: &dyn MaterializationOracle,
) -> ModelResult<Vec<PlannedTarget>> {
    let member_paths: Vec<String> = manifest.members.iter().map(|m| m.path.clone()).collect();
    // Route against a synthetic workspace root so the escape check is
    // workspace-relative (AD10), not client-absolute. A non-`/` sentinel is
    // required: `..` above the workspace top must escape *out* of the sentinel so
    // `strip_prefix` fails (a `/` root would silently clamp `/..` back to `/`).
    let ws_root = std::path::Path::new("/__gwz_ws__");
    let cwd = ws_root.join(cwd_rel);

    // Accumulate repo-relative pathspecs per candidate scope, plus whether root
    // was targeted at all.
    let mut root_specs: Vec<String> = Vec::new();
    let mut member_specs: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut root_touched = false;
    let mut members_touched: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for spec in pathspecs {
        let routed = route_pathspec(ws_root, &member_paths, &cwd, spec)?;
        match routed.member_path {
            Some(member_path) => {
                // A pathspec naming a member directly is explicit: an
                // inactive / non-Git / unmaterialized owner is a typed error
                // (plan §D2 "error on explicit unmaterialized member pathspecs").
                validate_explicit_member_pathspec(manifest, &member_path, spec, oracle)?;
                members_touched.insert(member_path.clone());
                member_specs
                    .entry(member_path)
                    .or_default()
                    .push(routed.pathspec);
            }
            None => {
                root_touched = true;
                root_specs.push(routed.pathspec.clone());
                // A root-territory directory pathspec fans out into the members
                // it contains (e.g. `.` at the workspace root reaches every
                // member). Members strictly under the pathspec get whole-repo
                // scope (`.`).
                let rel = lexical_normalize(&join_cwd(&cwd, spec));
                let rel = rel.strip_prefix(ws_root).unwrap_or(&rel);
                for member_path in &member_paths {
                    if (rel.as_os_str().is_empty()
                        || std::path::Path::new(member_path).starts_with(rel))
                        && members_touched.insert(member_path.clone())
                    {
                        member_specs
                            .entry(member_path.clone())
                            .or_default()
                            .push(".".to_owned());
                    }
                }
            }
        }
    }

    // Keep only planned targets the pathspecs touched; attach their pathspecs.
    let kept = targets
        .into_iter()
        .filter_map(|mut target| match &target.scope {
            PlanScope::Root => {
                if root_touched {
                    target.pathspecs = dedup_sorted(root_specs.clone());
                    Some(target)
                } else {
                    None
                }
            }
            PlanScope::Member { member_path, .. } => match member_specs.get(member_path) {
                Some(specs) => {
                    target.pathspecs = dedup_sorted(specs.clone());
                    Some(target)
                }
                None => None,
            },
        })
        .collect();

    Ok(kept)
}

/// A pathspec that names a member directly requires that member to be an active,
/// materialized Git repo. An inactive, non-Git, or unmaterialized owner is a
/// typed request error (plan §D2). Absent from the manifest is unreachable here
/// (routing only returns known member paths), but is defended against.
fn validate_explicit_member_pathspec(
    manifest: &ManifestArtifact,
    member_path: &str,
    spec: &str,
    oracle: &dyn MaterializationOracle,
) -> ModelResult<()> {
    let member = manifest
        .members
        .iter()
        .find(|m| m.path == member_path)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MemberNotFound,
                format!("pathspec '{spec}' targets unknown member '{member_path}'"),
            )
        })?;
    if !member.active {
        return Err(ModelError::new(
            ErrorCode::MemberInactive,
            format!("pathspec '{spec}' targets inactive member '{member_path}'"),
        ));
    }
    if member.source_kind != ArtifactSourceKind::Git {
        return Err(ModelError::new(
            ErrorCode::UnsupportedSourceKind,
            format!("pathspec '{spec}' targets non-Git member '{member_path}'"),
        ));
    }
    if !oracle.is_materialized(member) {
        return Err(ModelError::new(
            ErrorCode::MemberNotFound,
            format!("pathspec '{spec}' targets unmaterialized member '{member_path}'"),
        ));
    }
    Ok(())
}

fn dedup_sorted(mut specs: Vec<String>) -> Vec<String> {
    specs.sort();
    specs.dedup();
    // A whole-repo `.` alongside specific paths collapses to whole-repo.
    if specs.iter().any(|s| s == ".") {
        return vec![".".to_owned()];
    }
    specs
}

/// True when the selection explicitly names `@root` (as opposed to root being
/// pulled in by the default/`@all` expansion).
fn selection_names_root(selection: Option<&crate::Selection>) -> bool {
    selection.is_some_and(|s| {
        s.targets.iter().any(|t| t == "@root") || s.member_ids.iter().any(|t| t == "@root")
    })
}

fn member_scope(member: &ManifestMember) -> PlanScope {
    PlanScope::Member {
        member_id: member.id.clone(),
        member_path: member.path.clone(),
        source_kind: member.source_kind,
    }
}

fn planned_to_wire(target: &PlannedTarget) -> DiffParsedTarget {
    DiffParsedTarget {
        target_id: target.scope.target_id(),
        scope: target.scope.to_wire(),
        comparison: comparison_to_wire(&target.spec),
        pathspecs: target.pathspecs.clone(),
        // D3 resolves oids when it opens the repo; planning leaves them None.
        left_oid: None,
        right_oid: None,
        merge_base_oid: None,
        left_snapshot_id: target.left_snapshot_id.clone(),
        right_snapshot_id: target.right_snapshot_id.clone(),
    }
}

fn comparison_to_wire(spec: &ComparisonSpec) -> DiffComparison {
    DiffComparison {
        kind: spec.kind.to_wire(),
        left: spec.left.clone(),
        right: spec.right.clone(),
        merge_base: spec.merge_base.then_some(true),
    }
}

fn source_kind_to_wire(kind: ArtifactSourceKind) -> SourceKind {
    match kind {
        ArtifactSourceKind::Git => SourceKind::Git,
        ArtifactSourceKind::Archive => SourceKind::Archive,
        ArtifactSourceKind::Package => SourceKind::Package,
        ArtifactSourceKind::Local => SourceKind::Local,
        ArtifactSourceKind::Generated => SourceKind::Generated,
    }
}

impl RepoDiffComparisonKind {
    /// Whether this kind resolves an old-side commit token per repo (used by D3;
    /// exposed here so the plan projection stays cohesive).
    pub fn takes_old_commit(self) -> bool {
        matches!(
            self,
            RepoDiffComparisonKind::IndexVsTree
                | RepoDiffComparisonKind::WorktreeVsTree
                | RepoDiffComparisonKind::TreeVsTree
        )
    }
}
