use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::artifact::{ArtifactSourceKind, LockArtifact, ManifestArtifact, SnapshotArtifact};
use crate::diff::{
    Endpoint, ParsedRevisionArg, RevContext, candidate_repos, classify_operands_for_command,
    default_rev_resolver, missing_exact_local_tags, parse_revision_arg_with_snapshot_ids,
    parse_tagged_revision_args, read_referenced_snapshots, resolved_cwd_rel,
    validate_exact_tag_narrowing,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::{
    CommandDefaultTargets, RootSelectionPolicy, SelectedTarget, assert_workspace_id, join_cwd,
    lexical_normalize, owning_member, resolve_targets, resolve_workspace_root, route_pathspec,
};

use super::{
    CommitLogDegradation, CommitLogDegradationKind, CommitLogTarget, RepositoryHistory,
    RepositoryState, WalkPlan,
};

/// Request-scoped repository histories plus the S2.2 strictness parameter.
/// Later merge/output steps consume these cursors without collecting history.
pub(super) struct CommitLogHistories {
    histories: Vec<RepositoryHistory>,
    has_explicit_range: bool,
    strict: bool,
}

impl CommitLogHistories {
    pub fn histories(&self) -> &[RepositoryHistory] {
        &self.histories
    }

    pub fn has_explicit_range(&self) -> bool {
        self.has_explicit_range
    }

    /// Apply only L-TOL-2's strict overlay to an observed degradation bit.
    /// S2.6 remains responsible for the complete event-derived aggregate.
    pub fn strictness_status(&self, degradation_seen: bool) -> crate::AggregateStatus {
        if self.strict && degradation_seen {
            crate::AggregateStatus::Failed
        } else {
            crate::AggregateStatus::Ok
        }
    }

    pub fn into_histories(self) -> Vec<RepositoryHistory> {
        self.histories
    }
}

struct TargetPlan {
    target: CommitLogTarget,
    repo_path: PathBuf,
    pathspecs: Vec<String>,
    degradation: Option<CommitLogDegradation>,
}

const LOCK_ENDPOINT_ID: &str = "lock";

/// Lower one S2.0 wire request into independently streaming repository cursors.
pub(super) fn open_request_histories(
    start: &Path,
    request: &crate::LogRequest,
) -> ModelResult<CommitLogHistories> {
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = crate::artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let cwd_rel = resolved_cwd_rel(start, &root)
        .unwrap_or_else(|| request.workspace_cwd.clone().unwrap_or_default());

    let tagged = request.tagged.unwrap_or(false);
    let (revision_args, tag_names, mut pathspecs) = if tagged {
        let (args, tags) = parse_tagged_revision_args(&request.operands)?;
        (args, tags, Vec::new())
    } else {
        let classified = {
            let context = RevContext {
                repos: candidate_repos(&root, &manifest),
                cwd: root.join(&cwd_rel),
                workspace_root: root.clone(),
                resolve: &default_rev_resolver,
            };
            classify_operands_for_command(&request.operands, &manifest, &context, "gwz log")?
        };
        let snapshot_ids = if classified
            .revisions
            .iter()
            .any(|operand| operand.contains('+'))
        {
            crate::artifact::snapshot_ids_for_operand_parsing(&root)?
        } else {
            Vec::new()
        };
        (
            classified
                .revisions
                .iter()
                .map(|operand| parse_revision_arg_with_snapshot_ids(operand, &snapshot_ids))
                .collect::<ModelResult<Vec<_>>>()?,
            Vec::new(),
            classified.pathspecs,
        )
    };

    // Post-`--` values bypass classification by construction. In particular,
    // `+name` here is a literal path rather than a snapshot reference.
    pathspecs.extend(request.explicit_pathspecs.iter().cloned());
    let lock = uses_lock_endpoint(&revision_args)
        .then(|| crate::artifact::read_lock(&root))
        .transpose()?;
    if let Some(lock) = &lock
        && lock.workspace_id != manifest.workspace.id
    {
        return Err(ModelError::new(
            ErrorCode::SourceIdentityMismatch,
            "workspace manifest and lock identify different workspaces",
        ));
    }
    let snapshots =
        read_referenced_snapshots(&root, &manifest.workspace.id, &snapshot_ids(&revision_args))?;
    let selected = resolve_targets(
        &manifest,
        request.meta.selection.as_ref(),
        CommandDefaultTargets::All,
        RootSelectionPolicy::Allow,
    )?;
    let plans = validate_selected_operands(
        selected_plans(&root, selected),
        &revision_args,
        &snapshots,
        lock.as_ref(),
    );
    let plans = route_pathspecs(&root, &manifest, &cwd_rel, &pathspecs, plans)?;
    let plans = if tagged {
        narrow_to_exact_tags(plans, &tag_names)?
    } else {
        plans
    };

    let strict = request
        .options
        .as_ref()
        .and_then(|options| options.strict)
        .unwrap_or(false);
    let has_explicit_range = revision_args
        .iter()
        .any(|arg| matches!(arg, ParsedRevisionArg::Range { .. }));
    let histories = plans
        .into_iter()
        .map(|plan| open_history(plan, &revision_args, &snapshots, lock.as_ref()))
        .collect();
    Ok(CommitLogHistories {
        histories,
        has_explicit_range,
        strict,
    })
}

fn snapshot_ids(args: &[ParsedRevisionArg]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    for arg in args {
        let lock_is_pseudo = matches!(arg, ParsedRevisionArg::Range { .. });
        for endpoint in arg_endpoints(arg).into_iter().flatten() {
            if let Endpoint::Snapshot(id) = endpoint
                && !(lock_is_pseudo && id == LOCK_ENDPOINT_ID)
                && seen.insert(id.clone())
            {
                ids.push(id.clone());
            }
        }
    }
    ids
}

fn uses_lock_endpoint(args: &[ParsedRevisionArg]) -> bool {
    args.iter().any(|arg| {
        matches!(arg, ParsedRevisionArg::Range { .. })
            && arg_endpoints(arg).into_iter().flatten().any(
                |endpoint| matches!(endpoint, Endpoint::Snapshot(id) if id == LOCK_ENDPOINT_ID),
            )
    })
}

fn arg_endpoints(arg: &ParsedRevisionArg) -> [Option<&Endpoint>; 2] {
    match arg {
        ParsedRevisionArg::Endpoint(endpoint) => [Some(endpoint), None],
        ParsedRevisionArg::Range { left, right, .. } => [Some(left), Some(right)],
    }
}

fn selected_plans(root: &Path, selected: Vec<SelectedTarget<'_>>) -> Vec<TargetPlan> {
    selected
        .into_iter()
        .map(|selected| match selected {
            SelectedTarget::Root => TargetPlan {
                target: CommitLogTarget {
                    member_id: "@root".to_owned(),
                    member_path: ".".to_owned(),
                    source_kind: ArtifactSourceKind::Git,
                },
                repo_path: root.to_path_buf(),
                pathspecs: Vec::new(),
                degradation: None,
            },
            SelectedTarget::Member(member) => TargetPlan {
                target: CommitLogTarget {
                    member_id: member.id.clone(),
                    member_path: member.path.clone(),
                    source_kind: member.source_kind,
                },
                repo_path: root.join(&member.path),
                pathspecs: Vec::new(),
                degradation: None,
            },
        })
        .collect()
}

fn validate_selected_operands(
    mut plans: Vec<TargetPlan>,
    args: &[ParsedRevisionArg],
    snapshots: &[SnapshotArtifact],
    lock: Option<&LockArtifact>,
) -> Vec<TargetPlan> {
    for plan in &mut plans {
        if let Err(record) = validate_operands(&plan.target, args, snapshots, lock) {
            plan.degradation = Some(record);
        }
    }
    plans
}

fn route_pathspecs(
    root: &Path,
    manifest: &ManifestArtifact,
    cwd_rel: &str,
    pathspecs: &[String],
    plans: Vec<TargetPlan>,
) -> ModelResult<Vec<TargetPlan>> {
    if pathspecs.is_empty() {
        return Ok(plans);
    }

    let member_paths: Vec<String> = manifest
        .members
        .iter()
        .filter(|member| member.active)
        .map(|member| member.path.clone())
        .collect();
    let cwd = root.join(cwd_rel);
    let mut root_specs = Vec::new();
    let mut member_specs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut root_touched = false;
    let mut root_fanout_exclusions = Vec::new();

    for spec in pathspecs {
        let parsed = GitPathspec::parse(spec);
        let routing_cwd = if parsed.top {
            repository_root_for_cwd(root, &member_paths, &cwd)
        } else {
            cwd.clone()
        };
        let routed = route_pathspec(root, &member_paths, &routing_cwd, parsed.payload)?;
        let rewritten = parsed.with_payload(&routed.pathspec);
        if let Some(member_path) = routed.member_path {
            member_specs.entry(member_path).or_default().push(rewritten);
            continue;
        }

        root_touched = true;
        root_specs.push(rewritten);
        if parsed.exclude {
            root_fanout_exclusions.push(parsed.with_payload(parsed.payload));
            continue;
        }
        let absolute = lexical_normalize(&join_cwd(&routing_cwd, parsed.payload));
        let relative = absolute.strip_prefix(root).unwrap_or(&absolute);
        for member_path in &member_paths {
            if relative.as_os_str().is_empty() || Path::new(member_path).starts_with(relative) {
                member_specs
                    .entry(member_path.clone())
                    .or_default()
                    .push(".".to_owned());
            }
        }
    }

    for specs in member_specs.values_mut() {
        specs.extend(root_fanout_exclusions.iter().cloned());
    }

    Ok(plans
        .into_iter()
        .filter_map(|mut plan| {
            if plan.degradation.is_some() {
                return Some(plan);
            }
            let specs = if plan.target.member_id == "@root" {
                root_touched.then(|| root_specs.clone())
            } else {
                member_specs.get(&plan.target.member_path).cloned()
            }?;
            plan.pathspecs = normalize_pathspecs(specs);
            Some(plan)
        })
        .collect())
}

#[derive(Clone, Copy)]
struct GitPathspec<'a> {
    prefix: &'a str,
    payload: &'a str,
    top: bool,
    exclude: bool,
}

