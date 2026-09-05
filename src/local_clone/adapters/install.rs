//! `gwz_workspace_install::InstallPorts` over the real libraries and the
//! existing GWZ helpers: the inspector (`gwz-repo-inspect`) behind the
//! source snapshot, the source recheck and the destination observation; the
//! store (`gwz-family-store`) behind the destination's family metadata
//! reading; the conf-integrity helpers (`crate::artifact`) behind recapture
//! and manifest publication; [`super::git_config`] behind the destination's
//! Git configuration; and `gwz-history-check` behind the destination's
//! object-connectivity check (design §4.0 dest-complete).
//!
//! The snapshot is taken **before** the family lock (`preflight`), so a
//! source that design §4.0 refuses is refused with nothing written -- no
//! lock file, no index, no row -- and installation's own `snapshot_source`
//! call returns that capture; `recheck_source` re-inventories the source
//! before publication, which is what makes taking it early safe.
//!
//! Clean and bare modes are not composed here: `construct_repositories`
//! answers `Unimplemented` (LCM2.3 / LCM3.1), and `recapture_configuration`
//! and `publish_manifest` know only the verbatim shape.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gwz_copy_contract::Exclusion;
use gwz_family_model::{AllocationId, CloneMode, FamilyId, PointerObservation};
use gwz_family_store::{WorkspaceObservation, YamlFamilyStore};
use gwz_history_check::{HistoryOutcome, Limits, NeverCancelled, Witness, check_history};
use gwz_repo_contract::{HeadState, Observation, RepoInspector};
use gwz_repo_inspect::{LocalObjectReader, LocalRepoInspector};
use gwz_workspace_install::{
    ConfigurationPlan, ConfigurationReport, ConstructionRequest, DestinationObservation,
    GitInstallReport, InstallPortError, InstallPorts, ManifestReceipt, SourceSnapshot,
};

use super::exclusions::{FIXED_EXCLUSIONS, verbatim_exclusions, worktrees_of};
use super::git_config::{DestinationRepository, install_destination_git};
use super::inventory::{IncludedRepository, included_repositories, recheck, snapshot};
use crate::artifact::{self, ConfIntegrityVerdict, ManifestArtifact};
use crate::model::ModelResult;
use crate::workspace::{RUNTIME_DIR, WORKSPACE_MANIFEST};

/// Whether a workspace has an open gwz merge (design §4.1, "refuse verbatim
/// while source has an open gwz merge"): `Some(merge_id)` when one is open.
/// Supplied by the dispatch slot, which can name the merge store's own
/// classifier; the adapter never decodes a record.
pub type OpenMergeProbe = fn(&Path) -> ModelResult<Option<String>>;

/// The real install ports for one create.
pub struct CoreInstallPorts {
    inspector: LocalRepoInspector,
    store: YamlFamilyStore,
    /// The family root, canonical.
    root: PathBuf,
    family_id: FamilyId,
    allocation: AllocationId,
    /// The source workspace, canonical.
    source: PathBuf,
    open_merge: OpenMergeProbe,
    repositories: Vec<IncludedRepository>,
    exclusions: Vec<Exclusion>,
    snapshot: Option<SourceSnapshot>,
    manifest: Option<ManifestArtifact>,
}

impl CoreInstallPorts {
    pub fn new(
        root: PathBuf,
        family_id: FamilyId,
        allocation: AllocationId,
        source: PathBuf,
        open_merge: OpenMergeProbe,
    ) -> Self {
        Self {
            inspector: LocalRepoInspector::new(),
            store: YamlFamilyStore::new(),
            root,
            family_id,
            allocation,
            source,
            open_merge,
            repositories: Vec::new(),
            exclusions: Vec::new(),
            snapshot: None,
            manifest: None,
        }
    }

