use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::artifact::{self, ManifestArtifact, ResolvedMemberArtifact};
use crate::git::{GitBackend, GitHeadState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OperationRequest, WorkspaceMutatorLock};

use super::*;

/// Fan out `git commit` across selected members and the root (root last) — the multi-repo
/// commit verb. Members with staged changes (or, with `all`, tracked modifications too)
/// are committed; a member with nothing to commit is skipped (never an empty commit). The
/// member commits update gwz's lock (new member HEADs); that lock update is then committed
/// into the root last, so the root records the post-commit composition. Members are hidden
/// via `.git/info/exclude`, not tracked. Nothing commit-able anywhere is a success no-op.
pub fn handle_commit<B>(
    backend: &B,
    start: &Path,
    request: crate::CommitRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CommitResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Commit(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let _guard = WorkspaceMutatorLock::acquire(&root)?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected_targets = resolve_targets(
        &manifest,
        request.meta.selection.as_ref(),
        CommandDefaultTargets::All,
        RootSelectionPolicy::Allow,
    )?;
    let mut selected = Vec::new();
    let mut commit_root_selected = false;
    for target in selected_targets {
        match target {
            SelectedTarget::Root => commit_root_selected = true,
            SelectedTarget::Member(member) => {
                if !lock.members.contains_key(&member.id) {
                    return Err(ModelError::new(
                        ErrorCode::LockNotFound,
                        format!("lock record missing for member '{}'", member.id),
                    ));
                }
                selected.push(member.id.clone());
            }
        }
    }
    let all = request.all.unwrap_or(false);
    let marker_enabled = request.commit_marker.unwrap_or(true);

    // Validate (non-mutating): every selected member must be materialized before any commit.
    for member_id in &selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        if !backend.is_repository(&root.join(&member.path))? {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("member '{member_id}' is not materialized; cannot commit"),
            ));
        }
    }

    let mut members_to_commit = Vec::new();
    for member_id in &selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let member_root = root.join(&member.path);
        let status = backend.status(&member_root)?;
        let has_changes = if all {
            status.staged > 0 || status.unstaged > 0
        } else {
            status.staged > 0
        };
        if has_changes {
            members_to_commit.push(member_id.clone());
        }
    }

    let root_is_repo = backend.is_repository(&root)?;
    let root_before_head = if root_is_repo {
        Some(backend.head(&root)?)
    } else {
        None
    };
    let root_has_changes = if commit_root_selected && root_is_repo {
        let root_status = backend.status(&root)?;
        if all {
            root_status.staged > 0 || root_status.unstaged > 0
        } else {
            root_status.staged > 0
        }
    } else {
        false
    };
    let will_mutate = !members_to_commit.is_empty() || root_has_changes;
    if !will_mutate {
        return Ok(crate::CommitResponse {
            response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
        });
    }

    let marker = if marker_enabled {
        if !root_is_repo {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "workspace root is not a Git repository; cannot persist commit marker",
            ));
        }
        let marker = CommitMarkerContext {
            gwz_commit_id: new_uuid_v7()?,
            workspace_id: manifest.workspace.id.clone(),
            origin_url_hash: root_origin_url_hash(backend, &root)?,
            root_before_head: root_before_head.ok_or_else(|| {
                ModelError::new(
                    ErrorCode::GitCommandFailed,
                    "workspace root has no readable Git HEAD",
                )
            })?,
        };
        preflight_marker_path(&root, &marker.gwz_commit_id)?;
        Some(marker)
    } else {
        None
    };
    let commit_message = marker
        .as_ref()
        .map(|marker| marker.commit_message(&request.message))
        .unwrap_or_else(|| request.message.clone());

    // Commit members first; skip any with nothing to commit (never an empty commit).
    let mut committed_members = Vec::new();
    for member_id in &members_to_commit {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let member_root = root.join(&member.path);
        backend.commit(&member_root, &commit_message, all)?;
        committed_members.push(member_id.clone());
    }
    let committed_member = !committed_members.is_empty();

    let mut lock_for_boundary = lock.clone();
    if committed_member {
        // Observe → re-lock from the post-commit member HEADs (the capture machinery).
        let members = observed_member_map(backend, &root, &manifest, &lock, &selected)?;
        let mut next = read_lock_or_empty(&root, &manifest.workspace.id)?;
        for (member_id, state) in &members {
            next.members.insert(member_id.clone(), state.clone());
        }
        artifact::write_lock(&root, &next)?;
        lock_for_boundary = next;
    }

    if let Some(marker) = &marker {
        let full_members =
            marker_member_map(backend, &root, &manifest, &lock_for_boundary, &selected)?;
        let mut committed_targets = committed_members.clone();
        committed_targets.push("@root".to_owned());
        artifact::write_marker(
            &root,
            &artifact::MarkerArtifact {
                schema: artifact::MARKER_SCHEMA.to_owned(),
                gwz_commit_id: marker.gwz_commit_id.clone(),
                workspace_id: marker.workspace_id.clone(),
                origin_url_hash: marker.origin_url_hash.clone(),
                created_at: now_marker(),
                created_by: created_by(&context),
                root: artifact::MarkerRootArtifact {
                    path: ".".to_owned(),
                    before_commit: marker.root_before_head.commit.clone(),
                    branch: marker.root_before_head.branch.clone(),
                },
                selected_targets: selected_marker_targets(&selected, commit_root_selected),
                committed_targets,
                members: full_members,
            },
        )?;
        // Refresh the boundary excludes + stage gwz.conf so the lock update and marker
        // land in the root commit.
        sync_workspace_boundary(backend, &root, &manifest, &lock_for_boundary)?;
    } else if committed_member {
        // Refresh the boundary excludes + stage gwz.conf so the lock update (the
        // post-commit member HEADs) lands in the root commit when the root is selected.
        sync_workspace_boundary(backend, &root, &manifest, &lock_for_boundary)?;
    }

    // Commit the root last. This covers both the lock update from member commits
    // and ordinary root-only staged changes. Marker-enabled member commits force a
    // root metadata commit so the marker is not left pending.
    if marker.is_some() || root_has_changes {
        backend.commit(&root, &commit_message, all)?;
    }

    Ok(crate::CommitResponse {
        response: response_envelope(
            context,
            crate::AggregateStatus::Ok,
            if committed_member {
                let next = artifact::read_lock(&root)?;
                locked_member_responses(&manifest, &next.members)
            } else {
                Vec::new()
            },
        ),
    })
}