impl<'a> GitPathspec<'a> {
    fn parse(spec: &'a str) -> Self {
        if let Some(body) = spec.strip_prefix(":(")
            && let Some(close) = body.find(')')
        {
            let prefix_len = 2 + close + 1;
            let magic = &body[..close];
            let words = magic.split(',').collect::<Vec<_>>();
            return Self {
                prefix: &spec[..prefix_len],
                payload: &spec[prefix_len..],
                top: words.iter().any(|word| matches!(*word, "top" | "/")),
                exclude: words
                    .iter()
                    .any(|word| matches!(*word, "exclude" | "!" | "^")),
            };
        }
        for prefix in [":!", ":^"] {
            if let Some(payload) = spec.strip_prefix(prefix) {
                return Self {
                    prefix,
                    payload,
                    top: false,
                    exclude: true,
                };
            }
        }
        if let Some(payload) = spec.strip_prefix(":/") {
            return Self {
                prefix: ":/",
                payload,
                top: true,
                exclude: false,
            };
        }
        Self {
            prefix: "",
            payload: spec,
            top: false,
            exclude: false,
        }
    }

    fn with_payload(self, payload: &str) -> String {
        format!("{}{payload}", self.prefix)
    }
}

fn repository_root_for_cwd(root: &Path, member_paths: &[String], cwd: &Path) -> PathBuf {
    let relative = cwd.strip_prefix(root).unwrap_or(cwd);
    owning_member(member_paths, relative)
        .map(|member| root.join(member))
        .unwrap_or_else(|| root.to_path_buf())
}

