use std::ffi::{OsStr, OsString};
use std::path::Component;

use super::authority::{
    CheckedArtifactAuthority, RetainedSource, authority_name, family_prefix, goal_name,
    scratch_name, source_name,
};
use super::fault::{CheckedArtifactFault, fault};
use super::identity::{self, ObjectIdentity};
use super::observation::{LeafObservation, io_op_error, observe_leaf_exact};
use super::{CheckedArtifact, CheckedArtifactFact, ParentState, error};
use crate::filesystem::{FileSystem, FsDirectory, FsKind, make_filesystem};
use crate::model::{ErrorCode, ModelError, ModelResult};

const MAX_FAMILY_ENTRIES: usize = 64;
const MAX_FAMILY_BYTES: u64 = 1024 * 1024;

pub(super) struct FamilyFile {
    pub(super) name: OsString,
    pub(super) identity: ObjectIdentity,
}

pub(super) struct FamilyResidue {
    pub(super) authority: Option<CheckedArtifactAuthority>,
    pub(super) source: Option<FamilyFile>,
    pub(super) goal: Option<FamilyFile>,
    pub(super) foreign: bool,
}

impl FamilyResidue {
    fn empty() -> Self {
        Self {
            authority: None,
            source: None,
            goal: None,
            foreign: false,
        }
    }
}

