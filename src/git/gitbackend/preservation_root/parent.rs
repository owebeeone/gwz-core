use super::super::*;
use super::files;
use super::{FaultBoundary, fault};

use crate::filesystem::{FileSystem, FsDirectory, RenameMode, make_filesystem};
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::path::Path;

const STAGE_PREFIX: &str = ".gwz-markers-";
const FINAL_NAME: &str = "markers";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum State {
    Missing,
    Empty,
    ExpectedMarker,
    StagingOnly,
    Invalid,
}

pub(super) struct PinnedConfig {
    root: FsDirectory,
    dir: FsDirectory,
}

impl PinnedConfig {
    pub(super) fn open(root: &Path) -> ModelResult<Self> {
        let filesystem = make_filesystem();
        let root = filesystem
            .open_directory(root)
            .map_err(crate::git::io_error)?;
        let dir = filesystem
            .open_directory_at(&root, OsStr::new("gwz.conf"))
            .map_err(crate::git::io_error)?;
        if !filesystem
            .directory_entry_matches(&root, OsStr::new("gwz.conf"), &dir)
            .map_err(crate::git::io_error)?
        {
            return Err(evidence_error("gwz.conf parent changed while opening"));
        }
        Ok(Self { root, dir })
    }

    pub(super) fn is_current(&self) -> ModelResult<bool> {
        make_filesystem()
            .directory_entry_matches(&self.root, OsStr::new("gwz.conf"), &self.dir)
            .map_err(crate::git::io_error)
    }

    pub(super) fn observe(&self, marker_path: &str, staging: &str) -> ModelResult<State> {
        let (_, marker) = files::split_relative(Path::new(marker_path))?;
        let filesystem = make_filesystem();
        let final_state =
            directory_state(&filesystem, &self.dir, FINAL_NAME.as_ref(), Some(&marker))?;
        let stage_state = directory_state(&filesystem, &self.dir, staging.as_ref(), None)?;
        let foreign_stage = filesystem
            .read_directory_at(&self.dir)
            .map_err(crate::git::io_error)?
            .into_iter()
            .map(|entry| entry.name)
            .any(|name| name.to_string_lossy().starts_with(STAGE_PREFIX) && name != staging);
        Ok(if !self.is_current()? || foreign_stage {
            State::Invalid
        } else {
            match (final_state, stage_state) {
                (DirectoryState::Missing, DirectoryState::Missing) => State::Missing,
                (DirectoryState::Missing, DirectoryState::Empty) => State::StagingOnly,
                (DirectoryState::Empty, DirectoryState::Missing) => State::Empty,
                (DirectoryState::ExpectedLeaf, DirectoryState::Missing) => State::ExpectedMarker,
                _ => State::Invalid,
            }
        })
    }

    pub(super) fn publish(&self, staging: &str) -> ModelResult<()> {
        let filesystem = make_filesystem();
        if !self.is_current()? {
            return Err(evidence_error("gwz.conf parent changed before publication"));
        }
        if directory_state(&filesystem, &self.dir, staging.as_ref(), None)?
            == DirectoryState::Missing
        {
            fault(FaultBoundary::BeforeParentStageCreate)?;
            filesystem
                .create_directory_at(&self.dir, staging.as_ref())
                .map_err(crate::git::io_error)?;
            fault(FaultBoundary::AfterParentStageCreate)?;
        }
        let stage = filesystem
            .open_directory_at(&self.dir, staging.as_ref())
            .map_err(crate::git::io_error)?;
        if !filesystem
            .read_directory_at(&stage)
            .map_err(crate::git::io_error)?
            .is_empty()
        {
            return Err(evidence_error(
                "marker-parent staging directory is not empty",
            ));
        }
        #[cfg(unix)]
        filesystem
            .sync_directory_at(&stage)
            .map_err(crate::git::io_error)?;
        // Windows denies renaming a directory whose handle lacks DELETE
        // sharing, and `stage` is our own such handle; release it before the
        // publish rename (same edge as the staging-capability drop in
        // pre_catalog/provider/directory_mutation.rs).
        drop(stage);
        fault(FaultBoundary::BeforeParentPublish)?;
        filesystem
            .rename_at(
                &self.dir,
                staging.as_ref(),
                &self.dir,
                FINAL_NAME.as_ref(),
                RenameMode::NoReplace,
            )
            .map_err(crate::git::io_error)?;
        fault(FaultBoundary::AfterParentPublish)?;
        barrier_after_publish(&filesystem, &self.dir)?;
        if !self.is_current()? {
            return Err(evidence_error("gwz.conf parent changed during publication"));
        }
        Ok(())
    }

    pub(super) fn barrier(&self, staging: &str) -> ModelResult<()> {
        let filesystem = make_filesystem();
        if !self.is_current()? {
            return Err(evidence_error(
                "gwz.conf parent changed before durability barrier",
            ));
        }
        barrier_platform(&filesystem, &self.dir, staging, FINAL_NAME)?;
        if !self.is_current()? {
            return Err(evidence_error(
                "gwz.conf parent changed during durability barrier",
            ));
        }
        Ok(())
    }
}