fn normalize_pathspecs(mut pathspecs: Vec<String>) -> Vec<String> {
    pathspecs.sort();
    pathspecs.dedup();
    pathspecs
}

fn narrow_to_exact_tags(
    plans: Vec<TargetPlan>,
    tag_names: &[String],
) -> ModelResult<Vec<TargetPlan>> {
    let mut found_anywhere = vec![false; tag_names.len()];
    let mut kept_count = 0;
    let mut failures = 0;
    let narrowed = plans
        .into_iter()
        .filter_map(|mut plan| {
            if plan.target.source_kind != ArtifactSourceKind::Git {
                return None;
            }
            let missing = match missing_exact_local_tags(&plan.repo_path, tag_names) {
                Ok(missing) => missing,
                Err(error) => {
                    failures += 1;
                    plan.degradation = Some(record(
                        &plan.target,
                        CommitLogDegradationKind::RepositoryUnreadable,
                        None,
                        format!("could not inspect local tags: {}", error.message),
                    ));
                    return Some(plan);
                }
            };
            for (index, tag) in tag_names.iter().enumerate() {
                if !missing.contains(tag) {
                    found_anywhere[index] = true;
                }
            }
            if missing.is_empty() {
                kept_count += 1;
                Some(plan)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if failures == 0 || kept_count > 0 {
        validate_exact_tag_narrowing(tag_names, &found_anywhere, kept_count, "log")?;
    }
    Ok(narrowed)
}

fn open_history(
    plan: TargetPlan,
    args: &[ParsedRevisionArg],
    snapshots: &[SnapshotArtifact],
    lock: Option<&LockArtifact>,
) -> RepositoryHistory {
    let TargetPlan {
        target,
        repo_path,
        pathspecs,
        degradation,
    } = plan;
    if let Some(record) = degradation {
        return degraded_history(target, pathspecs, record);
    }
    if target.source_kind != ArtifactSourceKind::Git {
        return degraded(
            target,
            pathspecs,
            CommitLogDegradationKind::UnsupportedSourceKind,
            None,
            "commit history supports Git members only",
        );
    }
    let repository = match git2::Repository::open(&repo_path) {
        Ok(repository) => repository,
        Err(error) => {
            return degraded(
                target,
                pathspecs,
                CommitLogDegradationKind::RepositoryUnreadable,
                None,
                format!("could not open repository: {}", error.message()),
            );
        }
    };
    let walk = match resolve_walk(&repository, &target, args, snapshots, lock) {
        Ok(walk) => walk,
        Err(record) => return degraded_history(target, pathspecs, record),
    };
    RepositoryHistory {
        target,
        pathspecs,
        state: RepositoryState::Ready { repository, walk },
    }
}

fn validate_operands(
    target: &CommitLogTarget,
    args: &[ParsedRevisionArg],
    snapshots: &[SnapshotArtifact],
    lock: Option<&LockArtifact>,
) -> Result<(), CommitLogDegradation> {
    for arg in args {
        let lock_is_pseudo = matches!(arg, ParsedRevisionArg::Range { .. });
        for endpoint in arg_endpoints(arg).into_iter().flatten() {
            if let Endpoint::Snapshot(snapshot_id) = endpoint {
                if lock_is_pseudo && snapshot_id == LOCK_ENDPOINT_ID {
                    lock_commit(target, lock)?;
                } else {
                    snapshot_commit(target, snapshot_id, snapshots)?;
                }
            }
        }
    }
    Ok(())
}

fn lock_commit<'a>(
    target: &CommitLogTarget,
    lock: Option<&'a LockArtifact>,
) -> Result<&'a str, CommitLogDegradation> {
    let missing = |detail| {
        record(
            target,
            CommitLogDegradationKind::LockEntryMissing,
            Some("+lock".to_owned()),
            detail,
        )
    };
    if target.member_id == "@root" {
        return Err(missing(
            "the workspace lock does not record the workspace root".to_owned(),
        ));
    }
    let member = lock
        .and_then(|lock| lock.members.get(&target.member_id))
        .ok_or_else(|| {
            missing(format!(
                "member '{}' is not recorded in the workspace lock",
                target.member_id
            ))
        })?;
    member
        .commit
        .as_deref()
        .filter(|_| member.source_kind == ArtifactSourceKind::Git)
        .ok_or_else(|| {
            missing(format!(
                "member '{}' has no Git commit in the workspace lock",
                target.member_id
            ))
        })
}

fn snapshot_commit<'a>(
    target: &CommitLogTarget,
    snapshot_id: &str,
    snapshots: &'a [SnapshotArtifact],
) -> Result<&'a str, CommitLogDegradation> {
    let operand = format!("+{snapshot_id}");
    if target.member_id == "@root" {
        return Err(record(
            target,
            CommitLogDegradationKind::SnapshotEntryMissing,
            Some(operand),
            "snapshots do not record the workspace root",
        ));
    }
    let Some(snapshot) = snapshots
        .iter()
        .find(|snapshot| snapshot.snapshot_id == snapshot_id)
    else {
        return Err(record(
            target,
            CommitLogDegradationKind::SnapshotEntryMissing,
            Some(operand),
            format!("snapshot '{snapshot_id}' was unavailable during member resolution"),
        ));
    };
    let Some(member) = snapshot.members.get(&target.member_id) else {
        return Err(record(
            target,
            CommitLogDegradationKind::SnapshotEntryMissing,
            Some(operand),
            format!(
                "member '{}' is not recorded in snapshot '{snapshot_id}'",
                target.member_id
            ),
        ));
    };
    member
        .commit
        .as_deref()
        .filter(|_| member.source_kind == ArtifactSourceKind::Git)
        .ok_or_else(|| {
            record(
                target,
                CommitLogDegradationKind::SnapshotEntryMissing,
                Some(operand),
                format!(
                    "member '{}' has no Git commit in snapshot '{snapshot_id}'",
                    target.member_id
                ),
            )
        })
}