    /// Inventory and capture the source once, before any family file
    /// exists. A design §4.0 hazard refuses here as
    /// [`InstallPortError::Layout`].
    pub fn preflight(&mut self) -> Result<&SourceSnapshot, InstallPortError> {
        let (repositories, exclusions, snapshot) = self.capture()?;
        self.manifest = Some(artifact::read_manifest(&self.source).map_err(|error| {
            InstallPortError::Configuration {
                detail: format!("source manifest: {}", error.message),
            }
        })?);
        self.repositories = repositories;
        self.exclusions = exclusions;
        self.snapshot = Some(snapshot);
        Ok(self.snapshot.as_ref().expect("just captured"))
    }

    /// Design §4.1's exclusion set for this source, known after
    /// [`preflight`](Self::preflight).
    pub fn exclusions(&self) -> &[Exclusion] {
        &self.exclusions
    }

    /// The included repositories of the source, root first.
    pub fn repositories(&self) -> &[IncludedRepository] {
        &self.repositories
    }

    fn capture(
        &self,
    ) -> Result<(Vec<IncludedRepository>, Vec<Exclusion>, SourceSnapshot), InstallPortError> {
        let fixed: Vec<Exclusion> = FIXED_EXCLUSIONS
            .iter()
            .map(|fixed| Exclusion::RelativePath(PathBuf::from(fixed)))
            .collect();
        let repositories = included_repositories(&self.source, &fixed).map_err(|detail| {
            InstallPortError::Layout(gwz_repo_contract::LayoutError::ReadFailed {
                path: self.source.clone(),
                detail,
            })
        })?;
        let git_dirs: Vec<PathBuf> = repositories
            .iter()
            .map(IncludedRepository::relative_git_dir)
            .collect();
        let exclusions = verbatim_exclusions(git_dirs.iter().map(PathBuf::as_path));
        let open_merge =
            (self.open_merge)(&self.source).map_err(|error| InstallPortError::Configuration {
                detail: format!("source merge store: {}", error.message),
            })?;
        let snapshot = snapshot(&self.inspector, &self.source, &repositories, open_merge)?;
        Ok((repositories, exclusions, snapshot))
    }

    fn destination_repositories(&self) -> Vec<DestinationRepository> {
        self.repositories
            .iter()
            .map(|repository| DestinationRepository {
                label: repository.label(),
                relative: repository.relative.clone(),
            })
            .collect()
    }

