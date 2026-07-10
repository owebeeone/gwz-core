use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::artifact::{
    self, ArtifactSourceKind, ManifestArtifact, ManifestMember, RemoteArtifact,
    ResolvedMemberArtifact,
};
use crate::git::GitBackend;
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult, SourceId};
use crate::operation::{EventEmitter, EventSink, OperationRequest, WorkspaceMutatorLock};
use crate::workspace::MemberPath;

use super::*;

pub(crate) const EMPTY_ATTACH_EVIDENCE_WARNING_PREFIX: &str =
    "no snapshot or marker commit evidence was available to verify repository identity";

#[derive(Clone, Debug)]
pub(crate) struct PreparedAttach {
    pub(crate) member: ManifestMember,
    pub(crate) locked: ResolvedMemberArtifact,
    pub(crate) verified_commits: usize,
    pub(crate) warning: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct SingleSourcePlan {
    source: crate::SourceUrl,
    path: MemberPath,
    member_id: String,
    source_id: String,
    reused_source_members: Vec<String>,
}

pub fn handle_clone_repo_member<B>(
    backend: &B,
    start: &Path,
    request: crate::CloneRepoMemberRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::CloneRepoMemberResponse>
where
    B: GitBackend,
{
    let context =
        OperationRequest::CloneRepoMember(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    let _guard = if dry_run {
        None
    } else {
        Some(WorkspaceMutatorLock::acquire(&root)?)
    };
    let mut manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let plan = single_source_plan(&manifest, &request)?;
    let member_root = root.join(plan.path.as_str());
    ensure_member_target_available(&member_root)?;

    if dry_run {
        return Ok(crate::CloneRepoMemberResponse {
            response: response_envelope(
                context,
                crate::AggregateStatus::Accepted,
                vec![planned_member(
                    &plan.member_id,
                    plan.path.as_str(),
                    crate::PlannedAction::Clone,
                    format!("clone {}", plan.source.url),
                )],
            ),
        });
    }

    let interval = request
        .meta
        .policy
        .as_ref()
        .and_then(|policy| policy.progress_min_interval_ms)
        .unwrap_or(0);
    let emitter = EventEmitter::new(&context, events, interval);
    emitter.operation_started();
    emitter.member_started(&plan.member_id, plan.path.as_str());

    let inspected = (|| {
        backend.clone_repo_with_progress(&plan.source.url, &member_root, &|progress| {
            emitter.member_progress(&plan.member_id, plan.path.as_str(), progress)
        })?;
        let head = backend.head(&member_root)?;
        let status = backend.status(&member_root)?;
        let remotes = backend.remotes(&member_root)?;
        let (verified_commits, warning) = verify_source_identity_reuse(
            backend,
            &root,
            &member_root,
            &plan.source_id,
            &plan.reused_source_members,
        )?;
        let member = ManifestMember {
            id: plan.member_id.clone(),
            path: plan.path.as_str().to_owned(),
            source_kind: ArtifactSourceKind::Git,
            source_id: plan.source_id.clone(),
            active: true,
            desired: Some(desired_from_head(&head)),
            remotes: observed_remotes(&remotes),
        };
        let locked = resolved_member(&member, &head, &status);
        Ok::<_, ModelError>((member, locked, verified_commits, warning))
    })();

    let (member, locked, verified_commits, warning) = match inspected {
        Ok(inspected) => inspected,
        Err(error) => {
            let _ = fs::remove_dir_all(&member_root);
            emitter.operation_finished();
            return Err(error);
        }
    };

    manifest.members.push(member.clone());
    let lock = (|| {
        manifest.validate()?;
        let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
        lock.members.insert(member.id.clone(), locked.clone());
        Ok::<_, ModelError>(lock)
    })();
    let lock = match lock {
        Ok(lock) => lock,
        Err(error) => {
            let _ = fs::remove_dir_all(&member_root);
            emitter.operation_finished();
            return Err(error);
        }
    };
    if let Err(error) = artifact::write_manifest_and_lock(&root, &manifest, &lock) {
        let published = artifact::read_manifest(&root)
            .map(|current| current.members.iter().any(|item| item.id == member.id))
            .unwrap_or(false);
        if !published {
            let _ = fs::remove_dir_all(&member_root);
        }
        emitter.operation_finished();
        return Err(error);
    }
    if let Err(error) = sync_workspace_boundary(backend, &root, &manifest, &lock) {
        emitter.operation_finished();
        return Err(error);
    }
    emitter.member_finished(&member.id, &member.path);

    if let Some(message) = &warning {
        emit_warning(&emitter, &member, message);
    }
    emitter.operation_finished();
    let mut response = response_envelope(
        context,
        crate::AggregateStatus::Ok,
        vec![ok_member(&member, &locked, crate::MemberStatus::Ok)],
    );
    response.meta.message = warning.or_else(|| {
        (!plan.reused_source_members.is_empty()).then(|| {
            format!(
                "cloned {}; verified {verified_commits} historical commit(s) for source identity {}",
                member.id, member.source_id
            )
        })
    });
    Ok(crate::CloneRepoMemberResponse { response })
}

pub fn handle_detach_repo_member<B>(
    backend: &B,
    start: &Path,
    request: crate::DetachRepoMemberRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::DetachRepoMemberResponse>
where
    B: GitBackend,
{
    let context =
        OperationRequest::DetachRepoMember(request.clone()).context(operation_id.into())?;
    let selector = validate_single_detach_selector(request.meta.selection.as_ref())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    let _guard = if dry_run {
        None
    } else {
        Some(WorkspaceMutatorLock::acquire(&root)?)
    };
    let mut manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let index = resolve_detach_member_index(&manifest, &selector)?;
    let member = manifest.members[index].clone();

    if dry_run {
        return Ok(crate::DetachRepoMemberResponse {
            response: response_envelope(
                context,
                crate::AggregateStatus::Accepted,
                vec![planned_member(
                    &member.id,
                    &member.path,
                    crate::PlannedAction::DetachMember,
                    "mark member inactive and retain its checkout".to_owned(),
                )],
            ),
        });
    }

    manifest.members[index].active = false;
    manifest.validate()?;
    let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
    lock.members.remove(&member.id);
    artifact::write_manifest_and_lock(&root, &manifest, &lock)?;
    sync_workspace_boundary(backend, &root, &manifest, &lock)?;

    let mut response = response_envelope(
        context,
        crate::AggregateStatus::Ok,
        vec![crate::MemberResponse {
            member_id: member.id.clone(),
            member_path: member.path.clone(),
            source_kind: artifact_source_kind_to_protocol(member.source_kind),
            status: crate::MemberStatus::Ok,
            error: None,
            planned: None,
            state: None,
            git_status: None,
            target_kind: Some(crate::TargetKind::Member),
            lock_match: Some(crate::LockMatch::Missing),
        }],
    );
    response.meta.message = Some(format!(
        "detached {}; checkout retained at {}",
        member.id, member.path
    ));
    Ok(crate::DetachRepoMemberResponse { response })
}

pub fn handle_attach_repo_member<B>(
    backend: &B,
    start: &Path,
    request: crate::AttachRepoMemberRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::AttachRepoMemberResponse>
where
    B: GitBackend,
{
    let context =
        OperationRequest::AttachRepoMember(request.clone()).context(operation_id.into())?;
    let member_id = validate_single_attach_selector(request.meta.selection.as_ref())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    let _guard = if dry_run {
        None
    } else {
        Some(WorkspaceMutatorLock::acquire(&root)?)
    };
    let mut manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let index = manifest
        .members
        .iter()
        .position(|member| member.id == member_id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member id not found"))?;

    if manifest.members[index].active {
        let member = &manifest.members[index];
        let mut response = response_envelope(
            context,
            crate::AggregateStatus::Noop,
            vec![crate::MemberResponse {
                member_id: member.id.clone(),
                member_path: member.path.clone(),
                source_kind: artifact_source_kind_to_protocol(member.source_kind),
                status: crate::MemberStatus::Noop,
                error: None,
                planned: Some(crate::PlannedChange {
                    action: crate::PlannedAction::Noop,
                    from_ref: None,
                    to_ref: None,
                    message: Some("member is already active".to_owned()),
                }),
                state: None,
                git_status: None,
                target_kind: Some(crate::TargetKind::Member),
                lock_match: None,
            }],
        );
        response.meta.message = Some(format!("{} is already attached", member.id));
        return Ok(crate::AttachRepoMemberResponse { response });
    }

    let prepared = prepare_attach(backend, &root, &manifest, &member_id)?;
    if dry_run {
        let mut response = response_envelope(
            context,
            crate::AggregateStatus::Accepted,
            vec![planned_member(
                &prepared.member.id,
                &prepared.member.path,
                crate::PlannedAction::AttachMember,
                format!(
                    "reactivate member after verifying {} historical commit(s)",
                    prepared.verified_commits
                ),
            )],
        );
        response.meta.message = prepared.warning;
        return Ok(crate::AttachRepoMemberResponse { response });
    }

    let emitter = EventEmitter::new(&context, events, 0);
    emitter.operation_started();
    emitter.member_started(&prepared.member.id, &prepared.member.path);
    apply_prepared_attach(&mut manifest, &prepared)?;
    let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
    lock.members
        .insert(prepared.member.id.clone(), prepared.locked.clone());
    artifact::write_manifest_and_lock(&root, &manifest, &lock)?;
    if let Err(error) = sync_workspace_boundary(backend, &root, &manifest, &lock) {
        emitter.operation_finished();
        return Err(error);
    }
    if let Some(message) = &prepared.warning {
        emit_warning(&emitter, &prepared.member, message);
    }
    emitter.member_finished(&prepared.member.id, &prepared.member.path);
    emitter.operation_finished();

    let mut response = response_envelope(
        context,
        crate::AggregateStatus::Ok,
        vec![ok_member(
            &prepared.member,
            &prepared.locked,
            crate::MemberStatus::Ok,
        )],
    );
    response.meta.message = prepared.warning.or_else(|| {
        Some(format!(
            "attached {}; verified {} historical commit(s)",
            prepared.member.id, prepared.verified_commits
        ))
    });
    Ok(crate::AttachRepoMemberResponse { response })
}

pub(crate) fn prepare_attach<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    member_id: &str,
) -> ModelResult<PreparedAttach> {
    let historical = manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member id not found"))?;
    if historical.source_kind != ArtifactSourceKind::Git {
        return Err(ModelError::new(
            ErrorCode::UnsupportedSourceKind,
            "repo attach supports Git members only",
        ));
    }
    let path = MemberPath::parse(&historical.path)?;
    reject_existing_active_member_path_overlap(manifest, &path)?;
    let repo = root.join(path.as_str());
    if !backend.is_repository(&repo)? {
        return Err(ModelError::new(
            ErrorCode::MemberNotFound,
            format!(
                "checkout for {member_id} is not a Git repository; restore it at {} before attaching",
                historical.path
            ),
        ));
    }

    let evidence = historical_member_commits(root, &[member_id.to_owned()])?;
    let member_evidence = evidence.get(member_id).map(Vec::as_slice).unwrap_or(&[]);
    let verified_commits = verify_historical_identity(backend, &repo, member_evidence)?;
    let head = backend.head(&repo)?;
    let status = backend.status(&repo)?;
    let remotes = backend.remotes(&repo)?;
    let mut member = historical.clone();
    member.active = true;
    member.desired = Some(desired_from_head(&head));
    member.remotes = observed_remotes(&remotes);
    let locked = resolved_member(&member, &head, &status);
    let warning = (verified_commits == 0).then(|| empty_attach_warning(member_id));
    Ok(PreparedAttach {
        member,
        locked,
        verified_commits,
        warning,
    })
}

pub(crate) fn apply_prepared_attach(
    manifest: &mut ManifestArtifact,
    prepared: &PreparedAttach,
) -> ModelResult<()> {
    let member = manifest
        .members
        .iter_mut()
        .find(|member| member.id == prepared.member.id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member id not found"))?;
    *member = prepared.member.clone();
    manifest.validate()
}

pub(crate) fn empty_attach_warning(member_id: &str) -> String {
    format!("attached {member_id}; {EMPTY_ATTACH_EVIDENCE_WARNING_PREFIX}")
}

fn single_source_plan(
    manifest: &ManifestArtifact,
    request: &crate::CloneRepoMemberRequest,
) -> ModelResult<SingleSourcePlan> {
    if request.source.url.trim().is_empty() {
        return Err(invalid("repo clone requires a non-empty URL"));
    }
    if request.source.branch.is_some() {
        return Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "repo clone branch selection is not supported in v0",
        ));
    }
    if request
        .source
        .remote_name
        .as_ref()
        .is_some_and(|name| name != "origin")
    {
        return Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "repo clone custom remote names are not supported in v0",
        ));
    }
    let raw_path = request
        .source
        .path
        .clone()
        .unwrap_or_else(|| repo_name_from_url(&request.source.url));
    let path = MemberPath::parse(&raw_path)?;
    reject_existing_active_member_path_overlap(manifest, &path)?;
    if request.member_id.is_none()
        && manifest
            .members
            .iter()
            .any(|member| !member.active && member.path == path.as_str())
    {
        return Err(invalid(
            "member path has inactive history; pass a new --member-id or use gwz repo attach <id>",
        ));
    }
    let slug = path_slug(path.as_str())?;
    let member_id = request
        .member_id
        .clone()
        .unwrap_or_else(|| format!("mem_{slug}"));
    MemberId::parse_str(&member_id)?;
    reject_duplicate_member_id(manifest, &member_id)?;
    let default_source_id = format!(
        "src_{}",
        member_id
            .strip_prefix("mem_")
            .expect("validated member id has mem_ prefix")
    );
    let source_id = request.source_id.clone().unwrap_or(default_source_id);
    SourceId::parse_str(&source_id)?;
    let reused_source_members = manifest
        .members
        .iter()
        .filter(|member| member.source_id == source_id)
        .map(|member| member.id.clone())
        .collect::<Vec<_>>();
    if request.source_id.is_none() && !reused_source_members.is_empty() {
        return Err(invalid(format!(
            "source id {source_id} already exists; pass --source-id {source_id} to confirm reuse"
        )));
    }
    Ok(SingleSourcePlan {
        source: request.source.clone(),
        path,
        member_id,
        source_id,
        reused_source_members,
    })
}

pub(crate) fn verify_source_identity_reuse<B: GitBackend>(
    backend: &B,
    root: &Path,
    repo: &Path,
    source_id: &str,
    member_ids: &[String],
) -> ModelResult<(usize, Option<String>)> {
    if member_ids.is_empty() {
        return Ok((0, None));
    }
    let by_member = historical_member_commits(root, member_ids)?;
    let mut unique = BTreeMap::new();
    for evidence in by_member.values().flatten() {
        let entry = unique
            .entry(evidence.commit.clone())
            .or_insert_with(|| evidence.clone());
        for provenance in &evidence.provenance {
            if !entry.provenance.contains(provenance) {
                entry.provenance.push(provenance.clone());
            }
        }
    }
    let evidence = unique.into_values().collect::<Vec<_>>();
    let verified = verify_historical_identity(backend, repo, &evidence)?;
    let warning = (verified == 0).then(|| {
        format!("accepted source identity {source_id}; {EMPTY_ATTACH_EVIDENCE_WARNING_PREFIX}")
    });
    Ok((verified, warning))
}

fn validate_single_detach_selector(selection: Option<&crate::Selection>) -> ModelResult<String> {
    validate_single_literal_selector(selection, false)
}

fn validate_single_attach_selector(selection: Option<&crate::Selection>) -> ModelResult<String> {
    let selector = validate_single_literal_selector(selection, true)?;
    MemberId::parse_str(&selector)?;
    Ok(selector)
}

fn validate_single_literal_selector(
    selection: Option<&crate::Selection>,
    member_id_only: bool,
) -> ModelResult<String> {
    let selection = selection.ok_or_else(|| {
        invalid(if member_id_only {
            "repo attach requires exactly one literal member id"
        } else {
            "repo detach requires exactly one literal member id or path"
        })
    })?;
    if selection.all == Some(true) || !selection.exclude_targets.is_empty() {
        return Err(invalid(
            "repo lifecycle selectors do not support sets or exclusions",
        ));
    }
    if member_id_only && !selection.paths.is_empty() {
        return Err(invalid(
            "repo attach requires a literal member id, not a path",
        ));
    }
    let mut selectors = Vec::new();
    selectors.extend(selection.member_ids.iter().cloned());
    selectors.extend(selection.paths.iter().cloned());
    selectors.extend(selection.targets.iter().cloned());
    if selectors.len() != 1 || selectors[0].starts_with('@') {
        return Err(invalid(if member_id_only {
            "repo attach requires exactly one literal member id"
        } else {
            "repo detach requires exactly one literal member id or path"
        }));
    }
    if member_id_only && !selectors[0].starts_with("mem_") {
        return Err(invalid("repo attach requires a literal mem_... member id"));
    }
    Ok(selectors.remove(0))
}

fn resolve_detach_member_index(manifest: &ManifestArtifact, selector: &str) -> ModelResult<usize> {
    let selected = if selector.starts_with("mem_") {
        MemberId::parse_str(selector)?;
        manifest
            .members
            .iter()
            .position(|member| member.id == selector)
    } else {
        MemberPath::parse(selector)?;
        manifest
            .members
            .iter()
            .position(|member| member.active && member.path == selector)
            .or_else(|| {
                manifest
                    .members
                    .iter()
                    .position(|member| member.path == selector)
            })
    };
    let index =
        selected.ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
    if !manifest.members[index].active {
        return Err(ModelError::new(
            ErrorCode::MemberInactive,
            format!("{} is already detached", manifest.members[index].id),
        ));
    }
    Ok(index)
}

pub(crate) fn observed_remotes(remotes: &[crate::git::GitRemote]) -> Vec<RemoteArtifact> {
    remotes
        .iter()
        .map(|remote| RemoteArtifact {
            name: remote.name.clone(),
            url: remote.url.clone().unwrap_or_default(),
            fetch: true,
            push: true,
        })
        .collect()
}

pub(crate) fn planned_member(
    member_id: &str,
    member_path: &str,
    action: crate::PlannedAction,
    message: String,
) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: member_id.to_owned(),
        member_path: member_path.to_owned(),
        source_kind: crate::SourceKind::Git,
        status: crate::MemberStatus::Planned,
        error: None,
        planned: Some(crate::PlannedChange {
            action,
            from_ref: None,
            to_ref: None,
            message: Some(message),
        }),
        state: None,
        git_status: None,
        target_kind: Some(crate::TargetKind::Member),
        lock_match: None,
    }
}

pub(crate) fn ok_member(
    member: &ManifestMember,
    locked: &ResolvedMemberArtifact,
    status: crate::MemberStatus,
) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: member.path.clone(),
        source_kind: artifact_source_kind_to_protocol(member.source_kind),
        status,
        error: None,
        planned: None,
        state: Some(protocol_state(member, locked)),
        git_status: None,
        target_kind: Some(crate::TargetKind::Member),
        lock_match: Some(crate::LockMatch::Matches),
    }
}

fn emit_warning(emitter: &EventEmitter<'_>, member: &ManifestMember, message: &str) {
    emitter.emit(
        crate::EventKind::ArtifactWritten,
        crate::Severity::Warn,
        Some(member.id.clone()),
        Some(member.path.clone()),
        Some(message.to_owned()),
        None,
    );
}
