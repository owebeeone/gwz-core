use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use cap_fs_ext::OsMetadataExt;
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, ambient_authority};
use cap_std::fs::{Dir, OpenOptions};

use super::identity::{self, ObjectIdentity};
use super::{CheckedArtifact, CheckedArtifactFact, CheckedArtifactPolicy, ParentState, error};
use crate::model::{ErrorCode, ModelError, ModelResult};

impl CheckedArtifact {
    /// Prepare a canonical no-follow directory hierarchy before an operation
    /// can persist an action that depends on it.
    pub(super) fn prepare_parent(
        root: &Path,
        relative: &Path,
        code: ErrorCode,
        label: impl Into<String>,
    ) -> ModelResult<()> {
        let label = label.into();
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(error(code, &label, "parent path is noncanonical"));
        }
        let mut current = Dir::open_ambient_dir(root, ambient_authority()).map_err(|cause| {
            io_op_error(code, &label, "open root for parent preparation", cause)
        })?;
        for component in relative.components() {
            let Component::Normal(component) = component else {
                return Err(error(code, &label, "parent path is noncanonical"));
            };
            let metadata = match current.symlink_metadata(component) {
                Ok(metadata) => metadata,
                Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                    current.create_dir(component).map_err(|cause| {
                        io_op_error(code, &label, "create parent component", cause)
                    })?;
                    super::platform::sync_parent(&current).map_err(|cause| {
                        io_op_error(code, &label, "sync parent after creating component", cause)
                    })?;
                    current.symlink_metadata(component).map_err(|cause| {
                        io_op_error(code, &label, "reread created component metadata", cause)
                    })?
                }
                Err(cause) => {
                    return Err(io_op_error(
                        code,
                        &label,
                        "read parent component metadata",
                        cause,
                    ));
                }
            };
            if !metadata.is_dir() || metadata.is_symlink() {
                return Err(error(code, &label, "parent component is noncanonical"));
            }
            let next = current.open_dir_nofollow(component).map_err(|cause| {
                io_op_error(code, &label, "open parent component no-follow", cause)
            })?;
            if metadata_identity(&metadata)
                != metadata_identity(&next.dir_metadata().map_err(|cause| {
                    io_op_error(code, &label, "stat opened parent component", cause)
                })?)
            {
                return Err(error(
                    code,
                    &label,
                    "parent component changed while opening",
                ));
            }
            current = next;
        }
        Ok(())
    }

    pub(super) fn acquire(
        policy: CheckedArtifactPolicy,
        relative: &Path,
        code: ErrorCode,
        label: impl Into<String>,
    ) -> ModelResult<Self> {
        Self::acquire_with_escape(policy, relative, code, label, IdentityGapEscape::Substrate)
    }

    /// [`Self::acquire`], with the sentence a HANDLE-PROBE refusal renders
    /// chosen by the caller (`GwzM5-8M5d-Charter.md` §3(b), 2026-09-03).
    ///
    /// Only the door knows which escape is true for it, and on a handle-fail
    /// volume the substrate's own remedy is CIRCULAR for a reverse door: it
    /// advertises `gwz merge --abort`, which is the very door that just
    /// refused. `entry.rs`'s four reverse helpers pass
    /// [`IdentityGapEscape::ReverseMergeDoor`] instead; every other caller
    /// keeps today's rendering through [`Self::acquire`].
    ///
    /// The escape is not retained on the artifact because it cannot matter
    /// later: the FIRST thing this function does after opening the root is
    /// probe it, so a volume that refuses handles never yields a
    /// `CheckedArtifact` at all, and `parent_is_current`'s own probe below is
    /// unreachable there.
    pub(super) fn acquire_with_escape(
        policy: CheckedArtifactPolicy,
        relative: &Path,
        code: ErrorCode,
        label: impl Into<String>,
        escape: IdentityGapEscape,
    ) -> ModelResult<Self> {
        let label = label.into();
        let relative = relative.to_path_buf();
        let (parent_relative, leaf) =
            split_relative(&relative).map_err(|detail| error(code, &label, detail))?;
        let private_root = policy.artifact_root().to_path_buf();
        let quarantine_parent = policy.private_parent();
        let root = Dir::open_ambient_dir(policy.artifact_root(), ambient_authority())
            .map_err(|cause| io_op_error(code, &label, "open ambient artifact root", cause))?;
        let root_identity = durable_identity(&root, &label, escape)?;
        let canonical_path_identity = identity::canonical_path_identity(&root, &relative)
            .map_err(|cause| unsupported(&label, cause))?;
        let parent = match traverse(&root, &parent_relative)
            .map_err(|cause| io_op_error(code, &label, "traverse to artifact parent", cause))?
        {
            Traversal::Missing => ParentState::Missing,
            Traversal::Invalid => ParentState::Invalid,
            Traversal::Open(dir) => {
                let identity = durable_identity(&dir, &label, escape)?;
                ParentState::Open { dir, identity }
            }
        };
        Ok(Self {
            root,
            root_identity,
            canonical_path_identity,
            parent_relative,
            parent,
            leaf,
            private_root,
            quarantine_parent,
            code,
            label,
        })
    }

    pub(super) fn observe(&self) -> ModelResult<CheckedArtifactFact> {
        let ParentState::Open { dir, identity } = &self.parent else {
            return Ok(match self.parent {
                ParentState::Missing => CheckedArtifactFact::Missing,
                ParentState::Invalid => CheckedArtifactFact::Invalid,
                ParentState::Open { .. } => unreachable!(),
            });
        };
        if !self.parent_is_current(identity)? {
            return Ok(CheckedArtifactFact::Invalid);
        }
        observe_leaf(dir, &self.leaf, self.code, &self.label)
    }

    pub(super) fn observe_leaf_exact_current(&self) -> ModelResult<LeafObservation> {
        let ParentState::Open { dir, identity } = &self.parent else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid",
            ));
        };
        if !self.parent_is_current(identity)? {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent changed while observing artifact",
            ));
        }
        observe_leaf_exact(dir, &self.leaf, self.code, &self.label)
    }

    pub(super) fn parent_is_canonical(&self) -> ModelResult<bool> {
        match &self.parent {
            ParentState::Open { identity, .. } => self.parent_is_current(identity),
            ParentState::Missing | ParentState::Invalid => Ok(false),
        }
    }

    pub(super) fn parent_is_current(&self, expected: &ObjectIdentity) -> ModelResult<bool> {
        let current = traverse(&self.root, &self.parent_relative).map_err(|cause| {
            io_op_error(self.code, &self.label, "retraverse artifact parent", cause)
        })?;
        let Traversal::Open(current) = current else {
            return Ok(false);
        };
        let observed =
            identity::object_identity(&current).map_err(|cause| unsupported(&self.label, cause))?;
        Ok(observed == *expected)
    }
}