    /// Design §4.0 dest-complete for every repository that stands at the
    /// destination: an admitted layout (no external common dir, alternates,
    /// escaping link or configuration), HEAD at the frozen source HEAD, and
    /// every protected root's object graph complete in the destination's
    /// own store -- one `check_history` call per repository, with the
    /// repository as its own witness.
    fn dependencies(&self, destination: &Path) -> Vec<String> {
        let mut details = Vec::new();
        for repository in &self.repositories {
            let path = destination.join(&repository.relative);
            if fs::symlink_metadata(path.join(".git")).is_err() {
                details.push(format!(
                    "{}: no repository at {}",
                    repository.key,
                    path.display()
                ));
                continue;
            }
            let info = match self.inspector.inspect_layout(&path) {
                Ok(info) => info,
                Err(error) => {
                    details.push(format!("{}: {error}", repository.key));
                    continue;
                }
            };
            let frozen = self
                .snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .repositories
                        .iter()
                        .find(|captured| captured.key == repository.key)
                })
                .map(|captured| captured.head.clone());
            let observed = match &info.head {
                HeadState::Attached { target, .. } | HeadState::Detached { target } => {
                    Some(target.clone())
                }
                HeadState::Unborn { .. } => None,
            };
            if let Some(frozen) = frozen
                && frozen != observed
            {
                details.push(format!(
                    "{}: HEAD {} is not the frozen source HEAD {}",
                    repository.key,
                    observed.map_or_else(|| "unborn".to_owned(), |oid| oid.to_hex()),
                    frozen.map_or_else(|| "unborn".to_owned(), |oid| oid.to_hex()),
                ));
            }
            let protected = match self.inspector.inventory_history(&info) {
                Observation::Known(protected) if protected.is_complete() => protected,
                Observation::Known(protected) => {
                    details.push(format!(
                        "{}: the protected-root inventory is incomplete: {:?}",
                        repository.key, protected.unknown
                    ));
                    continue;
                }
                Observation::Unknown(reasons) => {
                    details.push(format!(
                        "{}: the protected-root inventory is unknown: {reasons:?}",
                        repository.key
                    ));
                    continue;
                }
            };
            let reader = LocalObjectReader::open(&info);
            let witness = Witness {
                repository: repository.key.clone(),
                label: path.display().to_string(),
            };
            match check_history(
                &protected,
                &[witness],
                &reader,
                Limits::default(),
                &NeverCancelled,
            ) {
                HistoryOutcome::Verified(_) => {}
                HistoryOutcome::Unpreserved(items) => {
                    let missing: Vec<String> = items
                        .iter()
                        .map(|item| {
                            item.missing.as_ref().map_or_else(
                                || format!("{:?} at {}", item.root.source, item.root.oid),
                                |oid| format!("{oid} (below {})", item.root.oid),
                            )
                        })
                        .collect();
                    details.push(format!(
                        "{}: objects missing from the destination store: {}",
                        repository.key,
                        missing.join(", ")
                    ));
                }
                HistoryOutcome::Unknown(reasons) => {
                    details.push(format!(
                        "{}: object connectivity could not be established: {reasons:?}",
                        repository.key
                    ));
                }
            }
        }
        details
    }

    /// Design §4.1's "at ready" column: what must be absent but still is.
    fn residual(&self, destination: &Path) -> Vec<PathBuf> {
        let mut residual: Vec<PathBuf> = [
            gwz_family_model::LOCK_RELATIVE_PATH,
            ".gwz/catalog-final",
            ".gwz/checked-artifacts",
            crate::stash::STASH_BUNDLE_DIR,
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        residual.extend(
            self.repositories
                .iter()
                .map(|repository| worktrees_of(&repository.relative_git_dir())),
        );
        residual
            .into_iter()
            .filter(|relative| fs::symlink_metadata(destination.join(relative)).is_ok())
            .collect()
    }
}

impl InstallPorts for CoreInstallPorts {
    fn snapshot_source(&mut self, source: &Path) -> Result<SourceSnapshot, InstallPortError> {
        let same_source = fs::canonicalize(source)
            .map(|resolved| resolved == self.source)
            .unwrap_or(false);
        if let (true, Some(snapshot)) = (same_source, self.snapshot.as_ref()) {
            return Ok(snapshot.clone());
        }
        Err(InstallPortError::Destination {
            path: source.to_path_buf(),
            detail: format!(
                "the source is not the preflighted workspace {}",
                self.source.display()
            ),
        })
    }