fn resolve_walk(
    repository: &git2::Repository,
    target: &CommitLogTarget,
    args: &[ParsedRevisionArg],
    snapshots: &[SnapshotArtifact],
    lock: Option<&LockArtifact>,
) -> Result<WalkPlan, CommitLogDegradation> {
    if args.is_empty() {
        return match repository
            .head()
            .and_then(|head| head.peel_to_commit())
            .map(|commit| commit.id())
        {
            Ok(head) => Ok(WalkPlan {
                pushes: vec![head],
                hides: Vec::new(),
            }),
            Err(error) if error.code() == git2::ErrorCode::UnbornBranch => Err(record(
                target,
                CommitLogDegradationKind::UnbornHead,
                None,
                "repository HEAD is unborn",
            )),
            Err(error) => Err(record(
                target,
                CommitLogDegradationKind::HistoryUnreadable,
                None,
                format!("could not resolve HEAD locally: {}", error.message()),
            )),
        };
    }

    let mut walk = WalkPlan::default();
    for arg in args {
        match arg {
            ParsedRevisionArg::Endpoint(endpoint) => {
                push_unique(
                    &mut walk.pushes,
                    resolve_oid(repository, target, endpoint, snapshots, lock, false)?,
                );
            }
            ParsedRevisionArg::Range {
                left,
                right,
                symmetric,
            } => {
                let left_oid = resolve_oid(repository, target, left, snapshots, lock, true)?;
                let right_oid = resolve_oid(repository, target, right, snapshots, lock, true)?;
                push_unique(&mut walk.pushes, right_oid);
                if *symmetric {
                    push_unique(&mut walk.pushes, left_oid);
                    match repository.merge_bases(left_oid, right_oid) {
                        Ok(bases) => {
                            for base in bases.iter().copied() {
                                push_unique(&mut walk.hides, base);
                            }
                        }
                        Err(error) if error.code() == git2::ErrorCode::NotFound => {}
                        Err(error) => {
                            return Err(record(
                                target,
                                CommitLogDegradationKind::HistoryUnreadable,
                                Some(format!(
                                    "{}...{}",
                                    endpoint_operand(left),
                                    endpoint_operand(right)
                                )),
                                format!("could not resolve merge bases: {}", error.message()),
                            ));
                        }
                    }
                } else {
                    push_unique(&mut walk.hides, left_oid);
                }
            }
        }
    }
    Ok(walk)
}