#[derive(Clone, Debug)]
struct CommitMarkerContext {
    gwz_commit_id: String,
    workspace_id: String,
    origin_url_hash: Option<String>,
    root_before_head: GitHeadState,
}

impl CommitMarkerContext {
    fn commit_message(&self, message: &str) -> String {
        let mut out = message.trim_end_matches('\n').to_owned();
        out.push_str("\n\nGWZ-Commit-ID: ");
        out.push_str(&self.gwz_commit_id);
        out.push_str("\nGWZ-Workspace-ID: ");
        out.push_str(&self.workspace_id);
        if let Some(hash) = &self.origin_url_hash {
            out.push_str("\nGWZ-Origin-URL-Hash: ");
            out.push_str(hash);
        }
        out
    }
}

fn preflight_marker_path(root: &Path, gwz_commit_id: &str) -> ModelResult<()> {
    let marker_path = artifact::marker_path(root, gwz_commit_id);
    if marker_path.exists() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("commit marker '{gwz_commit_id}' already exists"),
        ));
    }
    fs::create_dir_all(root.join(artifact::MARKER_DIR)).map_err(|error| {
        ModelError::new(
            ErrorCode::IoError,
            format!("failed to create commit marker directory: {error}"),
        )
    })
}

fn root_origin_url_hash<B: GitBackend>(backend: &B, root: &Path) -> ModelResult<Option<String>> {
    let remotes = backend.remotes(root)?;
    Ok(remotes
        .into_iter()
        .find(|remote| remote.name == "origin")
        .and_then(|remote| remote.url)
        .map(|url| {
            let digest = Sha256::digest(url.trim_end_matches('\n').as_bytes());
            format!("sha256:{digest:x}")
        }))
}

fn new_uuid_v7() -> ModelResult<String> {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            ModelError::new(
                ErrorCode::InternalError,
                format!("system clock before Unix epoch: {error}"),
            )
        })?
        .as_millis();
    if timestamp_ms > 0xffff_ffff_ffff {
        return Err(ModelError::new(
            ErrorCode::InternalError,
            "current time exceeds UUIDv7 timestamp range",
        ));
    }

    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ModelError::new(
            ErrorCode::InternalError,
            format!("failed to generate commit marker UUID randomness: {error}"),
        )
    })?;
    let timestamp = timestamp_ms as u64;
    bytes[0] = (timestamp >> 40) as u8;
    bytes[1] = (timestamp >> 32) as u8;
    bytes[2] = (timestamp >> 24) as u8;
    bytes[3] = (timestamp >> 16) as u8;
    bytes[4] = (timestamp >> 8) as u8;
    bytes[5] = timestamp as u8;
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    ))
}

fn selected_marker_targets(selected_members: &[String], root_selected: bool) -> Vec<String> {
    let mut targets = Vec::with_capacity(selected_members.len() + usize::from(root_selected));
    if root_selected {
        targets.push("@root".to_owned());
    }
    targets.extend(selected_members.iter().cloned());
    targets
}

fn marker_member_map<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &artifact::LockArtifact,
    selected: &[String],
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    let selected: BTreeSet<&str> = selected.iter().map(String::as_str).collect();
    let mut members = BTreeMap::new();
    for member in &manifest.members {
        if !member.active && !selected.contains(member.id.as_str()) {
            continue;
        }
        let member_root = root.join(&member.path);
        if member_root.exists() && backend.is_repository(&member_root)? {
            let head = backend.head(&member_root)?;
            let status = backend.status(&member_root)?;
            members.insert(member.id.clone(), resolved_member(member, &head, &status));
        } else if let Some(state) = lock.members.get(&member.id) {
            let mut state = state.clone();
            state.materialized = Some(false);
            members.insert(member.id.clone(), state);
        } else {
            members.insert(
                member.id.clone(),
                ResolvedMemberArtifact {
                    path: member.path.clone(),
                    source_id: Some(member.source_id.clone()),
                    source_kind: member.source_kind,
                    materialized: Some(false),
                    ..ResolvedMemberArtifact::default()
                },
            );
        }
    }
    Ok(members)
}
