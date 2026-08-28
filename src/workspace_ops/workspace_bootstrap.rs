use std::fs;
use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{ActionKind, OpenMergeCommand, OperationContext};

use super::*;
use claude_settings::{CLAUDE_SETTINGS_PATH, ensure_claude_settings};
pub(crate) use conf_gate::{assert_conf_unmodified_for, reconcile_authority};

mod claude_settings;
mod conf_gate;

pub const AGENTS_GWZ_PATH: &str = "AGENTS_GWZ.md";
pub const AGENTS_PATH: &str = "AGENTS.md";
pub const AGENTS_GWZ_REFERENCE: &str =
    "Read and follow `AGENTS_GWZ.md` before doing any work in this workspace.\n";

const AGENTS_GWZ_TEMPLATE_BODY: &str = include_str!("agents_gwz_template.md");
const MANAGED_HEADER_PREFIX: &str = "<!-- gwz-managed-file: sha256=";
const MANAGED_HEADER_SUFFIX: &str = " -->";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BootstrapUpdateStatus {
    Created,
    Updated,
    Unchanged,
}

impl BootstrapUpdateStatus {
    fn aggregate_status(self) -> crate::AggregateStatus {
        match self {
            Self::Created | Self::Updated => crate::AggregateStatus::Ok,
            Self::Unchanged => crate::AggregateStatus::Noop,
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Created => "created workspace agent bootstrap files",
            Self::Updated => "updated workspace agent bootstrap files",
            Self::Unchanged => "workspace agent bootstrap files already current",
        }
    }
}

pub fn handle_update_workspace_bootstrap<B>(
    backend: &B,
    start: &Path,
    meta: crate::RequestMeta,
    operation_id: impl Into<String>,
) -> ModelResult<crate::ResponseEnvelope>
where
    B: GitBackend,
{
    let context =
        OperationContext::from_meta(operation_id.into(), ActionKind::InitFromSources, &meta)?;
    let (_guard, root) = guarded_workspace_root(
        start,
        meta.workspace.as_ref(),
        OpenMergeCommand::InitUpdate,
        meta.dry_run.unwrap_or(false),
    )?;
    let dry_run = meta.dry_run.unwrap_or(false);
    let force = force_bootstrap_overwrite(&meta);
    // `--force` is the single sanctioned way to accept a hand-edited gwz.conf: it reads
    // past the integrity gate and records the current on-disk state as the new baseline.
    //
    // NOTE: this is the same `--force` that authorizes overwriting a locally edited
    // AGENTS_GWZ.md, so forcing for that reason also accepts any conf drift. A distinct
    // flag would separate the two; that is an operator decision, not this lane's.
    let accepted_conf_state =
        force && !dry_run && artifact::inspect_conf_integrity(&root).refuses();
    if !force {
        assert_conf_unmodified_for(
            backend,
            &root,
            OpenMergeCommand::InitUpdate,
            reconcile_authority(_guard.as_ref(), dry_run),
        )?;
    }
    let manifest = artifact::read_manifest(&root)?;
    if force {
        // Read BOTH documents before blessing them, so `--force` can never enshrine bytes
        // gwz cannot parse. The lock is read only when it exists: a workspace mid-init, or
        // one whose lock is legitimately absent, must still be acceptable.
        if root.join(artifact::LOCK_PATH).exists() {
            artifact::read_lock(&root)?;
        }
        if !dry_run {
            artifact::refresh_conf_integrity_marker(&root)?;
        }
    }
    assert_workspace_id(&manifest, meta.workspace.as_ref())?;
    let mut outcome = ensure_workspace_bootstrap_files(backend, &root, dry_run, force)?;
    if accepted_conf_state {
        outcome.notes.push(format!(
            "accepted the current on-disk {} state as the gwz-written baseline",
            crate::workspace::WORKSPACE_DIR
        ));
    }
    let mut response = response_envelope(context, outcome.status.aggregate_status(), Vec::new());
    response.meta.message = Some(outcome.message());
    Ok(response)
}

