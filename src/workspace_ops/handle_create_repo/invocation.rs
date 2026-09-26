use std::path::{Path, PathBuf};

use crate::model::ModelResult;
use crate::workspace::discover_workspace_root;

use super::super::*;

pub fn resolve_workspace_root(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
) -> ModelResult<PathBuf> {
    let caller_start = normalize_absolute_path(start, "invocation caller_cwd")?;
    resolve_workspace_root_from_caller(&caller_start, workspace, false)
}

/// Resolve the caller directory recorded with a request.
///
/// Serialized requests carry their caller context explicitly, so a receiver
/// never borrows meaning from its own process directory. Every production entry
/// point takes its `start` from [`caller_directory`], so the two always agree.
/// The absent-context arm serves only direct library callers, such as tests:
/// their supplied `start` is the same explicit context and must already be
/// absolute.
pub fn invocation_start(start: &Path, meta: &crate::RequestMeta) -> ModelResult<PathBuf> {
    let supplied = meta
        .invocation
        .as_ref()
        .map(|context| Path::new(&context.caller_cwd))
        .unwrap_or(start);
    if meta.invocation.is_some()
        && meta
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.root.as_ref())
            .is_some_and(|root| !Path::new(root).is_absolute())
    {
        return Err(invalid("workspace root must be an absolute path"));
    }
    normalize_absolute_path(supplied, "invocation caller_cwd")
}

/// The caller's directory, taken only from the request.
///
/// A receiver has no directory of its own to fall back to: its process working
/// directory is never consulted, so a request that carries `RequestMeta`
/// without an invocation context is refused.
pub fn caller_directory(meta: &crate::RequestMeta) -> ModelResult<PathBuf> {
    let Some(context) = meta.invocation.as_ref() else {
        return Err(invalid("request carries no invocation context (caller_cwd)"));
    };
    normalize_absolute_path(Path::new(&context.caller_cwd), "invocation caller_cwd")
}

/// A path as request text, exactly or not at all.
///
/// Request path fields are Unicode text. A path that is not valid Unicode is
/// refused, with its invalid bytes escaped, instead of being sent altered.
pub fn path_text(path: &Path, label: &str) -> ModelResult<String> {
    match path.to_str() {
        Some(text) => Ok(text.to_owned()),
        None => Err(invalid(format!("{label} is not valid Unicode: {path:?}"))),
    }
}

/// Normalize one absolute path without interpreting it against process state.
pub fn normalize_absolute_path(path: &Path, label: &str) -> ModelResult<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid(format!("{label} must be an absolute path")));
    }
    Ok(lexical_normalize(path))
}

/// Bind a raw filesystem operand to an already-captured invocation directory.
pub fn resolve_invocation_path(start: &Path, value: &str) -> ModelResult<PathBuf> {
    let start = normalize_absolute_path(start, "invocation caller_cwd")?;
    let path = Path::new(value);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        start_dir(&start).join(path)
    };
    normalize_absolute_path(&candidate, "resolved path")
}

/// Bind a local Git source path to the explicit invocation directory.
///
/// Network URLs, `file://` URLs, and scp-style Git syntax retain their wire
/// spelling. Every other value is a local filesystem operand and is made
/// absolute before it reaches the Git backend.
pub fn resolve_invocation_git_source(start: &Path, source: &str) -> ModelResult<String> {
    if source.is_empty() {
        return Err(invalid("Git source must not be empty"));
    }
    if source.contains("://") || crate::git::git_host(source).is_some() {
        return Ok(source.to_owned());
    }
    Ok(resolve_invocation_path(start, source)?
        .to_string_lossy()
        .into_owned())
}

/// Resolve the workspace addressed by one request using its serialized caller
/// context. An explicit root from a serialized request is already caller-bound
/// and therefore must be absolute. The legacy adapter is allowed to qualify a
/// relative root against its explicitly supplied absolute `start`.
pub fn resolve_request_workspace_root(
    start: &Path,
    meta: &crate::RequestMeta,
) -> ModelResult<PathBuf> {
    let caller_start = invocation_start(start, meta)?;
    resolve_workspace_root_from_caller(
        &caller_start,
        meta.workspace.as_ref(),
        meta.invocation.is_some(),
    )
}

fn resolve_workspace_root_from_caller(
    caller_start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    serialized: bool,
) -> ModelResult<PathBuf> {
    if let Some(root) = workspace.and_then(|workspace| workspace.root.as_ref()) {
        let root = Path::new(root);
        if root.is_absolute() {
            return normalize_absolute_path(root, "workspace root");
        }
        if serialized {
            return Err(invalid("workspace root must be an absolute path"));
        }
        return resolve_invocation_path(caller_start, root.to_string_lossy().as_ref());
    }
    discover_workspace_root(caller_start)
}

pub(crate) fn resolve_input_path(start: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        normalize_path(path)
    } else {
        normalize_path(&start_dir(start).join(path))
    }
}

pub(crate) fn start_dir(start: &Path) -> &Path {
    if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{caller_directory, path_text};

    fn meta(caller_cwd: Option<&str>) -> crate::RequestMeta {
        crate::RequestMeta {
            invocation: caller_cwd.map(|cwd| crate::InvocationContext {
                caller_cwd: cwd.to_owned(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn caller_directory_comes_only_from_the_request() {
        let (given, expected) = if cfg!(windows) {
            (r"C:\work\a\..\b", r"C:\work\b")
        } else {
            ("/work/a/../b", "/work/b")
        };
        assert_eq!(caller_directory(&meta(Some(given))).unwrap(), PathBuf::from(expected));
        assert!(caller_directory(&meta(None)).is_err());
        assert!(caller_directory(&meta(Some("relative/dir"))).is_err());
    }

    #[test]
    fn path_text_is_exact_or_refused() {
        let valid = if cfg!(windows) { r"C:\work\b" } else { "/work/b" };
        assert_eq!(path_text(Path::new(valid), "root").unwrap(), valid);
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                use std::os::unix::ffi::OsStrExt;
                let path = Path::new(std::ffi::OsStr::from_bytes(b"/work/\xff"));
                let error = path_text(path, "working directory").unwrap_err();
                assert!(format!("{error}").contains("\\xFF"), "{error}");
            }
        }
    }
}
