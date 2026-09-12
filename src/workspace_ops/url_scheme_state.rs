//! Workspace memory of the URL-scheme preference, and the request-level
//! resolution that feeds `clone` and `materialize`.
//!
//! The preference lives in `<workspace>/.gwz/url-scheme.yml`: local runtime
//! state beside the local-family index, never under `gwz.conf/`, never in the
//! manifest. Precedence, highest first: the request, this file, `manifest`.
//! Design: gwz-dev dev-docs/GwzUrlSchemePlan.md §2.1 and §2.4.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::artifact;
use crate::git::{KnownHost, UrlResolution, UrlScheme, UrlSchemeRefusal, known_host, uses_ssh};
use crate::model::{ErrorCode, ModelError, ModelResult};

/// Workspace-relative path of the recorded preference.
pub const URL_SCHEME_STATE_PATH: &str = ".gwz/url-scheme.yml";

const URL_SCHEME_STATE_SCHEMA: &str = "gwz.url-scheme/v1";

const IDENTITY_HINT: &str =
    "; for public repositories, retry with --url-scheme https or set GWZ_URL_SCHEME=https";

/// Where the effective scheme came from, as far as core can tell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UrlSchemeSource {
    /// Nothing asked: every manifest URL is used as written.
    Default,
    /// The request named a scheme (a flag or an environment variable, client-side).
    Request,
    /// The workspace's recorded preference.
    Workspace,
}

/// The scheme one operation applies, with its provenance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectiveUrlScheme {
    /// The scheme every clone URL is derived towards.
    pub scheme: UrlScheme,
    /// Where it came from.
    pub source: UrlSchemeSource,
}

impl EffectiveUrlScheme {
    /// The built-in default: manifest URLs as written.
    pub const MANIFEST: Self = Self {
        scheme: UrlScheme::Manifest,
        source: UrlSchemeSource::Default,
    };
}

#[derive(Debug, Deserialize, Serialize)]
struct UrlSchemeRecord {
    schema: String,
    scheme: String,
    recorded_by: String,
    recorded_at_ms: u64,
}

/// The scheme named by the request, if any.
pub fn requested_url_scheme(meta: &crate::RequestMeta) -> Option<UrlScheme> {
    meta.transport
        .as_ref()
        .and_then(|transport| transport.url_scheme)
        .map(UrlScheme::from)
}

/// Resolves the effective scheme: the request, then the workspace record when a
/// workspace root is known, then `manifest`.
pub fn resolve_url_scheme(
    root: Option<&Path>,
    requested: Option<UrlScheme>,
) -> ModelResult<EffectiveUrlScheme> {
    if let Some(scheme) = requested {
        return Ok(EffectiveUrlScheme {
            scheme,
            source: UrlSchemeSource::Request,
        });
    }
    if let Some(root) = root
        && let Some(scheme) = read_workspace_url_scheme(root)?
    {
        return Ok(EffectiveUrlScheme {
            scheme,
            source: UrlSchemeSource::Workspace,
        });
    }
    Ok(EffectiveUrlScheme::MANIFEST)
}

/// Reads the recorded preference; `None` when no record exists. A record that
/// cannot be understood is refused, naming the file and the remedy.
pub fn read_workspace_url_scheme(root: &Path) -> ModelResult<Option<UrlScheme>> {
    let path = root.join(URL_SCHEME_STATE_PATH);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(ModelError::new(
                ErrorCode::IoError,
                format!("cannot read {}: {error}", path.display()),
            ));
        }
    };
    let unreadable = |detail: String| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            format!(
                "workspace URL-scheme preference {} is unreadable: {detail}; delete the file, or run with --url-scheme manifest to clear it",
                path.display()
            ),
        )
    };
    let record: UrlSchemeRecord =
        serde_yaml::from_slice(&bytes).map_err(|error| unreadable(error.to_string()))?;
    if record.schema != URL_SCHEME_STATE_SCHEMA {
        return Err(unreadable(format!("unsupported schema {}", record.schema)));
    }
    let scheme = UrlScheme::parse(&record.scheme)
        .ok_or_else(|| unreadable(format!("unknown scheme {:?}", record.scheme)))?;
    if scheme == UrlScheme::Manifest {
        return Err(unreadable(
            "a recorded preference must be ssh or https".to_string(),
        ));
    }
    Ok(Some(scheme))
}