pub(crate) fn preflight_workspace_bootstrap_files(root: &Path, force: bool) -> ModelResult<()> {
    if let Some(contents) = read_optional_text(&root.join(AGENTS_GWZ_PATH))?
        && !(force
            || contents == managed_agents_gwz_contents()
            || has_trusted_managed_header(&contents))
    {
        return Err(untrusted_bootstrap_error());
    }
    read_optional_text(&root.join(AGENTS_PATH)).map(|_| ())
}

fn agents_with_gwz_reference(existing: Option<&str>) -> Option<String> {
    let reference = AGENTS_GWZ_REFERENCE.trim_end();
    if existing.is_some_and(|contents| contents.lines().any(|line| line.trim() == reference)) {
        return None;
    }

    let mut target = existing.unwrap_or_default().to_owned();
    if !target.is_empty() {
        if !target.ends_with('\n') {
            target.push('\n');
        }
        if !target.ends_with("\n\n") {
            target.push('\n');
        }
    }
    target.push_str(AGENTS_GWZ_REFERENCE);
    Some(target)
}

fn read_optional_text(path: &Path) -> ModelResult<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}

fn combine_bootstrap_status(
    agents_gwz_status: BootstrapUpdateStatus,
    agents_reference_changed: bool,
) -> BootstrapUpdateStatus {
    if agents_gwz_status == BootstrapUpdateStatus::Created {
        BootstrapUpdateStatus::Created
    } else if agents_gwz_status == BootstrapUpdateStatus::Updated || agents_reference_changed {
        BootstrapUpdateStatus::Updated
    } else {
        BootstrapUpdateStatus::Unchanged
    }
}

/// What the bootstrap sweep did, plus anything the caller should say out loud. The
/// `.claude/settings.json` merge can decline to touch a file it cannot parse, and an
/// unreadable conf-integrity marker is reported too; neither may fail the run.
pub(crate) struct BootstrapOutcome {
    pub(crate) status: BootstrapUpdateStatus,
    pub(crate) notes: Vec<String>,
}

impl BootstrapOutcome {
    pub(crate) fn message(&self) -> String {
        let mut message = self.status.message().to_owned();
        for note in &self.notes {
            message.push_str("; ");
            message.push_str(note);
        }
        message
    }
}

pub(crate) fn ensure_workspace_bootstrap_files<B>(
    backend: &B,
    root: &Path,
    dry_run: bool,
    force: bool,
) -> ModelResult<BootstrapOutcome>
where
    B: GitBackend,
{
    preflight_workspace_bootstrap_files(root, force)?;
    let path = root.join(AGENTS_GWZ_PATH);
    let existing = read_optional_text(&path)?;
    let target = managed_agents_gwz_contents();
    let agents_gwz_status = match existing.as_deref() {
        None => BootstrapUpdateStatus::Created,
        Some(contents) if contents == target => BootstrapUpdateStatus::Unchanged,
        Some(_) => BootstrapUpdateStatus::Updated,
    };
    let agents_path = root.join(AGENTS_PATH);
    let agents_target = agents_with_gwz_reference(read_optional_text(&agents_path)?.as_deref());
    let settings = ensure_claude_settings(root, dry_run)?;
    let status = combine_bootstrap_status(
        agents_gwz_status,
        agents_target.is_some() || settings.changed(),
    );
    let mut notes = Vec::new();
    notes.extend(settings.warning());

    if !dry_run {
        if agents_gwz_status != BootstrapUpdateStatus::Unchanged {
            fs::write(&path, target).map_err(io_error)?;
        }
        if path.exists() {
            backend.stage_paths(root, &[AGENTS_GWZ_PATH])?;
        }
        if let Some(agents_target) = agents_target {
            fs::write(&agents_path, agents_target).map_err(io_error)?;
            backend.stage_paths(root, &[AGENTS_PATH])?;
        }
        // Stage it only when this run wrote it. A pre-existing file gwz declined to
        // touch — unparseable, or the wrong shape — stays exactly as the operator left
        // it, including deliberately untracked.
        if settings.changed() && root.join(CLAUDE_SETTINGS_PATH).exists() {
            backend.stage_paths(root, &[CLAUDE_SETTINGS_PATH])?;
        }
        notes.extend(adopt_conf_integrity(backend, root)?);
    }

    Ok(BootstrapOutcome { status, notes })
}