pub(super) fn observe_leaf(
    dir: &Dir,
    leaf: &OsStr,
    code: ErrorCode,
    label: &str,
) -> ModelResult<CheckedArtifactFact> {
    Ok(observe_leaf_exact(dir, leaf, code, label)?.fact)
}

pub(super) struct LeafObservation {
    pub(super) fact: CheckedArtifactFact,
    pub(super) identity: Option<ObjectIdentity>,
}

pub(super) fn observe_leaf_exact(
    dir: &Dir,
    leaf: &OsStr,
    code: ErrorCode,
    label: &str,
) -> ModelResult<LeafObservation> {
    let metadata = match dir.symlink_metadata(leaf) {
        Ok(metadata) => metadata,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LeafObservation {
                fact: CheckedArtifactFact::Missing,
                identity: None,
            });
        }
        Err(cause) => {
            return Err(io_op_error(
                code,
                label,
                "read artifact leaf metadata",
                cause,
            ));
        }
    };
    if !metadata.is_file() || metadata.is_symlink() || executable(&metadata) {
        return Ok(LeafObservation {
            fact: CheckedArtifactFact::Invalid,
            identity: None,
        });
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = match dir.open_with(leaf, &options) {
        Ok(file) => file,
        Err(cause)
            if matches!(
                cause.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(LeafObservation {
                fact: CheckedArtifactFact::Invalid,
                identity: None,
            });
        }
        Err(cause) => return Err(io_op_error(code, label, "open artifact no-follow", cause)),
    };
    let opened = file
        .metadata()
        .map_err(|cause| io_op_error(code, label, "read opened artifact metadata", cause))?;
    if metadata_identity(&opened) != metadata_identity(&metadata) {
        return Ok(LeafObservation {
            fact: CheckedArtifactFact::Invalid,
            identity: None,
        });
    }
    // Anchor nit 1, the E7 dual's Q1 shape (`GwzM5-8R2E-E7-Acceptance.md` §4's
    // O12 row; template at `platform.rs`'s sealed publication verifier): the
    // read is bounded by the reader's OWN already-identity-checked `fstat`
    // above, not by a family constant — this reader serves fixed-expected
    // verifiers and arbitrary user-artifact content reads alike, and a constant
    // would refuse legitimate user files. What it cures is the infallible
    // geometric growth of a bare `read_to_end`: an object growing under the
    // read now costs one bounded reservation and a typed refusal instead of an
    // allocation abort.
    //
    // The bound is this reader's own and is NOT INHERITED by its callers: a
    // caller that needs a tighter budget must impose it before the read, which
    // is what `residue.rs`'s family survey now does at stat level.
    //
    // Accepted residual, stated: a stable multi-GB foreign object still
    // reserves its stat size fallibly, and is refused typed only if the
    // reservation fails.
    let bound = opened.len().saturating_add(1);
    let Ok(capacity) = usize::try_from(bound) else {
        // A leaf larger than this address space is not a canonical artifact.
        return Ok(LeafObservation {
            fact: CheckedArtifactFact::Invalid,
            identity: None,
        });
    };
    let mut bytes = Vec::new();
    // `io_op_error`'s rendering is `"{operation}: {cause}"` and an allocation
    // refusal has no `io::Error`, so the same sentence is built with its
    // sibling constructor at the same `ErrorCode`. Never an abort.
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| error(code, label, "read artifact bytes: allocation refused"))?;
    // Over-read by exactly one byte so a leaf that grew past its `fstat` fails
    // the existing five-way check below (`opened.len() != bytes.len()`) and is
    // reported `Invalid` — today's arm, kept; no new refusal vocabulary.
    file.by_ref()
        .take(bound)
        .read_to_end(&mut bytes)
        .map_err(|cause| io_op_error(code, label, "read artifact bytes", cause))?;
    let after = match dir.symlink_metadata(leaf) {
        Ok(after) => after,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LeafObservation {
                fact: CheckedArtifactFact::Invalid,
                identity: None,
            });
        }
        Err(cause) => {
            return Err(io_op_error(
                code,
                label,
                "reread artifact leaf metadata",
                cause,
            ));
        }
    };
    if !after.is_file()
        || after.is_symlink()
        || executable(&after)
        || metadata_identity(&after) != metadata_identity(&metadata)
        || metadata_identity(&after) != metadata_identity(&opened)
        || opened.len() != bytes.len() as u64
    {
        return Ok(LeafObservation {
            fact: CheckedArtifactFact::Invalid,
            identity: None,
        });
    }
    Ok(LeafObservation {
        fact: CheckedArtifactFact::Bytes(bytes),
        identity: Some(identity::file_identity(&file).map_err(|cause| unsupported(label, cause))?),
    })
}

