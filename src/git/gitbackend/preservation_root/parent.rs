use super::super::*;
use super::files;
use super::{FaultBoundary, fault};

use cap_fs_ext::{DirExt, ambient_authority};
use cap_std::fs::Dir;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
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
    root: Dir,
    dir: Dir,
    identity: (u64, u64),
}

impl PinnedConfig {
    pub(super) fn open(root: &Path) -> ModelResult<Self> {
        let root =
            Dir::open_ambient_dir(root, ambient_authority()).map_err(crate::git::io_error)?;
        let metadata = root
            .symlink_metadata("gwz.conf")
            .map_err(crate::git::io_error)?;
        if !metadata.is_dir() || metadata.is_symlink() {
            return Err(evidence_error("gwz.conf parent is missing or replaced"));
        }
        let dir = root
            .open_dir_nofollow("gwz.conf")
            .map_err(crate::git::io_error)?;
        let identity = files::identity(&metadata);
        if identity != files::identity(&dir.dir_metadata().map_err(crate::git::io_error)?) {
            return Err(evidence_error("gwz.conf parent changed while opening"));
        }
        Ok(Self {
            root,
            dir,
            identity,
        })
    }

    pub(super) fn is_current(&self) -> ModelResult<bool> {
        let metadata = match self.root.symlink_metadata("gwz.conf") {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(crate::git::io_error(error)),
        };
        Ok(metadata.is_dir()
            && !metadata.is_symlink()
            && files::identity(&metadata) == self.identity)
    }

    pub(super) fn observe(&self, marker_path: &str, staging: &str) -> ModelResult<State> {
        let (_, marker) = files::split_relative(Path::new(marker_path))?;
        let final_state = directory_state(&self.dir, Path::new(FINAL_NAME), Some(&marker))?;
        let stage_state = directory_state(&self.dir, Path::new(staging), None)?;
        let foreign_stage = self
            .dir
            .entries()
            .map_err(crate::git::io_error)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::git::io_error)?
            .into_iter()
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
        if !self.is_current()? {
            return Err(evidence_error("gwz.conf parent changed before publication"));
        }
        if directory_state(&self.dir, Path::new(staging), None)? == DirectoryState::Missing {
            fault(FaultBoundary::BeforeParentStageCreate)?;
            self.dir.create_dir(staging).map_err(crate::git::io_error)?;
            fault(FaultBoundary::AfterParentStageCreate)?;
        }
        let stage = self
            .dir
            .open_dir_nofollow(staging)
            .map_err(crate::git::io_error)?;
        let mut entries = stage.entries().map_err(crate::git::io_error)?;
        match entries.next() {
            Some(Ok(_)) => {
                return Err(evidence_error(
                    "marker-parent staging directory is not empty",
                ));
            }
            Some(Err(error)) => return Err(crate::git::io_error(error)),
            None => {}
        }
        #[cfg(unix)]
        sync_dir(&stage)?;
        fault(FaultBoundary::BeforeParentPublish)?;
        rename_no_replace(&self.dir, staging, FINAL_NAME)?;
        fault(FaultBoundary::AfterParentPublish)?;
        barrier_after_publish(&self.dir)?;
        if !self.is_current()? {
            return Err(evidence_error("gwz.conf parent changed during publication"));
        }
        Ok(())
    }

    pub(super) fn barrier(&self, staging: &str) -> ModelResult<()> {
        if !self.is_current()? {
            return Err(evidence_error(
                "gwz.conf parent changed before durability barrier",
            ));
        }
        barrier_platform(&self.dir, staging, FINAL_NAME)?;
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
    dir: &Dir,
    name: &Path,
    expected: Option<&OsString>,
) -> ModelResult<DirectoryState> {
    let metadata = match dir.symlink_metadata(name) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DirectoryState::Missing);
        }
        Err(error) => return Err(crate::git::io_error(error)),
    };
    if !metadata.is_dir() || metadata.is_symlink() {
        return Ok(DirectoryState::Invalid);
    }
    let child = match dir.open_dir_nofollow(name) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(crate::git::io_error(error));
        }
        Err(_) => return Ok(DirectoryState::Invalid),
    };
    let entries = child
        .entries()
        .map_err(crate::git::io_error)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(crate::git::io_error)?;
    Ok(match (entries.as_slice(), expected) {
        ([], _) => DirectoryState::Empty,
        ([actual], Some(expected)) if actual == expected => DirectoryState::ExpectedLeaf,
        _ => DirectoryState::Invalid,
    })
}

#[cfg(unix)]
fn sync_dir(dir: &Dir) -> ModelResult<()> {
    dir.try_clone()
        .and_then(|value| value.into_std_file().sync_all())
        .map_err(crate::git::io_error)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rename_no_replace(dir: &Dir, source: &str, destination: &str) -> ModelResult<()> {
    rustix::fs::renameat_with(
        dir,
        source,
        dir,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|error| crate::git::io_error(std::io::Error::from_raw_os_error(error.raw_os_error())))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn rename_no_replace(_dir: &Dir, _source: &str, _destination: &str) -> ModelResult<()> {
    Err(ModelError::new(
        ErrorCode::UnsupportedOperation,
        "atomic no-replace directory publication is unsupported on this Unix target",
    ))
}

#[cfg(unix)]
fn barrier_after_publish(dir: &Dir) -> ModelResult<()> {
    sync_parent(dir)
}

#[cfg(unix)]
fn barrier_platform(dir: &Dir, _staging: &str, _final_name: &str) -> ModelResult<()> {
    sync_parent(dir)
}

#[cfg(unix)]
fn sync_parent(dir: &Dir) -> ModelResult<()> {
    fault(FaultBoundary::BeforeUnixParentSync)?;
    sync_dir(dir)?;
    fault(FaultBoundary::AfterUnixParentSync)
}

#[cfg(windows)]
fn rename_no_replace(dir: &Dir, source: &str, destination: &str) -> ModelResult<()> {
    rename_windows(dir, source, destination)
}

#[cfg(windows)]
fn barrier_after_publish(_dir: &Dir) -> ModelResult<()> {
    Ok(())
}

#[cfg(windows)]
fn barrier_platform(dir: &Dir, staging: &str, final_name: &str) -> ModelResult<()> {
    fault(FaultBoundary::BeforeWindowsFirstBarrierRename)?;
    rename_windows(dir, final_name, staging)?;
    fault(FaultBoundary::AfterWindowsFirstBarrierRename)?;
    fault(FaultBoundary::BeforeWindowsSecondBarrierRename)?;
    rename_windows(dir, staging, final_name)?;
    fault(FaultBoundary::AfterWindowsSecondBarrierRename)
}

#[cfg(windows)]
fn rename_windows(dir: &Dir, source: &str, destination: &str) -> ModelResult<()> {
    use cap_fs_ext::{OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::*;

    let mut options = OpenOptions::new();
    options
        .access_mode(DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH,
        )
        .follow(cap_fs_ext::FollowSymlinks::No)
        .maybe_dir(true);
    let source = dir
        .open_with(source, &options)
        .map_err(crate::git::io_error)?;
    let name = destination.encode_utf16().collect::<Vec<_>>();
    let size = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * 2;
    let mut storage = vec![0_usize; size.div_ceil(std::mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = dir.as_raw_handle();
        (*info).FileNameLength = u32::try_from(name.len() * 2)
            .map_err(|_| evidence_error("marker-parent staging name is too long"))?;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
        if SetFileInformationByHandle(
            source.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            u32::try_from(size).map_err(|_| evidence_error("rename buffer is too large"))?,
        ) == 0
        {
            return Err(crate::git::io_error(std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

fn evidence_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
}