/// Records the request's preference after a successful operation: `ssh` and
/// `https` are written, an explicit `manifest` removes the record, and a
/// workspace or default source changes nothing.
pub fn record_workspace_url_scheme(
    root: &Path,
    effective: EffectiveUrlScheme,
    recorded_by: &str,
) -> ModelResult<()> {
    if effective.source != UrlSchemeSource::Request {
        return Ok(());
    }
    let path = root.join(URL_SCHEME_STATE_PATH);
    if effective.scheme == UrlScheme::Manifest {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ModelError::new(
                ErrorCode::IoError,
                format!("cannot remove {}: {error}", path.display()),
            )),
        };
    }
    let record = UrlSchemeRecord {
        schema: URL_SCHEME_STATE_SCHEMA.to_owned(),
        scheme: effective.scheme.as_str().to_owned(),
        recorded_by: recorded_by.to_owned(),
        recorded_at_ms: now_ms(),
    };
    let yaml = serde_yaml::to_string(&record).map_err(|error| {
        ModelError::new(
            ErrorCode::InternalError,
            format!("cannot encode URL-scheme preference: {error}"),
        )
    })?;
    artifact::write_atomic(
        &path,
        format!(
            "# Written by gwz: local runtime state, not committed. Clear with --url-scheme manifest.\n{yaml}"
        ),
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// The URL the workspace root is cloned from: derived like a member URL, with a
/// refusal that names the root rather than a member.
pub fn resolve_root_url(url: &str, scheme: EffectiveUrlScheme) -> ModelResult<UrlResolution> {
    crate::git::derive(url, scheme.scheme)
        .map_err(|refusal| url_scheme_refusal_error(&refusal, None))
}

/// A refusal as the typed error every caller reports.
pub(crate) fn url_scheme_refusal_error(
    refusal: &UrlSchemeRefusal,
    member: Option<(&str, &str)>,
) -> ModelError {
    let message = match member {
        Some((id, path)) => format!("member '{id}' ({path}): {refusal}"),
        None => format!("workspace root: {refusal}"),
    };
    let mut error = ModelError::new(ErrorCode::UrlSchemeUnavailable, message);
    if let Some((id, path)) = member {
        error.member_id = Some(id.to_owned());
        error.member_path = Some(path.to_owned());
    }
    error
}

/// Appends the remedy to the two SSH failures a reader without keys meets on a
/// known host, when the effective URL is an ssh form and https was not already
/// the scheme in use. Any other message is returned unchanged.
pub(crate) fn append_url_scheme_hint(message: String, url: &str, scheme: UrlScheme) -> String {
    if scheme == UrlScheme::Https || !uses_ssh(url) {
        return message;
    }
    let Some(host) = known_host(url) else {
        return message;
    };
    if message.contains("no usable identity in the ssh-agent") {
        return format!("{message}{IDENTITY_HINT}");
    }
    if message.contains("invalid or unknown remote ssh hostkey") {
        return format!(
            "{message}; run ssh -T git@{} once to record the host key, or retry with --url-scheme https",
            KnownHost::host(host)
        );
    }
    message
}

/// The wire form of one member's resolution.
pub(crate) fn protocol_url_resolution(
    resolution: &UrlResolution,
    source: UrlSchemeSource,
) -> crate::MemberUrlResolution {
    crate::MemberUrlResolution {
        manifest_url: resolution.manifest_url.clone(),
        effective_url: resolution.effective_url.clone(),
        scheme: resolution.scheme.into(),
        source: source.into(),
        derived: resolution.derived,
        host_known: resolution.host_known,
    }
}

impl From<crate::UrlScheme> for UrlScheme {
    fn from(scheme: crate::UrlScheme) -> Self {
        match scheme {
            crate::UrlScheme::Manifest => Self::Manifest,
            crate::UrlScheme::Ssh => Self::Ssh,
            crate::UrlScheme::Https => Self::Https,
        }
    }
}

impl From<UrlScheme> for crate::UrlScheme {
    fn from(scheme: UrlScheme) -> Self {
        match scheme {
            UrlScheme::Manifest => Self::Manifest,
            UrlScheme::Ssh => Self::Ssh,
            UrlScheme::Https => Self::Https,
        }
    }
}

impl From<UrlSchemeSource> for crate::UrlSchemeSource {
    fn from(source: UrlSchemeSource) -> Self {
        match source {
            UrlSchemeSource::Default => Self::Default,
            UrlSchemeSource::Request => Self::Request,
            UrlSchemeSource::Workspace => Self::Workspace,
        }
    }
}