fn metadata_identity(metadata: &cap_fs_ext::Metadata) -> (u64, u64) {
    (MetadataExt::dev(metadata), MetadataExt::ino(metadata))
}

enum Traversal {
    Missing,
    Invalid,
    Open(Dir),
}

fn traverse(root: &Dir, relative: &Path) -> std::io::Result<Traversal> {
    let mut current = root.try_clone()?;
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Ok(Traversal::Invalid);
        };
        let metadata = match current.symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Traversal::Missing);
            }
            Err(cause) => return Err(cause),
        };
        if !metadata.is_dir() || metadata.is_symlink() {
            return Ok(Traversal::Invalid);
        }
        let next = match current.open_dir_nofollow(component) {
            Ok(next) => next,
            Err(cause) if cause.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(cause);
            }
            Err(_) => return Ok(Traversal::Invalid),
        };
        if metadata_identity(&metadata) != metadata_identity(&next.dir_metadata()?) {
            return Ok(Traversal::Invalid);
        }
        current = next;
    }
    Ok(Traversal::Open(current))
}

/// Which sentence a refusal of the LEGACY persistent-handle probe renders.
///
/// M5d step (3) (`GwzM5-8M5d-Charter.md` §3(b), 2026-09-03). The probe is the
/// same either way; only the escape offered differs, because the escape that
/// is true for a door depends on which door it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IdentityGapEscape {
    /// The substrate's own remedy, verbatim — today's rendering, kept for
    /// every door but the four reverse merge doors.
    Substrate,
    /// A reverse merge door (a selected root's artifacts, a preservation
    /// bundle, published evidence) on a volume without persistent handles.
    ReverseMergeDoor,
}