pub(super) fn staging_name(
    spec: &GitRootPreservationSpec,
    source: GitRootManagedFormName,
    goal: GitRootManagedFormName,
) -> String {
    let mut hash = Sha256::new();
    for value in [
        spec.managed_marker_path.as_str(),
        spec.attached_commit.as_str(),
        spec.restore_commit.as_str(),
    ] {
        hash.update(value.len().to_be_bytes());
        hash.update(value.as_bytes());
    }
    for name in [source, goal] {
        hash.update([name as u8]);
        hash_form(&mut hash, selected_form(spec, name));
    }
    format!("{STAGE_PREFIX}{:x}.stage", hash.finalize())
}

fn selected_form(
    spec: &GitRootPreservationSpec,
    name: GitRootManagedFormName,
) -> &GitRootManagedForm {
    match name {
        GitRootManagedFormName::AttachedClean => &spec.attached_clean_form,
        GitRootManagedFormName::RestoreClean => &spec.restore_clean_form,
        GitRootManagedFormName::Handoff => &spec.handoff_form,
    }
}

#[rustfmt::skip]
fn hash_form(hash: &mut Sha256, form: &GitRootManagedForm) {
    fn field(hash: &mut Sha256, value: &[u8]) {
        hash.update(value.len().to_be_bytes()); hash.update(value);
    }
    fn file(hash: &mut Sha256, value: Option<&GitCandidateFile>) {
        hash.update([u8::from(value.is_some())]);
        if let Some(value) = value { field(hash, value.path.as_bytes()); field(hash, &value.bytes); }
    }
    fn fact(hash: &mut Sha256, value: &GitRootManagedIndexFact) {
        match value {
            GitRootManagedIndexFact::Absent { path } => { hash.update([0]); field(hash, path); }
            GitRootManagedIndexFact::Present(entry) => {
                hash.update([1]); field(hash, &entry.path); field(hash, entry.object_id.as_bytes());
                hash.update(entry.mode.to_be_bytes()); hash.update([entry.stage, entry.assume_valid as u8,
                    entry.skip_worktree as u8, entry.intent_to_add as u8]);
            }
        }
    }
    file(hash, form.marker.as_ref()); file(hash, Some(&form.lock));
    fact(hash, &form.index.marker); fact(hash, &form.index.lock);
}

pub(super) fn observe(root: &Path, marker_path: &str, staging: &str) -> ModelResult<State> {
    PinnedConfig::open(root)?.observe(marker_path, staging)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectoryState {
    Missing,
    Empty,
    ExpectedLeaf,
    Invalid,
}

fn directory_state(
    filesystem: &impl FileSystem,
    dir: &FsDirectory,
    name: &OsStr,
    expected: Option<&OsString>,
) -> ModelResult<DirectoryState> {
    let child = match filesystem.open_directory_at(dir, name) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DirectoryState::Missing);
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(crate::git::io_error(error));
        }
        Err(_) => return Ok(DirectoryState::Invalid),
    };
    let entries = filesystem
        .read_directory_at(&child)
        .map_err(crate::git::io_error)?;
    Ok(match (entries.as_slice(), expected) {
        ([], _) => DirectoryState::Empty,
        ([actual], Some(expected)) if &actual.name == expected => DirectoryState::ExpectedLeaf,
        _ => DirectoryState::Invalid,
    })
}

#[cfg(unix)]
fn barrier_after_publish(filesystem: &impl FileSystem, dir: &FsDirectory) -> ModelResult<()> {
    sync_parent(filesystem, dir)
}

#[cfg(unix)]
fn barrier_platform(
    filesystem: &impl FileSystem,
    dir: &FsDirectory,
    _staging: &str,
    _final_name: &str,
) -> ModelResult<()> {
    sync_parent(filesystem, dir)
}

#[cfg(unix)]
fn sync_parent(filesystem: &impl FileSystem, dir: &FsDirectory) -> ModelResult<()> {
    fault(FaultBoundary::BeforeUnixParentSync)?;
    filesystem
        .sync_directory_at(dir)
        .map_err(crate::git::io_error)?;
    fault(FaultBoundary::AfterUnixParentSync)
}

#[cfg(windows)]
fn barrier_after_publish(_filesystem: &impl FileSystem, _dir: &FsDirectory) -> ModelResult<()> {
    Ok(())
}

#[cfg(windows)]
fn barrier_platform(
    filesystem: &impl FileSystem,
    dir: &FsDirectory,
    staging: &str,
    final_name: &str,
) -> ModelResult<()> {
    fault(FaultBoundary::BeforeWindowsFirstBarrierRename)?;
    filesystem
        .rename_at(
            dir,
            final_name.as_ref(),
            dir,
            staging.as_ref(),
            RenameMode::NoReplace,
        )
        .map_err(crate::git::io_error)?;
    fault(FaultBoundary::AfterWindowsFirstBarrierRename)?;
    fault(FaultBoundary::BeforeWindowsSecondBarrierRename)?;
    filesystem
        .rename_at(
            dir,
            staging.as_ref(),
            dir,
            final_name.as_ref(),
            RenameMode::NoReplace,
        )
        .map_err(crate::git::io_error)?;
    fault(FaultBoundary::AfterWindowsSecondBarrierRename)
}
fn evidence_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
}
