//! `gwz fetch` — contact every selected repository's configured remote, update
//! its remote-tracking refs, and report what moved.
//!
//! The verb integrates nothing (no merge, no rebase, no fast-forward, no reset)
//! and writes no workspace artifact (no lock, no manifest, no boundary sync).
//! Its only side effect is inside each repository's `refs/remotes/*`, which is
//! why it takes no workspace mutation guard and is not gated by an open merge
//! (gwz-cli dev-docs/GwzFetchPlan.md §1, D8).
//!
//! Selection is `push`'s, literally: the `ActionKind::Fetch` policy row is
//! `(All, Allow, "fetch")`, so the root is included by default (plan D1). The
//! N+1 repositories are read concurrently under the same global (`--jobs`) and
//! per-host (`--max-per-host`) ceilings `push --check-remotes` and `pull` use.
//!
//! There is no "unchanged since the last fetch" short-circuit and no
//! `--check-remotes`: contacting the remote IS the operation, so `push`'s
//! `RemoteCheck::changed` has no counterpart here (plan D4).

use std::path::{Path, PathBuf};

use crate::artifact::{self, ManifestMember};
use crate::git::{GitBackend, GitHeadState, MergeAuthorityBackend, git_host};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{
    EventEmitter, EventSink, NullSink, OperationRequest, par_map_per_host, resolve_jobs,
    resolve_per_host,
};

use super::pull_head_member_preflight::member_preflight::{
    pull_fetch_remote_name, pull_remote_host,
};
use super::*;