/// The create door's own handle probe, applied to one directory.
///
/// M5d step (3) (`GwzM5-8M5d-Charter.md` §3, "Where handle capability is
/// learned"). `crash_recovery_decision` runs THIS against the workspace root —
/// the directory whose identity [`CheckedArtifact::acquire`] takes first, and
/// which exists before any write — so the decision point learns handle
/// capability at the decision point rather than meeting it later at the door.
/// It is deliberately not applied to `.gwz`: a first merge has no `.gwz` yet,
/// and a missing private directory is not a filesystem capability gap
/// (charter revision 5, S-P2-3).
///
/// Read-only and total: it opens a directory and asks the host one question,
/// creating nothing. Any failure — the open, or the probe — answers `false`,
/// because every one of them means the door cannot bind this directory's
/// durable identity.
pub(super) fn directory_handles_ok(directory: &Path) -> bool {
    Dir::open_ambient_dir(directory, ambient_authority())
        .is_ok_and(|dir| identity::object_identity(&dir).is_ok())
}

fn durable_identity(
    dir: &Dir,
    label: &str,
    escape: IdentityGapEscape,
) -> ModelResult<ObjectIdentity> {
    identity::object_identity(dir).map_err(|cause| match escape {
        IdentityGapEscape::Substrate => unsupported(label, cause),
        IdentityGapEscape::ReverseMergeDoor => reverse_door_unsupported(label),
    })
}

pub(super) fn io_op_error(
    code: ErrorCode,
    label: &str,
    operation: &'static str,
    cause: std::io::Error,
) -> ModelError {
    error(code, label, format_args!("{operation}: {cause}"))
}

fn unsupported(label: &str, cause: std::io::Error) -> ModelError {
    ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("checked {label}: durable filesystem identity is unsupported: {cause}"),
    )
}

/// The refusal a REVERSE merge door renders on a handle-fail volume
/// (`GwzM5-8M5d-Charter.md` §3(b), 2026-09-03).
///
/// Not [`unsupported`]: that renders the substrate's own remedy, which names
/// `gwz merge --abort` as the escape — circular here, because this IS that
/// door refusing. The cause is not interpolated either; it is always the same
/// gap (this probe has exactly one failure meaning) and the charter asks for
/// ONE escape stated plainly, not an `errno` the user cannot act on.
fn reverse_door_unsupported(label: &str) -> ModelError {
    ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!(
            "checked {label}: {}",
            super::capability::HANDLE_FAIL_REVERSE_DOOR_ESCAPE
        ),
    )
}

fn split_relative(path: &Path) -> Result<(PathBuf, OsString), &'static str> {
    if path.is_absolute() {
        return Err("path is not workspace-relative");
    }
    let mut components = path.components().collect::<Vec<_>>();
    let leaf = match components.pop() {
        Some(Component::Normal(leaf)) => leaf.to_owned(),
        _ => return Err("leaf name is noncanonical"),
    };
    let mut parent = PathBuf::new();
    for component in components {
        let Component::Normal(component) = component else {
            return Err("parent path is noncanonical");
        };
        parent.push(component);
    }
    Ok((parent, leaf))
}

#[cfg(unix)]
fn executable(metadata: &cap_fs_ext::Metadata) -> bool {
    OsMetadataExt::mode(metadata) & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &cap_fs_ext::Metadata) -> bool {
    false
}
