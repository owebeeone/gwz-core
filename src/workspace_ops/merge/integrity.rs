//! The configuration marker is part of new composition candidates. Legacy
//! candidates without this field keep their original publication/rollback shape.
use std::path::Path;

use crate::artifact::{self, CONF_INTEGRITY_MARKER_PATH};
use crate::filesystem::{FileSystem, FsKind};
use crate::git::{GitBackend, GitCandidateFile};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::PublicationCandidate;
use super::root::artifact_facts::RegularFileFact;

pub(super) fn append_candidate(
    candidate: &PublicationCandidate,
    files: &mut Vec<GitCandidateFile>,
) {
    if let Some(marker) = &candidate.conf_integrity {
        files.push(GitCandidateFile {
            path: CONF_INTEGRITY_MARKER_PATH.into(),
            bytes: marker.yaml.as_bytes().to_vec(),
        });
    }
}

pub(super) fn append_baseline(
    candidate: &PublicationCandidate,
    files: &mut Vec<GitCandidateFile>,
    absent: &mut Vec<String>,
) {
    if let Some(marker) = &candidate.conf_integrity {
        if let Some(bytes) = &marker.baseline {
            files.push(GitCandidateFile {
                path: CONF_INTEGRITY_MARKER_PATH.into(),
                bytes: bytes.clone(),
            });
        } else {
            absent.push(CONF_INTEGRITY_MARKER_PATH.into());
        }
    }
}

pub(super) fn capture_baseline<B: GitBackend>(
    filesystem: &dyn FileSystem,
    backend: &B,
    root: &Path,
    head: Option<&str>,
) -> ModelResult<Option<Vec<u8>>> {
    let observed = observe(filesystem, root)?;
    let baseline = match head {
        Some(head) => backend.read_file_at_commit(root, head, CONF_INTEGRITY_MARKER_PATH)?,
        // A newly initialized root has no committed tree. Its valid staged
        // marker belongs to the same initial configuration as the staged lock.
        // Capture it exactly so abort restores that initial index/worktree state.
        None => match &observed {
            RegularFileFact::Missing => None,
            RegularFileFact::Bytes(bytes)
                if artifact::inspect_conf_integrity_in(filesystem, root)
                    == artifact::ConfIntegrityVerdict::Verified =>
            {
                Some(bytes.clone())
            }
            _ => return Err(drift()),
        },
    };
    let expected = baseline
        .clone()
        .map_or(RegularFileFact::Missing, RegularFileFact::Bytes);
    let (files, absent) = match &baseline {
        Some(bytes) => (
            vec![GitCandidateFile {
                path: CONF_INTEGRITY_MARKER_PATH.into(),
                bytes: bytes.clone(),
            }],
            Vec::new(),
        ),
        None => (Vec::new(), vec![CONF_INTEGRITY_MARKER_PATH.into()]),
    };
    if observed != expected
        || !backend.index_entries_match_candidate_files(root, &files, &absent)?
    {
        return Err(drift());
    }
    Ok(baseline)
}

/// Both exact states are resumable before staging. No third value is owned.
pub(super) fn states(
    filesystem: &dyn FileSystem,
    root: &Path,
    candidate: &PublicationCandidate,
) -> ModelResult<(bool, bool)> {
    let Some(marker) = &candidate.conf_integrity else {
        return Ok((true, true));
    };
    let observed = observe(filesystem, root)?;
    let baseline = marker
        .baseline
        .clone()
        .map_or(RegularFileFact::Missing, RegularFileFact::Bytes);
    Ok((
        observed == baseline,
        observed == RegularFileFact::Bytes(marker.yaml.as_bytes().to_vec()),
    ))
}

pub(super) fn publish(
    filesystem: &dyn FileSystem,
    root: &Path,
    candidate: &PublicationCandidate,
) -> ModelResult<()> {
    let Some(marker) = &candidate.conf_integrity else {
        return Ok(());
    };
    let (baseline, published) = states(filesystem, root, candidate)?;
    if !baseline && !published {
        return Err(drift());
    }
    if !published {
        // Atomic rename has no intermediate absence. A crash before index staging
        // leaves exact candidate bytes, which the publication observer admits.
        artifact::write_atomic_in(
            filesystem,
            &root.join(CONF_INTEGRITY_MARKER_PATH),
            &marker.yaml,
        )?;
    }
    Ok(())
}

pub(super) fn restore(
    filesystem: &dyn FileSystem,
    root: &Path,
    candidate: &PublicationCandidate,
) -> ModelResult<()> {
    let Some(marker) = &candidate.conf_integrity else {
        return Ok(());
    };
    let (baseline, published) = states(filesystem, root, candidate)?;
    if !baseline && !published {
        return Err(drift());
    }
    if !baseline {
        if let Some(bytes) = &marker.baseline {
            artifact::write_atomic_bytes_in(
                filesystem,
                &root.join(CONF_INTEGRITY_MARKER_PATH),
                bytes,
            )?;
        } else {
            filesystem
                .remove_file(&root.join(CONF_INTEGRITY_MARKER_PATH))
                .map_err(|error| ModelError::new(ErrorCode::IoError, error.to_string()))?;
        }
    }
    Ok(())
}

fn drift() -> ModelError {
    ModelError::new(
        ErrorCode::MergeDrift,
        "configuration integrity marker differs from the frozen publication baseline and candidate",
    )
}

/// Forward publication also runs on filesystems without persistent handles.
/// Match its existing raw lock/marker observers: regular bytes, explicit parent
/// boundaries and no symlinks, without adding a checked-recovery capability gate.
fn observe(filesystem: &dyn FileSystem, root: &Path) -> ModelResult<RegularFileFact> {
    for (relative, kind) in [
        ("gwz.conf", FsKind::Directory),
        ("gwz.conf/markers", FsKind::Directory),
        (CONF_INTEGRITY_MARKER_PATH, FsKind::File),
    ] {
        let metadata = match filesystem.metadata(&root.join(relative)) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RegularFileFact::Missing);
            }
            Err(error) => return Err(ModelError::new(ErrorCode::IoError, error.to_string())),
        };
        if metadata.kind != kind || (kind == FsKind::File && metadata.executable) {
            return Ok(RegularFileFact::Invalid);
        }
    }
    filesystem
        .read(&root.join(CONF_INTEGRITY_MARKER_PATH))
        .map(RegularFileFact::Bytes)
        .map_err(|error| ModelError::new(ErrorCode::IoError, error.to_string()))
}