/// Grandfathering adoption point.
///
/// A workspace with no marker — every workspace created before this existed — is enrolled
/// here rather than on load: this is a write command that is already touching managed
/// files, whereas making read-only commands (`gwz status`, `gwz ls`, `gwz diff`) or a dry
/// run mutate the tree would dirty a workspace nobody asked to change. A workspace that
/// already has a marker is never re-blessed here; that is `--force`'s job alone.
fn adopt_conf_integrity<B>(backend: &B, root: &Path) -> ModelResult<Option<String>>
where
    B: GitBackend,
{
    let note = match artifact::inspect_conf_integrity(root) {
        artifact::ConfIntegrityVerdict::NotEnrolled => {
            artifact::refresh_conf_integrity_marker(root)?;
            None
        }
        verdict @ artifact::ConfIntegrityVerdict::MarkerUnreadable(_) => verdict.warning(),
        artifact::ConfIntegrityVerdict::Verified | artifact::ConfIntegrityVerdict::Mismatch(_) => {
            None
        }
    };
    // Stage whatever marker is now on disk — the freshly adopted one, or the one
    // `--force` just re-blessed. The marker only works if git moves it in the same
    // commit as the files it vouches for; staging an unchanged file is a no-op.
    if root.join(artifact::CONF_INTEGRITY_MARKER_PATH).exists() {
        backend.stage_paths(root, &[artifact::CONF_INTEGRITY_MARKER_PATH])?;
    }
    Ok(note)
}

pub(crate) fn force_bootstrap_overwrite(meta: &crate::RequestMeta) -> bool {
    meta.policy.as_ref().and_then(|policy| policy.destructive)
        == Some(crate::DestructiveBehavior::Allow)
}

pub(crate) fn managed_agents_gwz_contents() -> String {
    managed_agents_gwz_contents_for_body(AGENTS_GWZ_TEMPLATE_BODY)
}

pub(crate) fn managed_agents_gwz_contents_for_body(body: &str) -> String {
    format!(
        "{MANAGED_HEADER_PREFIX}{}{MANAGED_HEADER_SUFFIX}\n\n{body}",
        sha256_hex(body)
    )
}

fn has_trusted_managed_header(contents: &str) -> bool {
    let (header, mut body) = contents.split_once('\n').unwrap_or((contents, ""));
    let header = header.trim_end_matches('\r');
    if let Some(rest) = body.strip_prefix("\r\n") {
        body = rest;
    } else if let Some(rest) = body.strip_prefix('\n') {
        body = rest;
    }
    let Some(digest) = digest_from_header(header) else {
        return false;
    };
    sha256_hex(body).eq_ignore_ascii_case(digest)
}

fn digest_from_header(header: &str) -> Option<&str> {
    let digest = header
        .strip_prefix(MANAGED_HEADER_PREFIX)?
        .strip_suffix(MANAGED_HEADER_SUFFIX)?;
    if digest.len() == 64 && digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(digest)
    } else {
        None
    }
}

fn sha256_hex(body: &str) -> String {
    artifact::sha256_hex(body.as_bytes())
}

fn untrusted_bootstrap_error() -> ModelError {
    ModelError::new(
        ErrorCode::PermissionDenied,
        "AGENTS_GWZ.md has local edits or is missing a trusted gwz-managed-file header; rerun with --force to overwrite",
    )
}