    fn observe_destination(
        &mut self,
        destination: &Path,
    ) -> Result<DestinationObservation, InstallPortError> {
        let metadata = match fs::symlink_metadata(destination) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(DestinationObservation::absent());
            }
            Err(error) => {
                return Err(InstallPortError::Destination {
                    path: destination.to_path_buf(),
                    detail: error.to_string(),
                });
            }
        };
        if !metadata.is_dir() {
            // A file or a symlink where the destination should be: it
            // exists and it is occupied, and it is nothing installation
            // could complete.
            return Ok(DestinationObservation {
                exists: true,
                nonempty: true,
                ..DestinationObservation::absent()
            });
        }
        let nonempty = fs::read_dir(destination)
            .map_err(|error| InstallPortError::Destination {
                path: destination.to_path_buf(),
                detail: error.to_string(),
            })?
            .next()
            .is_some();
        let is_workspace = destination.join(WORKSPACE_MANIFEST).exists()
            || fs::symlink_metadata(destination.join(RUNTIME_DIR)).is_ok();
        let (family_index, pointer) = match self.store.observe_workspace(
            destination,
            &self.family_id,
            &self.root,
            &self.allocation,
        ) {
            WorkspaceObservation::Missing => (false, PointerObservation::Absent),
            WorkspaceObservation::Unobservable { detail } => {
                return Err(InstallPortError::Destination {
                    path: destination.to_path_buf(),
                    detail,
                });
            }
            WorkspaceObservation::Present(metadata) => (metadata.index, metadata.pointer),
        };
        let merge_store = fs::symlink_metadata(destination.join(".gwz/merge")).is_ok();
        let (residual, dependencies) = if nonempty {
            (self.residual(destination), self.dependencies(destination))
        } else {
            (Vec::new(), Vec::new())
        };
        Ok(DestinationObservation {
            exists: true,
            nonempty,
            is_workspace,
            family_index,
            pointer,
            merge_store,
            residual,
            dependencies,
        })
    }

    fn allocate_destination(&mut self, destination: &Path) -> Result<(), InstallPortError> {
        match fs::create_dir(destination) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let empty_directory = fs::symlink_metadata(destination)
                    .map(|metadata| metadata.is_dir())
                    .unwrap_or(false)
                    && fs::read_dir(destination)
                        .map(|mut entries| entries.next().is_none())
                        .unwrap_or(false);
                if empty_directory {
                    Ok(())
                } else {
                    Err(InstallPortError::Destination {
                        path: destination.to_path_buf(),
                        detail: "the destination exists and is not an empty directory".to_owned(),
                    })
                }
            }
            Err(error) => Err(InstallPortError::Destination {
                path: destination.to_path_buf(),
                detail: error.to_string(),
            }),
        }
    }

    fn construct_repositories(
        &mut self,
        request: &ConstructionRequest,
    ) -> Result<(), InstallPortError> {
        let _ = request;
        Err(InstallPortError::Unimplemented {
            operation: "construct_repositories (clean and bare clones are LCM3.1 and LCM2.3)",
        })
    }

    fn install_destination_git(
        &mut self,
        destination: &Path,
    ) -> Result<GitInstallReport, InstallPortError> {
        install_destination_git(destination, &self.destination_repositories())
    }

    fn recheck_source(&mut self, snapshot: &SourceSnapshot) -> Result<(), InstallPortError> {
        let (_, _, fresh) = self.capture()?;
        recheck(snapshot, &fresh)
    }

    fn recapture_configuration(
        &mut self,
        plan: &ConfigurationPlan,
    ) -> Result<ConfigurationReport, InstallPortError> {
        if plan.mode != CloneMode::Verbatim {
            return Err(InstallPortError::Unimplemented {
                operation: "recapture_configuration for clean and bare clones",
            });
        }
        // Verbatim carries the source lock as copied; it is read back so an
        // undecodable copy stops before the manifest, and nothing is
        // recaptured (the members' HEADs are the source's, byte for byte).
        if plan.destination.join(artifact::LOCK_PATH).exists() {
            artifact::read_lock(&plan.destination).map_err(|error| {
                InstallPortError::Configuration {
                    detail: format!("the copied lock does not decode: {}", error.message),
                }
            })?;
        }
        Ok(ConfigurationReport {
            lock_recaptured: false,
            generated_changes: Vec::new(),
        })
    }

    fn publish_manifest(
        &mut self,
        plan: &ConfigurationPlan,
    ) -> Result<ManifestReceipt, InstallPortError> {
        if plan.mode != CloneMode::Verbatim {
            return Err(InstallPortError::Unimplemented {
                operation: "publish_manifest for clean and bare clones",
            });
        }
        let manifest = self
            .manifest
            .as_ref()
            .ok_or(InstallPortError::Unimplemented {
                operation: "publish_manifest before the source was captured",
            })?;
        // The typed writer regenerates the conf-integrity marker over the
        // final manifest and the copied lock bytes (design §4.1).
        artifact::write_manifest(&plan.destination, manifest).map_err(|error| {
            InstallPortError::Configuration {
                detail: format!("destination manifest: {}", error.message),
            }
        })?;
        let marker_regenerated = matches!(
            artifact::inspect_conf_integrity(&plan.destination),
            ConfIntegrityVerdict::Verified
        );
        Ok(ManifestReceipt { marker_regenerated })
    }
}