pub fn handle_fetch<B>(
    backend: &B,
    start: &Path,
    request: crate::FetchRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::FetchResponse>
where
    B: GitBackend + MergeAuthorityBackend + Sync,
{
    handle_fetch_with_events(backend, start, request, operation_id, &NullSink)
}

pub fn handle_fetch_with_events<B>(
    backend: &B,
    start: &Path,
    request: crate::FetchRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::FetchResponse>
where
    B: GitBackend + MergeAuthorityBackend + Sync,
{
    let start = invocation_start(start, &request.meta)?;
    let scoped_backend = backend.with_transport(&start, request.meta.transport.as_ref())?;
    let backend = scoped_backend.as_ref().unwrap_or(backend);
    handle_fetch_with_events_in(backend, &start, request, operation_id, events)
}

pub(crate) fn handle_fetch_with_events_in<B>(
    backend: &B,
    start: &Path,
    request: crate::FetchRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::FetchResponse>
where
    B: GitBackend + Sync,
{
    let context = OperationRequest::Fetch(request.clone()).context(operation_id.into())?;
    let error_context = context.clone();
    let result: ModelResult<crate::FetchResponse> = (|| {
        // No mutation guard and no conf gate: this handler writes nothing the
        // workspace owns, so a workspace mid-merge stays observable (plan D8).
        let root = resolve_request_workspace_root(start, &request.meta)?;
        let manifest = artifact::read_manifest(&root)?;
        assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
        let selected = resolve_action_targets(
            &manifest,
            request.meta.selection.as_ref(),
            crate::ActionKind::Fetch,
        )?;
        let policy = request.meta.policy.as_ref();

        let mut targets = Vec::new();
        for target in selected {
            match target {
                SelectedTarget::Root => {
                    targets.push(FetchTarget::root(backend, &root, policy)?);
                }
                SelectedTarget::Member(member) => {
                    targets.push(FetchTarget::member(backend, &root, member, policy));
                }
            }
        }

        // Authenticate every named remote before contacting any of them, as
        // push and pull do: a request that cannot reach one destination is
        // refused whole rather than leaving a half-read report.
        backend.validate_transport_remotes(
            &targets
                .iter()
                .filter_map(|target| target.remote.clone())
                .collect::<Vec<_>>(),
        )?;

        if request.meta.dry_run.unwrap_or(false) {
            let rows: Vec<_> = targets.iter().map(FetchTarget::planned_row).collect();
            return Ok(fetch_response(context, rows));
        }

        let progress_interval = policy
            .and_then(|policy| policy.progress_min_interval_ms)
            .unwrap_or(0);
        let emitter = EventEmitter::new(&context, events, progress_interval);
        emitter.operation_started();

        let jobs = resolve_jobs(policy.and_then(|policy| policy.concurrency));
        let per_host = resolve_per_host(policy.and_then(|policy| policy.max_connections_per_host));
        let rows = par_map_per_host(
            targets,
            jobs,
            per_host,
            |target| target.host.clone(),
            |target| {
                emitter.member_started(&target.member_id, &target.member_path);
                let row = fetch_one(backend, &target);
                emitter.member_finished(&target.member_id, &target.member_path);
                row
            },
        );
        emitter.operation_finished();

        Ok(fetch_response(context, rows))
    })();
    result
        .map_err(|error| super::publication::attach_transport_error(backend, error, &error_context))
        .map(|mut response| {
            super::publication::attach_transport(backend, &mut response.response);
            response
        })
}

/// One selected repository, resolved far enough to be handed to a worker.
/// Everything fallible that does NOT touch the network happens while building
/// this, so a worker's only failure mode is the fetch itself.
struct FetchTarget {
    member_id: String,
    member_path: String,
    path: PathBuf,
    source_kind: crate::SourceKind,
    target_kind: crate::TargetKind,
    /// The remote to fetch, or `None` for a repository that has no fetch
    /// remote — a reported `no_upstream` row, never an error (plan D2).
    remote: Option<String>,
    /// The branch whose tracking ref is read, or `None` for a detached or
    /// unborn HEAD, which is also `no_upstream`.
    branch: Option<String>,
    /// The local commit ahead/behind is counted from, when there is one.
    local_commit: Option<String>,
    host: Option<String>,
    /// A non-network refusal found while resolving, reported on the row
    /// instead of failing the whole request.
    refusal: Option<ModelError>,
}

impl FetchTarget {
    fn root<B>(
        backend: &B,
        root: &Path,
        policy: Option<&crate::OperationPolicy>,
    ) -> ModelResult<Self>
    where
        B: GitBackend,
    {
        let mut target = Self {
            member_id: "@root".to_owned(),
            member_path: ".".to_owned(),
            path: root.to_path_buf(),
            source_kind: crate::SourceKind::Git,
            target_kind: crate::TargetKind::Root,
            remote: None,
            branch: None,
            local_commit: None,
            host: None,
            refusal: None,
        };
        if !backend.is_repository(root)? {
            return Ok(target);
        }
        target.remote = root_fetch_remote_name(backend, root, policy)?;
        target.refusal = named_remote_refusal(backend, root, policy)?;
        if let Some(remote) = &target.remote {
            target.host = backend.remotes(root)?.iter().find_map(|candidate| {
                if &candidate.name == remote {
                    candidate.url.as_deref().and_then(git_host)
                } else {
                    None
                }
            });
        }
        let head = backend.head(root)?;
        target.apply_head(&head);
        Ok(target)
    }

    fn member<B>(
        backend: &B,
        root: &Path,
        member: &ManifestMember,
        policy: Option<&crate::OperationPolicy>,
    ) -> Self
    where
        B: GitBackend,
    {
        let source_kind = artifact_source_kind_to_protocol(member.source_kind);
        let mut target = Self {
            member_id: member.id.clone(),
            member_path: member.path.clone(),
            path: root.join(&member.path),
            source_kind,
            target_kind: crate::TargetKind::Member,
            remote: None,
            branch: None,
            local_commit: None,
            host: None,
            refusal: None,
        };
        if member.source_kind != crate::artifact::ArtifactSourceKind::Git {
            target.refusal = Some(ModelError::new(
                ErrorCode::UnsupportedSourceKind,
                "fetch supports git members only",
            ));
            return target;
        }
        // A local-only member is deliberately not contacted, exactly as pull
        // leaves it alone; it reports `no upstream` rather than an error.
        let local_only = member
            .desired
            .as_ref()
            .and_then(|desired| desired.local_only)
            == Some(true);
        if local_only {
            return target;
        }
        match backend.is_repository(&target.path) {
            Ok(true) => {}
            Ok(false) => {
                target.refusal = Some(ModelError::new(
                    ErrorCode::MemberNotFound,
                    "member is not materialized",
                ));
                return target;
            }
            Err(error) => {
                target.refusal = Some(error);
                return target;
            }
        }
        target.remote = pull_fetch_remote_name(member, policy);
        target.host = pull_remote_host(member, policy);
        match named_remote_refusal(backend, &target.path, policy) {
            Ok(refusal) => target.refusal = refusal,
            Err(error) => {
                target.refusal = Some(error);
                return target;
            }
        }
        // The branch whose tracking ref moves is the one that is checked out,
        // read locally here so a worker's only failure mode is the network.
        match backend.head(&target.path) {
            Ok(head) => target.apply_head(&head),
            Err(error) => target.refusal = Some(error),
        }
        target
    }

    /// Fill in the branch and local commit from an observed HEAD. A detached
    /// or unborn HEAD leaves both `None`: there is no tracking ref to move.
    fn apply_head(&mut self, head: &GitHeadState) {
        if head.is_detached {
            return;
        }
        self.branch = head.branch.clone();
        self.local_commit = head.commit.clone();
    }

    fn row(&self, result: crate::FetchResult) -> crate::FetchRepoSummary {
        crate::FetchRepoSummary {
            member_id: self.member_id.clone(),
            member_path: self.member_path.clone(),
            source_kind: self.source_kind,
            result,
            remote: self.remote.clone(),
            branch: self.branch.clone(),
            before: None,
            after: None,
            upstream: None,
            ahead: None,
            behind: None,
        }
    }

    /// The `--dry-run` projection: what this repository would be contacted
    /// for, with nothing contacted.
    ///
    /// A repository that has both a remote and a branch yields
    /// [`crate::FetchResult::Planned`]: `Planned` means the repository was not
    /// contacted and the row carries no result from any remote, so it is
    /// distinct from `Unchanged`, which means the repository was contacted and
    /// its tracking ref did not move. `Planned` appears only under
    /// `--dry-run`; a live fetch never produces it.
    fn planned_row(&self) -> FetchRow {
        let (result, error) = match (&self.refusal, &self.remote, &self.branch) {
            (Some(error), _, _) => (crate::FetchResult::Failed, Some(error.clone())),
            (None, Some(_), Some(_)) => (crate::FetchResult::Planned, None),
            (None, _, _) => (crate::FetchResult::NoUpstream, None),
        };
        let status = match result {
            crate::FetchResult::Failed => crate::MemberStatus::Rejected,
            crate::FetchResult::NoUpstream => crate::MemberStatus::Noop,
            _ => crate::MemberStatus::Planned,
        };
        FetchRow {
            member: self.member_response(status, error.as_ref()),
            summary: self.row(result),
        }
    }

    fn member_response(
        &self,
        status: crate::MemberStatus,
        error: Option<&ModelError>,
    ) -> crate::MemberResponse {
        crate::MemberResponse {
            member_id: self.member_id.clone(),
            member_path: self.member_path.clone(),
            source_kind: self.source_kind,
            status,
            error: error.map(|error| {
                crate::GwzError::from(
                    &error
                        .clone()
                        .with_member(&self.member_id, &self.member_path),
                )
            }),
            planned: None,
            state: None,
            git_status: None,
            target_kind: Some(self.target_kind),
            lock_match: None,
            lock_difference_reasons: None,
            url_resolution: None,
        }
    }
}

/// One repository's answer: the envelope row and the report row, produced
/// together so the two lists cannot drift out of order.
pub(crate) struct FetchRow {
    member: crate::MemberResponse,
    summary: crate::FetchRepoSummary,
}

/// Contact one repository: read the tracking ref, fetch, read it again, and
/// count. Never returns `Err` — a failure is this repository's row.
fn fetch_one<B>(backend: &B, target: &FetchTarget) -> FetchRow
where
    B: GitBackend,
{
    if let Some(error) = &target.refusal {
        return FetchRow {
            member: target.member_response(crate::MemberStatus::Rejected, Some(error)),
            summary: target.row(crate::FetchResult::Failed),
        };
    }
    let (Some(remote), Some(branch)) = (&target.remote, &target.branch) else {
        return FetchRow {
            member: target.member_response(crate::MemberStatus::Noop, None),
            summary: target.row(crate::FetchResult::NoUpstream),
        };
    };
    let upstream = format!("refs/remotes/{remote}/{branch}");
    match contact(backend, target, remote, &upstream) {
        Ok(mut summary) => {
            let status = if summary.result == crate::FetchResult::Updated {
                crate::MemberStatus::Ok
            } else {
                crate::MemberStatus::Noop
            };
            summary.upstream = Some(upstream);
            FetchRow {
                member: target.member_response(status, None),
                summary,
            }
        }
        Err(error) => FetchRow {
            member: target.member_response(crate::MemberStatus::Failed, Some(&error)),
            summary: target.row(crate::FetchResult::Failed),
        },
    }
}

/// The network half, isolated so every error it can produce lands on one row.
fn contact<B>(
    backend: &B,
    target: &FetchTarget,
    remote: &str,
    upstream: &str,
) -> ModelResult<crate::FetchRepoSummary>
where
    B: GitBackend,
{
    let before = backend.read_ref(&target.path, upstream)?;
    backend
        .fetch(&target.path, remote)
        .map_err(|error| match error.code {
            ErrorCode::MissingRemote => error,
            _ => ModelError::new(ErrorCode::RemoteRejected, error.message),
        })?;
    let after = backend.read_ref(&target.path, upstream)?;
    let result = if before == after {
        crate::FetchResult::Unchanged
    } else {
        crate::FetchResult::Updated
    };
    let mut summary = target.row(result);
    // Ahead/behind is measured AFTER the fetch, against the tracking ref this
    // operation just wrote — the fact this operation established (plan D7).
    if let (Some(local), Some(after)) = (&target.local_commit, &after) {
        let counts = backend.ahead_behind(&target.path, local, after)?;
        summary.ahead = Some(counts.ahead as i64);
        summary.behind = Some(counts.behind as i64);
    }
    summary.before = before;
    summary.after = after;
    Ok(summary)
}

fn fetch_response(
    context: crate::operation::OperationContext,
    rows: Vec<FetchRow>,
) -> crate::FetchResponse {
    let aggregate = fetch_aggregate_status(&rows);
    let members: Vec<_> = rows.iter().map(|row| row.member.clone()).collect();
    let repos: Vec<_> = rows.into_iter().map(|row| row.summary).collect();
    crate::FetchResponse {
        response: response_envelope(context, aggregate, members),
        repos: Some(repos),
    }
}

/// Push's exit-code conventions, over fetch's own facts (plan §3.5).
///
/// It is computed from the report rows rather than delegated to
/// `push_aggregate_status`, because the two verbs mean different things by a
/// `Noop` row. Push's `Noop` is "nothing to publish"; fetch's `unchanged` is a
/// repository that WAS contacted and answered. So a batch where one remote
/// refused and every other repository was read cleanly is `Partial` (exit 1,
/// the report is incomplete) even though no row is `Ok` -- not `Failed`, which
/// would claim nothing was read.
///
/// `Rejected` (exit 2) is reserved for a batch in which nothing was contacted
/// at all and every row was refused before the network -- the plan's "refused
/// before any remote was contacted". A `Planned` row counts as contacted here:
/// under `--dry-run` it stands for the repository the live run would have
/// read, so a dry run and the live run of the same selection aggregate alike.
pub(crate) fn fetch_aggregate_status(rows: &[FetchRow]) -> crate::AggregateStatus {
    let contacted = rows.iter().any(|row| {
        matches!(
            row.summary.result,
            crate::FetchResult::Updated
                | crate::FetchResult::Unchanged
                | crate::FetchResult::Planned
        )
    });
    let updated = rows
        .iter()
        .any(|row| row.summary.result == crate::FetchResult::Updated);
    let refused = rows
        .iter()
        .any(|row| row.member.status == crate::MemberStatus::Rejected);
    let failed = rows
        .iter()
        .any(|row| row.member.status == crate::MemberStatus::Failed);
    if !contacted && refused && !failed {
        crate::AggregateStatus::Rejected
    } else if contacted && (refused || failed) {
        crate::AggregateStatus::Partial
    } else if refused || failed {
        crate::AggregateStatus::Failed
    } else if updated {
        crate::AggregateStatus::Ok
    } else {
        crate::AggregateStatus::Noop
    }
}

/// The root's fetch remote: the policy `--remote` token, else `origin`, else
/// whatever remote the root has first. Identical to pull's rule.
/// A `--remote <name>` the repository does not have is refused while
/// resolving, before any network, on the live path and the dry run alike:
/// the answer is local, so a dry run that promised to contact the name would
/// be rehearsing a fetch the live run cannot make.
fn named_remote_refusal<B>(
    backend: &B,
    path: &Path,
    policy: Option<&crate::OperationPolicy>,
) -> ModelResult<Option<ModelError>>
where
    B: GitBackend,
{
    let Some(name) = policy.and_then(|policy| policy.remote.as_deref()) else {
        return Ok(None);
    };
    if backend
        .remotes(path)?
        .iter()
        .any(|remote| remote.name == name)
    {
        return Ok(None);
    }
    Ok(Some(ModelError::new(
        ErrorCode::MissingRemote,
        format!("missing remote '{name}'"),
    )))
}

fn root_fetch_remote_name<B>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
) -> ModelResult<Option<String>>
where
    B: GitBackend,
{
    if let Some(remote) = policy.and_then(|policy| policy.remote.clone()) {
        return Ok(Some(remote));
    }
    let remotes = backend.remotes(root)?;
    Ok(remotes
        .iter()
        .find(|remote| remote.name == "origin")
        .or_else(|| remotes.first())
        .map(|remote| remote.name.clone()))
}
