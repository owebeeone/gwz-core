use super::super::*;
use crate::checked_artifact::entry::MergeArtifactTransition;
use cap_fs_ext::MetadataExt;
use std::path::{Component, Path, PathBuf};

pub(super) fn observe_relative(
    root: &Path,
    expected: &Option<GitCandidateFile>,
    path: &str,
) -> ModelResult<bool> {
    crate::checked_artifact::entry::observe_merge_preservation_workspace(
        root,
        Path::new(path),
        expected.as_ref().map(|file| file.bytes.as_slice()),
    )
}

pub(super) fn observe_required(root: &Path, expected: &GitCandidateFile) -> ModelResult<bool> {
    crate::checked_artifact::entry::observe_merge_preservation_workspace(
        root,
        Path::new(&expected.path),
        Some(&expected.bytes),
    )
}

pub(super) fn observe_boundary(root: &Path, expected: &[u8]) -> ModelResult<bool> {
    let repo = open_repo(root)?;
    crate::checked_artifact::entry::observe_merge_preservation_git_directory(
        repo.path(),
        Path::new("info/exclude"),
        Some(expected),
    )
}

pub(super) fn replace_relative(
    root: &Path,
    path: &str,
    source: Option<&GitCandidateFile>,
    goal: Option<&GitCandidateFile>,
) -> ModelResult<()> {
    crate::checked_artifact::entry::replace_merge_preservation_workspace(
        root,
        Path::new(path),
        source.map(|file| file.bytes.as_slice()),
        goal.map(|file| file.bytes.as_slice()),
    )
}

pub(super) fn observe_transition(
    root: &Path,
    path: &str,
    source: Option<&GitCandidateFile>,
    goal: Option<&GitCandidateFile>,
) -> ModelResult<MergeArtifactTransition> {
    crate::checked_artifact::entry::classify_merge_preservation_workspace(
        root,
        Path::new(path),
        source.map(|file| file.bytes.as_slice()),
        goal.map(|file| file.bytes.as_slice()),
    )
}

pub(super) fn split_relative(path: &Path) -> ModelResult<(PathBuf, std::ffi::OsString)> {
    if path.is_absolute() {
        return Err(evidence_error("managed path is not relative"));
    }
    let mut components = path.components().collect::<Vec<_>>();
    let leaf = match components.pop() {
        Some(Component::Normal(leaf)) => leaf.to_owned(),
        _ => return Err(evidence_error("managed leaf name is noncanonical")),
    };
    let mut parent = PathBuf::new();
    for component in components {
        let Component::Normal(component) = component else {
            return Err(evidence_error("managed parent path is noncanonical"));
        };
        parent.push(component);
    }
    Ok((parent, leaf))
}

fn evidence_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
}

pub(super) fn identity(metadata: &cap_fs_ext::Metadata) -> (u64, u64) {
    (MetadataExt::dev(metadata), MetadataExt::ino(metadata))
}

#[cfg(unix)]
pub(in crate::git::gitbackend) fn raw_path_to_path(value: &[u8]) -> ModelResult<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(value.to_vec()).into())
}

#[cfg(not(unix))]
pub(in crate::git::gitbackend) fn raw_path_to_path(value: &[u8]) -> ModelResult<PathBuf> {
    String::from_utf8(value.to_vec())
        .map(Into::into)
        .map_err(|_| evidence_error("non-UTF-8 Git path is unsupported on this platform"))
}

#[cfg(unix)]
pub(in crate::git::gitbackend) fn path_to_raw(value: &Path) -> ModelResult<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    Ok(value.as_os_str().as_bytes().to_vec())
}

#[cfg(not(unix))]
pub(in crate::git::gitbackend) fn path_to_raw(value: &Path) -> ModelResult<Vec<u8>> {
    value
        .to_str()
        .map(|value| value.as_bytes().to_vec())
        .ok_or_else(|| evidence_error("non-UTF-8 path is unsupported on this platform"))
}
