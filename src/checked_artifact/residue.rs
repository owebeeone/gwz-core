use std::ffi::{OsStr, OsString};
use std::io::Write;

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, ambient_authority};
use cap_std::fs::{Dir, OpenOptions};

use super::fault::{CheckedArtifactFault, fault};
use super::observation::{observe_leaf, observe_leaf_exact};
use super::{CheckedArtifact, CheckedArtifactFact, error, io_error};
use crate::model::ModelResult;

pub(super) struct QuarantinedSource {
    pub(super) name: OsString,
    pub(super) parent_identity: (u64, u64),
    pub(super) identity: (u64, u64),
}

pub(super) struct TransitionResidue {
    pub(super) source: Option<QuarantinedSource>,
    pub(super) goal_staged: bool,
    pub(super) foreign: bool,
}

impl CheckedArtifact {
    pub(super) fn open_quarantine(&self, create: bool) -> ModelResult<Option<Dir>> {
        if create {
            Self::prepare_parent(
                &self.private_root,
                &self.quarantine_parent,
                self.code,
                &self.label,
            )?;
        }
        let root = Dir::open_ambient_dir(&self.private_root, ambient_authority())
            .map_err(|cause| io_error(self.code, &self.label, cause))?;
        let mut current = root;
        for component in self.quarantine_parent.components() {
            let std::path::Component::Normal(component) = component else {
                return Err(error(
                    self.code,
                    &self.label,
                    "private recovery path is noncanonical",
                ));
            };
            current = match current.open_dir_nofollow(component) {
                Ok(dir) => dir,
                Err(cause) if cause.kind() == std::io::ErrorKind::NotFound && !create => {
                    return Ok(None);
                }
                Err(cause) => return Err(io_error(self.code, &self.label, cause)),
            };
        }
        Ok(Some(current))
    }

    pub(super) fn inspect_residue(
        &self,
        key: &str,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
    ) -> ModelResult<TransitionResidue> {
        let Some(dir) = self.open_quarantine(false)? else {
            return Ok(TransitionResidue {
                source: None,
                goal_staged: false,
                foreign: false,
            });
        };
        let mut source = None;
        let mut goal_staged = false;
        let mut foreign = false;
        let goal_name = goal_name(key);
        for entry in dir
            .entries()
            .map_err(|cause| io_error(self.code, &self.label, cause))?
        {
            let entry = entry.map_err(|cause| io_error(self.code, &self.label, cause))?;
            let name = entry.file_name();
            let text = name.to_string_lossy();
            if !text.starts_with(key) {
                continue;
            }
            if name == goal_name {
                goal_staged = goal.is_some_and(|bytes| {
                    observe_leaf(&dir, &name, self.code, &self.label)
                        .is_ok_and(|fact| fact == CheckedArtifactFact::Bytes(bytes.to_vec()))
                });
                foreign |= !goal_staged;
                continue;
            }
            let Some(parent_identity) = parse_source_name(key, &text) else {
                foreign = true;
                continue;
            };
            let observed = observe_leaf_exact(&dir, &name, self.code, &self.label)?;
            if source.is_some() || observed.fact != *expected || observed.identity.is_none() {
                foreign = true;
                continue;
            }
            source = Some(QuarantinedSource {
                name,
                parent_identity,
                identity: observed.identity.expect("checked above"),
            });
        }
        Ok(TransitionResidue {
            source,
            goal_staged,
            foreign,
        })
    }

    pub(super) fn stage_goal(&self, dir: &Dir, key: &str, goal: &[u8]) -> ModelResult<()> {
        let name = goal_name(key);
        match observe_leaf(dir, &name, self.code, &self.label)? {
            CheckedArtifactFact::Bytes(bytes) if bytes == goal => return Ok(()),
            CheckedArtifactFact::Missing => {}
            CheckedArtifactFact::Bytes(_) | CheckedArtifactFact::Invalid => {
                return Err(error(
                    self.code,
                    &self.label,
                    "foreign staged replacement goal",
                ));
            }
        }
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        cap_std::fs::OpenOptionsExt::mode(&mut options, 0o644);
        let mut file = dir
            .open_with(&name, &options)
            .map_err(|cause| io_error(self.code, &self.label, cause))?;
        file.write_all(goal)
            .map_err(|cause| io_error(self.code, &self.label, cause))?;
        file.sync_all()
            .map_err(|cause| io_error(self.code, &self.label, cause))?;
        drop(file);
        super::platform::sync_parent(dir).map_err(|cause| io_error(self.code, &self.label, cause))
    }

    pub(super) fn restore_source(
        &self,
        quarantine: &Dir,
        source: &OsStr,
        parent: &Dir,
    ) -> ModelResult<()> {
        if observe_leaf(parent, &self.leaf, self.code, &self.label)? != CheckedArtifactFact::Missing
        {
            return Err(error(
                self.code,
                &self.label,
                "foreign destination prevents safe source restoration",
            ));
        }
        super::platform::rename_relative(
            quarantine,
            source,
            parent,
            &self.leaf,
            false,
            self.code,
            &self.label,
        )?;
        super::platform::sync_parent(parent)
            .and_then(|_| super::platform::sync_parent(quarantine))
            .map_err(|cause| io_error(self.code, &self.label, cause))
    }

    pub(super) fn cleanup_source(
        &self,
        quarantine: &Dir,
        key: &str,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
    ) -> ModelResult<()> {
        let residue = self.inspect_residue(key, expected, goal)?;
        if residue.foreign || residue.goal_staged {
            return Err(error(
                self.code,
                &self.label,
                "foreign residue prevents checked-artifact cleanup",
            ));
        }
        if let Some(source) = residue.source {
            quarantine
                .remove_file(&source.name)
                .map_err(|cause| io_error(self.code, &self.label, cause))?;
            super::platform::sync_parent(quarantine)
                .map_err(|cause| io_error(self.code, &self.label, cause))?;
        }
        fault(CheckedArtifactFault::AfterCleanup, self.code, &self.label)
    }
}

pub(super) fn goal_name(key: &str) -> OsString {
    OsString::from(format!("{key}.goal"))
}

pub(super) fn source_name(key: &str, identity: (u64, u64)) -> OsString {
    OsString::from(format!(
        "{key}-{:016x}-{:016x}.source",
        identity.0, identity.1
    ))
}

fn parse_source_name(key: &str, name: &str) -> Option<(u64, u64)> {
    let tail = name.strip_prefix(key)?.strip_prefix('-')?;
    let tail = tail.strip_suffix(".source")?;
    let (device, inode) = tail.split_once('-')?;
    Some((
        u64::from_str_radix(device, 16).ok()?,
        u64::from_str_radix(inode, 16).ok()?,
    ))
}
