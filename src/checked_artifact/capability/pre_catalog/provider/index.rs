use std::ffi::OsString;
use std::io::{self, Read};
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use super::super::*;
use super::filesystem::PlatformProviderV1;
use super::retained::{RetainedFile, RetainedPlatformRoot, retain_index_file};
use super::snapshot::IndexSnapshotFacts;
use crate::checked_artifact::capability::{
    GitPathBytes, IndexTimestampV1, LosslessIndexEntry, LosslessIndexMetadataV1,
    PlatformCapability, TrackedWorktreeEntry, TrackedWorktreeKind,
};

pub(super) fn observe(
    retained: &RetainedPlatformRoot,
    platform: &impl PlatformProviderV1,
) -> Result<(IndexSnapshotFacts, Option<RetainedFile>), CheckedFsError> {
    let repository = git2::Repository::open(retained.root_path()).map_err(git_error)?;
    let index = repository.index().map_err(git_error)?;
    let expected_path = retained.git_directory_path().join("index");
    let actual_path = index.path().ok_or_else(|| {
        CheckedFsError::ambiguous("Git index", "repository returned an in-memory index")
    })?;
    if actual_path != expected_path {
        return Err(CheckedFsError::ambiguous(
            "Git index",
            "index path is not inside the actual Git directory",
        ));
    }

    let file = retain_index_file(retained.repository(), platform)?;
    let content_digest = file.as_ref().map(hash_file).transpose()?;
    let mut entries = Vec::new();
    entries.try_reserve_exact(index.len()).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "Git index fact allocation failed",
        )
    })?;
    let mut worktree = Vec::new();
    worktree.try_reserve_exact(index.len()).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "tracked worktree fact allocation failed",
        )
    })?;
    for entry in index.iter() {
        let stage = ((entry.flags >> 12) & 3) as u8;
        let path = GitPathBytes::new(entry.path.clone())?;
        entries.push(LosslessIndexEntry::new(
            path.clone(),
            stage,
            entry.mode,
            entry.flags,
            entry.flags_extended,
            LosslessIndexMetadataV1::new(
                IndexTimestampV1::new(entry.ctime.seconds(), entry.ctime.nanoseconds())?,
                IndexTimestampV1::new(entry.mtime.seconds(), entry.mtime.nanoseconds())?,
                [entry.dev, entry.ino, entry.uid, entry.gid, entry.file_size],
                entry.id.as_bytes().to_vec(),
            )?,
        )?);
        worktree.push(TrackedWorktreeEntry::new(
            path,
            worktree_kind(retained, &entry)?,
        ));
    }
    entries.sort_unstable_by(|left, right| {
        left.path()
            .as_bytes()
            .cmp(right.path().as_bytes())
            .then_with(|| left.stage().code().cmp(&right.stage().code()))
    });
    worktree.sort_unstable_by(|left, right| {
        left.path()
            .as_bytes()
            .cmp(right.path().as_bytes())
            .then_with(|| left.kind().code().cmp(&right.kind().code()))
    });
    let file_identity = file.as_ref().map(RetainedFile::encoded_identity);
    Ok((
        IndexSnapshotFacts {
            file_identity,
            content_digest,
            entries,
            worktree,
        },
        file,
    ))
}

fn hash_file(file: &RetainedFile) -> Result<[u8; 32], CheckedFsError> {
    let mut reader = file
        .handle()
        .try_clone()
        .map_err(|source| CheckedFsError::io("clone retained Git index", source))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| CheckedFsError::io("read retained Git index", source))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().into())
}

fn worktree_kind(
    retained: &RetainedPlatformRoot,
    entry: &git2::IndexEntry,
) -> Result<TrackedWorktreeKind, CheckedFsError> {
    if entry.mode & 0o170000 == 0o160000 {
        return Ok(TrackedWorktreeKind::Gitlink);
    }
    let Some(path) = worktree_path(&entry.path) else {
        return Ok(TrackedWorktreeKind::Other);
    };
    match retained.root().handle().symlink_metadata(&path) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(TrackedWorktreeKind::Missing),
        Err(source) => Err(CheckedFsError::io("observe tracked worktree path", source)),
        Ok(metadata) if metadata.is_symlink() => Ok(TrackedWorktreeKind::Symlink),
        Ok(metadata) if metadata.is_file() => Ok(TrackedWorktreeKind::RegularFile),
        Ok(metadata) if metadata.is_dir() => Ok(TrackedWorktreeKind::Directory),
        Ok(_) => Ok(TrackedWorktreeKind::Other),
    }
}

#[cfg(unix)]
fn worktree_path(path: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(OsString::from_vec(path.to_vec())))
}

#[cfg(windows)]
fn worktree_path(path: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(path).ok().map(PathBuf::from)
}

#[cfg(not(any(unix, windows)))]
fn worktree_path(path: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(path).ok().map(PathBuf::from)
}

fn git_error(error: git2::Error) -> CheckedFsError {
    CheckedFsError::io(
        "read pre-catalog Git index",
        io::Error::other(error.message().to_owned()),
    )
}