fn resolve_oid(
    repository: &git2::Repository,
    target: &CommitLogTarget,
    endpoint: &Endpoint,
    snapshots: &[SnapshotArtifact],
    lock: Option<&LockArtifact>,
    lock_is_pseudo: bool,
) -> Result<git2::Oid, CommitLogDegradation> {
    let operand = endpoint_operand(endpoint);
    let token = match endpoint {
        Endpoint::Revision(token) => token.as_str(),
        Endpoint::Snapshot(snapshot_id) if lock_is_pseudo && snapshot_id == LOCK_ENDPOINT_ID => {
            lock_commit(target, lock)?
        }
        Endpoint::Snapshot(snapshot_id) => snapshot_commit(target, snapshot_id, snapshots)?,
    };
    repository
        .revparse_single(token)
        .and_then(|object| object.peel_to_commit())
        .map(|commit| commit.id())
        .map_err(|error| {
            record(
                target,
                CommitLogDegradationKind::RevisionUnresolved,
                Some(operand.clone()),
                format!(
                    "revision '{}' does not resolve locally: {}",
                    operand,
                    error.message()
                ),
            )
        })
}

fn endpoint_operand(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Revision(token) => token.clone(),
        Endpoint::Snapshot(snapshot_id) => format!("+{snapshot_id}"),
    }
}

fn push_unique(oids: &mut Vec<git2::Oid>, oid: git2::Oid) {
    if !oids.contains(&oid) {
        oids.push(oid);
    }
}

fn degraded(
    target: CommitLogTarget,
    pathspecs: Vec<String>,
    kind: CommitLogDegradationKind,
    operand: Option<String>,
    detail: impl Into<String>,
) -> RepositoryHistory {
    let record = record(&target, kind, operand, detail);
    degraded_history(target, pathspecs, record)
}

fn degraded_history(
    target: CommitLogTarget,
    pathspecs: Vec<String>,
    record: CommitLogDegradation,
) -> RepositoryHistory {
    RepositoryHistory {
        target,
        pathspecs,
        state: RepositoryState::Degraded(record),
    }
}

fn record(
    target: &CommitLogTarget,
    kind: CommitLogDegradationKind,
    operand: Option<String>,
    detail: impl Into<String>,
) -> CommitLogDegradation {
    CommitLogDegradation {
        target: target.clone(),
        kind,
        operand,
        detail: detail.into(),
    }
}