impl CheckedArtifact {
    pub(super) fn open_private(&self, create: bool) -> ModelResult<Option<FsDirectory>> {
        if create {
            Self::prepare_parent(
                &self.private_root,
                &self.quarantine_parent,
                self.code,
                &self.label,
            )?;
        }
        let filesystem = make_filesystem();
        let root = filesystem
            .open_directory(&self.private_root)
            .map_err(|cause| {
                io_op_error(self.code, &self.label, "open ambient private root", cause)
            })?;
        let mut current = root;
        for component in self.quarantine_parent.components() {
            let Component::Normal(component) = component else {
                return Err(error(
                    self.code,
                    &self.label,
                    "private recovery path is noncanonical",
                ));
            };
            current = match filesystem.open_directory_at(&current, component) {
                Ok(dir) => dir,
                Err(cause) if cause.kind() == std::io::ErrorKind::NotFound && !create => {
                    return Ok(None);
                }
                Err(cause) => {
                    return Err(io_op_error(
                        self.code,
                        &self.label,
                        "open private component no-follow",
                        cause,
                    ));
                }
            };
        }
        let ParentState::Open { dir: parent, .. } = &self.parent else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid",
            ));
        };
        let managed_domain = identity::filesystem_rename_domain(parent)
            .map_err(|cause| unsupported(&self.label, "managed parent rename domain", cause))?;
        let private_domain = identity::filesystem_rename_domain(&current)
            .map_err(|cause| unsupported(&self.label, "private parent rename domain", cause))?;
        if managed_domain != private_domain {
            return Err(ModelError::new(
                ErrorCode::UnsupportedOperation,
                format!(
                    "checked {}: managed and private parents are not in one atomic rename domain",
                    self.label
                ),
            ));
        }
        super::platform::prepare_filesystem_private(&current, create, self.code, &self.label)?;
        Ok(Some(current))
    }

    pub(super) fn inspect_family(
        &self,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
    ) -> ModelResult<FamilyResidue> {
        let Some(dir) = self.open_private(false)? else {
            return Ok(FamilyResidue::empty());
        };
        let family = self.family_key();
        let action = self.action_key(expected, goal);
        let prefix = family_prefix(&family);
        let expected_authority_name = authority_name(&family, &action);
        let mut names = Vec::new();
        let mut total_bytes = 0_u64;
        let filesystem = make_filesystem();
        for entry in filesystem.read_directory_at(&dir).map_err(|cause| {
            io_op_error(self.code, &self.label, "list private family entries", cause)
        })? {
            let name = entry.name;
            if !name.to_string_lossy().starts_with(&prefix) {
                continue;
            }
            // [R2-E E7 Code F3] The family byte budget charges at STAT level,
            // before any read. It used to charge `bytes.len()` of an
            // already-read allocation, so a large foreign object under a
            // `ca1-` name was fully read before the 1 MiB intent could
            // classify it foreign. `DirEntry::metadata` is the same no-follow
            // stat `observe_leaf_exact` opens against, so the two agree on
            // what is being weighed.
            //
            // Residual, stated: an object that grows after this stat is still
            // read, bounded by its own post-open `fstat` (anchor nit 1's
            // cure), and caught by the len-mismatch arm — so the budget bounds
            // the survey, not a concurrent writer.
            if entry.kind == FsKind::File {
                let file = filesystem.open_file_at(&dir, &name).map_err(|cause| {
                    io_op_error(self.code, &self.label, "stat private family entry", cause)
                })?;
                total_bytes =
                    total_bytes.saturating_add(filesystem.file_len(&file).map_err(|cause| {
                        io_op_error(self.code, &self.label, "stat private family entry", cause)
                    })?);
            }
            names.push(name);
            if names.len() > MAX_FAMILY_ENTRIES || total_bytes > MAX_FAMILY_BYTES {
                return Ok(FamilyResidue {
                    foreign: true,
                    ..FamilyResidue::empty()
                });
            }
        }
        names.sort();

        let mut authority = None;
        let mut source = None;
        let mut staged_goal = None;
        let mut foreign = false;
        for name in names {
            let observed = observe_leaf_exact(&filesystem, &dir, &name, self.code, &self.label)?;
            let Some(text) = name.to_str() else {
                foreign = true;
                continue;
            };
            if text == expected_authority_name {
                let CheckedArtifactFact::Bytes(bytes) = observed.fact else {
                    foreign = true;
                    continue;
                };
                if authority.is_some() {
                    foreign = true;
                    continue;
                }
                authority = CheckedArtifactAuthority::decode(&bytes);
                foreign |= authority.is_none();
                continue;
            }
            if text.ends_with(".authority") {
                foreign = true;
                continue;
            }
            let Some(identity) = observed.identity else {
                foreign = true;
                continue;
            };
            let identity_digest = identity.name_digest();
            if text == goal_name(&family, &action, &identity_digest) {
                if staged_goal.is_some()
                    || goal.is_none_or(|bytes| {
                        observed.fact != CheckedArtifactFact::Bytes(bytes.to_vec())
                    })
                {
                    foreign = true;
                    continue;
                }
                staged_goal = Some(FamilyFile { name, identity });
            } else if text == source_name(&family, &action, &identity_digest) {
                if source.is_some() || observed.fact != *expected {
                    foreign = true;
                    continue;
                }
                source = Some(FamilyFile { name, identity });
            } else {
                foreign = true;
            }
        }

        if let Some(authority) = &authority {
            if !authority.matches_request(self, expected, goal) {
                foreign = true;
            }
            match (&authority.retained_source, &source) {
                (RetainedSource::Missing, Some(_)) => foreign = true,
                (RetainedSource::Existing(expected), Some(observed))
                    if *expected != observed.identity.durable =>
                {
                    foreign = true;
                }
                _ => {}
            }
        } else if source.is_some() || staged_goal.is_some() {
            foreign = true;
        }

        Ok(FamilyResidue {
            authority,
            source,
            goal: staged_goal,
            foreign,
        })
    }

    pub(super) fn ensure_authority(
        &self,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
        source: &LeafObservation,
    ) -> ModelResult<CheckedArtifactAuthority> {
        let dir = self.open_private(true)?.expect("private directory created");
        let prior = self.inspect_family(expected, goal)?;
        if prior.foreign {
            return Err(error(
                self.code,
                &self.label,
                "foreign family state prevents authority publication",
            ));
        }
        if let Some(authority) = prior.authority {
            let name = authority_name(&authority.family_key, &authority.action_key);
            self.rebarrier_exact(&dir, OsStr::new(&name))?;
            let verified = self.inspect_family(expected, goal)?;
            if verified.foreign || verified.authority.as_ref() != Some(&authority) {
                return Err(error(
                    self.code,
                    &self.label,
                    "retained authority changed while re-establishing durability",
                ));
            }
            return Ok(authority);
        }
        if prior.source.is_some() || prior.goal.is_some() {
            return Err(error(
                self.code,
                &self.label,
                "family residue exists without action authority",
            ));
        }
        let ParentState::Open {
            identity: parent_identity,
            ..
        } = &self.parent
        else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid",
            ));
        };
        let current_source = self.observe_leaf_exact_current()?;
        if !self.parent_is_current(parent_identity)?
            || source.fact != *expected
            || current_source.fact != source.fact
            || current_source.identity != source.identity
        {
            return Err(error(
                self.code,
                &self.label,
                "source changed before authority publication",
            ));
        }
        let retained_source = match expected {
            CheckedArtifactFact::Missing if source.identity.is_none() => RetainedSource::Missing,
            CheckedArtifactFact::Bytes(_) => RetainedSource::Existing(
                source
                    .identity
                    .as_ref()
                    .ok_or_else(|| error(self.code, &self.label, "exact source lacks identity"))?
                    .durable
                    .clone(),
            ),
            CheckedArtifactFact::Missing | CheckedArtifactFact::Invalid => {
                return Err(error(
                    self.code,
                    &self.label,
                    "source cannot authorize checked-artifact mutation",
                ));
            }
        };
        let authority = CheckedArtifactAuthority::for_source(
            self,
            expected,
            goal,
            parent_identity.durable.clone(),
            retained_source,
        )
        .ok_or_else(|| error(self.code, &self.label, "authority request is invalid"))?;
        let bytes = authority
            .encode()
            .ok_or_else(|| error(self.code, &self.label, "authority record exceeds bounds"))?;
        let name = authority_name(&authority.family_key, &authority.action_key);
        let scratch = scratch_name(&authority.family_key, &authority.action_key, "authority");
        self.publish_scratch(&dir, &scratch, &name, &bytes)?;
        self.rebarrier_exact(&dir, OsStr::new(&name))?;
        let after = self.inspect_family(expected, goal)?;
        if after.foreign || after.authority.as_ref() != Some(&authority) {
            return Err(error(
                self.code,
                &self.label,
                "published authority failed exact verification",
            ));
        }
        Ok(authority)
    }

    pub(super) fn ensure_goal(
        &self,
        authority: &CheckedArtifactAuthority,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<FamilyFile> {
        let dir = self.open_private(true)?.expect("private directory created");
        let prior = self.inspect_family(expected, Some(goal))?;
        if prior.foreign || prior.authority.as_ref() != Some(authority) {
            return Err(error(
                self.code,
                &self.label,
                "authority changed before goal staging",
            ));
        }
        if let Some(goal) = prior.goal {
            self.rebarrier_exact(&dir, &goal.name)?;
            return Ok(goal);
        }
        let scratch = scratch_name(&authority.family_key, &authority.action_key, "goal");
        fault(
            CheckedArtifactFault::BeforeGoalScratchCreate,
            self.code,
            &self.label,
        )?;
        let filesystem = make_filesystem();
        let file = self.open_staging(&filesystem, &dir, OsStr::new(&scratch))?;
        fault(
            CheckedArtifactFault::AfterGoalScratchCreate,
            self.code,
            &self.label,
        )?;
        filesystem.write_all(&file, goal).map_err(|cause| {
            io_op_error(self.code, &self.label, "write goal scratch bytes", cause)
        })?;
        fault(
            CheckedArtifactFault::AfterGoalScratchWrite,
            self.code,
            &self.label,
        )?;
        filesystem.sync_file(&file).map_err(|cause| {
            io_op_error(self.code, &self.label, "sync goal scratch file", cause)
        })?;
        fault(
            CheckedArtifactFault::AfterGoalScratchFlush,
            self.code,
            &self.label,
        )?;
        let identity = identity::filesystem_file_identity(&file)
            .map_err(|cause| unsupported(&self.label, "staged goal identity", cause))?;
        drop(file);
        let name = goal_name(
            &authority.family_key,
            &authority.action_key,
            &identity.name_digest(),
        );
        fault(
            CheckedArtifactFault::BeforeGoalPublication,
            self.code,
            &self.label,
        )?;
        super::platform::publish_verified_filesystem_leaf_no_replace(
            &dir,
            OsStr::new(&scratch),
            &dir,
            OsStr::new(&name),
            &super::platform::LeafPublicationSourceV1 {
                identity: &identity,
                bytes: goal,
            },
            self.code,
            &self.label,
        )?;
        fault(
            CheckedArtifactFault::AfterGoalPublication,
            self.code,
            &self.label,
        )?;
        super::platform::filesystem_private_barrier(
            &dir,
            super::platform::DirentBarrierClass::AnchoredPrivateArea,
            self.code,
            &self.label,
        )?;
        fault(
            CheckedArtifactFault::AfterGoalParentBarrier,
            self.code,
            &self.label,
        )?;
        self.rebarrier_exact(&dir, OsStr::new(&name))?;
        let observed =
            observe_leaf_exact(&filesystem, &dir, OsStr::new(&name), self.code, &self.label)?;
        if observed.fact != CheckedArtifactFact::Bytes(goal.to_vec())
            || observed.identity.as_ref() != Some(&identity)
        {
            return Err(error(
                self.code,
                &self.label,
                "staged goal failed exact identity verification",
            ));
        }
        Ok(FamilyFile {
            name: name.into(),
            identity,
        })
    }

    fn publish_scratch(
        &self,
        dir: &FsDirectory,
        scratch: &str,
        name: &str,
        bytes: &[u8],
    ) -> ModelResult<()> {
        fault(
            CheckedArtifactFault::BeforeAuthorityScratchCreate,
            self.code,
            &self.label,
        )?;
        let filesystem = make_filesystem();
        let file = self.open_staging(&filesystem, dir, OsStr::new(scratch))?;
        fault(
            CheckedArtifactFault::AfterAuthorityScratchCreate,
            self.code,
            &self.label,
        )?;
        filesystem.write_all(&file, bytes).map_err(|cause| {
            io_op_error(
                self.code,
                &self.label,
                "write authority scratch bytes",
                cause,
            )
        })?;
        fault(
            CheckedArtifactFault::AfterAuthorityScratchWrite,
            self.code,
            &self.label,
        )?;
        filesystem.sync_file(&file).map_err(|cause| {
            io_op_error(self.code, &self.label, "sync authority scratch file", cause)
        })?;
        fault(
            CheckedArtifactFault::AfterAuthorityScratchFlush,
            self.code,
            &self.label,
        )?;
        // The scratch's durable identity is taken from the write handle, exactly
        // as `ensure_goal` above takes the staged goal's, so the sealed
        // publication below can re-verify the object it renames. It adds no
        // platform requirement: every drive that reaches this point already
        // fails without `identity::file_identity` — through `inspect_family`'s
        // reobservation of the record it just published, or through the goal
        // staging that follows it.
        let identity = identity::filesystem_file_identity(&file)
            .map_err(|cause| unsupported(&self.label, "authority record identity", cause))?;
        drop(file);
        fault(
            CheckedArtifactFault::BeforeAuthorityPublication,
            self.code,
            &self.label,
        )?;
        super::platform::publish_verified_filesystem_leaf_no_replace(
            dir,
            OsStr::new(&scratch),
            dir,
            OsStr::new(name),
            &super::platform::LeafPublicationSourceV1 {
                identity: &identity,
                bytes,
            },
            self.code,
            &self.label,
        )?;
        fault(
            CheckedArtifactFault::AfterAuthorityPublication,
            self.code,
            &self.label,
        )?;
        super::platform::filesystem_private_barrier(
            dir,
            super::platform::DirentBarrierClass::AnchoredPrivateArea,
            self.code,
            &self.label,
        )?;
        fault(
            CheckedArtifactFault::AfterAuthorityParentBarrier,
            self.code,
            &self.label,
        )
    }

    /// Open the deterministic staging name for writing, fresh or on resume.
    ///
    /// A fresh drive opens `create_new`, so a racing creator fails the open
    /// closed exactly as it did under the random name. A resume — now possible,
    /// because the name is derivable rather than lost with the crashed process —
    /// truncates the *same* name instead of allocating beside it. Both are
    /// write-ahead staging with no committed meaning: nothing durable moves until
    /// the sealed publication re-verifies identity and bytes through the handle
    /// it renames, so a truncate that meets a foreign object at the now-derivable
    /// name still cannot publish that object's content.
    fn open_staging(
        &self,
        filesystem: &impl FileSystem,
        dir: &FsDirectory,
        scratch: &OsStr,
    ) -> ModelResult<crate::filesystem::FsFile> {
        let resident = match filesystem.metadata_at(dir, scratch) {
            Ok(_) => true,
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => false,
            Err(cause) => {
                return Err(io_op_error(
                    self.code,
                    &self.label,
                    "read staging scratch metadata",
                    cause,
                ));
            }
        };
        if resident {
            let file = filesystem
                .open_file_for_write_at(dir, scratch)
                .map_err(|cause| {
                    io_op_error(self.code, &self.label, "open staging scratch file", cause)
                })?;
            filesystem.set_len(&file, 0).map_err(|cause| {
                io_op_error(
                    self.code,
                    &self.label,
                    "truncate staging scratch file",
                    cause,
                )
            })?;
            Ok(file)
        } else {
            filesystem.create_file_at(dir, scratch).map_err(|cause| {
                io_op_error(self.code, &self.label, "create staging scratch file", cause)
            })
        }
    }

    pub(super) fn rebarrier_exact(&self, dir: &FsDirectory, name: &OsStr) -> ModelResult<()> {
        let filesystem = make_filesystem();
        let before = observe_leaf_exact(&filesystem, dir, name, self.code, &self.label)?;
        let file = filesystem.open_file_at(dir, name).map_err(|cause| {
            io_op_error(
                self.code,
                &self.label,
                "reopen family entry no-follow",
                cause,
            )
        })?;
        // Unix re-barriers the entry through its file handle; Windows cannot
        // FlushFileBuffers a read-access handle (os error 5) and its
        // durability model is write-through at write time plus the anchor
        // barrier issued just below, so the per-file re-sync is Unix-only
        // (W3, GwzWindowsMatrix-Classification.md).
        #[cfg(not(windows))]
        filesystem
            .sync_file(&file)
            .map_err(|cause| io_op_error(self.code, &self.label, "sync family entry", cause))?;
        drop(file);
        super::platform::filesystem_private_barrier(
            dir,
            super::platform::DirentBarrierClass::AnchoredPrivateArea,
            self.code,
            &self.label,
        )?;
        let after = observe_leaf_exact(&filesystem, dir, name, self.code, &self.label)?;
        if before.fact != after.fact || before.identity != after.identity {
            return Err(error(
                self.code,
                &self.label,
                "family entry changed while re-establishing durability",
            ));
        }
        Ok(())
    }
}

fn unsupported(label: &str, fact: &str, cause: std::io::Error) -> ModelError {
    ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("checked {label}: {fact} is unsupported: {cause}"),
    )
}
