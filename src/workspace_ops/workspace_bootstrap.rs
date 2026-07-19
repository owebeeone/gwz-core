use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{ActionKind, OpenMergeCommand, OperationContext};

use super::*;

pub const AGENTS_GWZ_PATH: &str = "AGENTS_GWZ.md";

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
            Self::Created => "created AGENTS_GWZ.md",
            Self::Updated => "updated AGENTS_GWZ.md",
            Self::Unchanged => "AGENTS_GWZ.md already current",
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
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, meta.workspace.as_ref())?;
    let status = ensure_workspace_bootstrap_files(
        backend,
        &root,
        meta.dry_run.unwrap_or(false),
        force_bootstrap_overwrite(&meta),
    )?;
    let mut response = response_envelope(context, status.aggregate_status(), Vec::new());
    response.meta.message = Some(status.message().to_owned());
    Ok(response)
}

pub(crate) fn preflight_workspace_bootstrap_files(root: &Path, force: bool) -> ModelResult<()> {
    let path = root.join(AGENTS_GWZ_PATH);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(error)),
    };
    if force || contents == managed_agents_gwz_contents() || has_trusted_managed_header(&contents) {
        Ok(())
    } else {
        Err(untrusted_bootstrap_error())
    }
}

pub(crate) fn ensure_workspace_bootstrap_files<B>(
    backend: &B,
    root: &Path,
    dry_run: bool,
    force: bool,
) -> ModelResult<BootstrapUpdateStatus>
where
    B: GitBackend,
{
    preflight_workspace_bootstrap_files(root, force)?;
    let path = root.join(AGENTS_GWZ_PATH);
    let existing = match fs::read_to_string(&path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_error(error)),
    };
    let target = managed_agents_gwz_contents();
    let status = match existing.as_deref() {
        None => BootstrapUpdateStatus::Created,
        Some(contents) if contents == target => BootstrapUpdateStatus::Unchanged,
        Some(_) => BootstrapUpdateStatus::Updated,
    };

    if !dry_run {
        if status != BootstrapUpdateStatus::Unchanged {
            fs::write(&path, target).map_err(io_error)?;
        }
        if path.exists() {
            backend.stage_paths(root, &[AGENTS_GWZ_PATH])?;
        }
    }

    Ok(status)
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
    let digest = Sha256::digest(body.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn untrusted_bootstrap_error() -> ModelError {
    ModelError::new(
        ErrorCode::PermissionDenied,
        "AGENTS_GWZ.md has local edits or is missing a trusted gwz-managed-file header; rerun with --force to overwrite",
    )
}
