//! `gwz_workspace_install::InstallPorts` over the real libraries and the
//! existing GWZ helpers: the inspector (`gwz-repo-inspect`) behind the
//! source snapshot, the source recheck and the destination observation; the
//! store (`gwz-family-store`) behind the destination's family metadata
//! reading; the conf-integrity helpers (`crate::artifact`) behind recapture
//! and manifest publication; [`super::git_config`] behind the destination's
//! Git configuration; and `gwz-history-check::check_connectivity` behind
//! the destination's object-connectivity check (design §4.0 dest-complete),
//! bounded by [`super::object_census`] and reported per repository
//! ([`RepositoryVerification`]; LCM1.1 fix 2, 2026-09-06).
//!
//! The snapshot is taken **before** the family lock ([`capture_source`]),
//! so a source that design §4.0 refuses is refused with nothing written --
//! no lock file, no index, no row -- and installation's own
//! `snapshot_source` call returns that capture; `recheck_source`
//! re-inventories the source before publication, which is what makes taking
//! it early safe.
//!
//! Clean and bare modes are not composed here: `construct_repositories`
//! answers `Unimplemented` (LCM2.3 / LCM3.1), and `recapture_configuration`
//! and `publish_manifest` know only the verbatim shape.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gwz_copy_contract::Exclusion;
use gwz_family_model::{AllocationId, CloneMode, FamilyId, PointerObservation};
use gwz_family_store::{WorkspaceObservation, YamlFamilyStore};
use gwz_history_check::{ConnectivityOutcome, NeverCancelled, check_connectivity};
use gwz_repo_contract::{HeadState, Observation, RepoInspector, RepoKey};
use gwz_repo_inspect::{LocalObjectReader, LocalRepoInspector};
use gwz_workspace_install::{
    ConfigurationPlan, ConfigurationReport, ConstructionRequest, DestinationObservation,
    GitInstallReport, InstallPortError, InstallPorts, ManifestReceipt, SourceSnapshot,
};

use super::exclusions::{FIXED_EXCLUSIONS, verbatim_exclusions, worktrees_of};
use super::git_config::{DestinationRepository, install_destination_git};
use super::inventory::{IncludedRepository, included_repositories, recheck, snapshot};
use super::object_census::{ObjectCensus, census_of, connectivity_limits};
use crate::artifact::{self, ConfIntegrityVerdict, ManifestArtifact};
use crate::model::ModelResult;
use crate::workspace::{RUNTIME_DIR, WORKSPACE_MANIFEST};

/// Whether a workspace has an open gwz merge (design §4.1, "refuse verbatim
/// while source has an open gwz merge"): `Some(merge_id)` when one is open.
/// Supplied by the dispatch slot, which can name the merge store's own
/// classifier; the adapter never decodes a record.
pub type OpenMergeProbe = fn(&Path) -> ModelResult<Option<String>>;

/// The source as captured once, before any family file exists (design §4
/// step 1): its included repositories, the exclusion set they imply, the
/// frozen snapshot and the manifest the destination will be given last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCapture {
    pub repositories: Vec<IncludedRepository>,
    pub exclusions: Vec<Exclusion>,
    pub snapshot: SourceSnapshot,
    pub manifest: ManifestArtifact,
}

/// Inventory and capture `source` (canonical). A design §4.0 hazard
/// refuses here as [`InstallPortError::Layout`], with every repository's
/// hazards aggregated; nothing is written.
pub fn capture_source(
    source: &Path,
    open_merge: OpenMergeProbe,
) -> Result<SourceCapture, InstallPortError> {
    let inspector = LocalRepoInspector::new();
    let fixed: Vec<Exclusion> = FIXED_EXCLUSIONS
        .iter()
        .map(|fixed| Exclusion::RelativePath(PathBuf::from(fixed)))
        .collect();
    let repositories = included_repositories(source, &fixed).map_err(|detail| {
        InstallPortError::Layout(gwz_repo_contract::LayoutError::ReadFailed {
            path: source.to_path_buf(),
            detail,
        })
    })?;
    let git_dirs: Vec<PathBuf> = repositories
        .iter()
        .map(IncludedRepository::relative_git_dir)
        .collect();
    let exclusions = verbatim_exclusions(git_dirs.iter().map(PathBuf::as_path));
    let open_merge = open_merge(source).map_err(|error| InstallPortError::Configuration {
        detail: format!("source merge store: {}", error.message),
    })?;
    let snapshot = snapshot(&inspector, source, &repositories, open_merge)?;
    let manifest =
        artifact::read_manifest(source).map_err(|error| InstallPortError::Configuration {
            detail: format!("source manifest: {}", error.message),
        })?;
    Ok(SourceCapture {
        repositories,
        exclusions,
        snapshot,
        manifest,
    })
}

/// One destination repository's dest-complete walk as it ran (LCM1.1 fix
/// 2): what bounded it and what it cost, so the create's report can say
/// exactly what was verified and at what price.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryVerification {
    pub key: RepoKey,
    /// Protected roots the walk started from: every ref, `HEAD`, every
    /// retained reflog entry and stash entry the destination holds.
    pub roots: u64,
    /// Distinct objects read: everything reachable from those roots, each
    /// once.
    pub objects_visited: u64,
    /// The destination store's own object count before the walk, the
    /// walk's ceiling.
    pub census: ObjectCensus,
    pub elapsed: Duration,
}

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
    capture: SourceCapture,
    /// The completion check's walks, one per destination repository that
    /// passed it; reset on every observation.
    verifications: Vec<RepositoryVerification>,
}

impl CoreInstallPorts {
    /// Ports for one create of the family `family_id` at `root`, copying
    /// `source` as captured by [`capture_source`].
    pub fn new(
        root: PathBuf,
        family_id: FamilyId,
        allocation: AllocationId,
        source: PathBuf,
        open_merge: OpenMergeProbe,
        capture: SourceCapture,
    ) -> Self {
        Self {
            inspector: LocalRepoInspector::new(),
            store: YamlFamilyStore::new(),
            root,
            family_id,
            allocation,
            source,
            open_merge,
            capture,
            verifications: Vec::new(),
        }
    }

    /// Design §4.1's exclusion set for this source.
    pub fn exclusions(&self) -> &[Exclusion] {
        &self.capture.exclusions
    }

    /// What the last completion check verified, per repository, in the
    /// inventory's order; empty until the destination has been observed
    /// with a tree in it.
    pub fn verifications(&self) -> &[RepositoryVerification] {
        &self.verifications
    }

    fn destination_repositories(&self) -> Vec<DestinationRepository> {
        self.capture
            .repositories
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
    /// own store -- one `check_connectivity` walk per repository from every
    /// root the inventory found, bounded by the store's own object census
    /// (`connectivity_limits`) and recorded in [`Self::verifications`].
    ///
    /// Not `check_history` with the repository as its own witness: that is
    /// the preservation proof, whose eligibility rule excludes a witness's
    /// own reflog and stash entries, so it called a commit only a reflog
    /// names "unpreserved" in every real repository measured (LCM1.1 fix
    /// 2). Dest-complete asks only whether the store holds what each root
    /// names.
    fn dependencies(&mut self, destination: &Path) -> Vec<String> {
        self.verifications.clear();
        let mut details = Vec::new();
        for repository in &self.capture.repositories {
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
                .capture
                .snapshot
                .repositories
                .iter()
                .find(|captured| captured.key == repository.key)
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
            let census = match census_of(&info.common_dir) {
                Ok(census) => census,
                Err(detail) => {
                    details.push(format!(
                        "{}: the destination object store could not be counted: {detail}",
                        repository.key
                    ));
                    continue;
                }
            };
            let limits = connectivity_limits(protected.roots.len(), &census);
            let reader = LocalObjectReader::open(&info);
            let started = Instant::now();
            match check_connectivity(&protected, &reader, limits, &NeverCancelled) {
                ConnectivityOutcome::Complete(coverage) => {
                    self.verifications.push(RepositoryVerification {
                        key: repository.key.clone(),
                        roots: coverage.roots_checked,
                        objects_visited: coverage.objects_visited,
                        census,
                        elapsed: started.elapsed(),
                    });
                }
                ConnectivityOutcome::Incomplete(items) => {
                    let missing: Vec<String> = items
                        .iter()
                        .map(|item| {
                            if item.missing == item.root.oid {
                                format!("{:?} at {}", item.root.source, item.root.oid)
                            } else {
                                format!("{} (below {})", item.missing, item.root.oid)
                            }
                        })
                        .collect();
                    details.push(format!(
                        "{}: objects missing from the destination store ({} objects in the \
                         store, {} roots): {}",
                        repository.key,
                        census.total(),
                        protected.roots.len(),
                        missing.join(", ")
                    ));
                }
                ConnectivityOutcome::Unknown(reasons) => {
                    details.push(format!(
                        "{}: object connectivity could not be established within the \
                         verification ceiling ({} objects in the store, {} roots, {} bytes of \
                         bookkeeping allowed): {reasons:?}",
                        repository.key,
                        census.total(),
                        protected.roots.len(),
                        limits.max_bookkeeping_bytes
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
            self.capture
                .repositories
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
        if same_source {
            return Ok(self.capture.snapshot.clone());
        }
        Err(InstallPortError::Destination {
            path: source.to_path_buf(),
            detail: format!(
                "the source is not the captured workspace {}",
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
        let fresh = capture_source(&self.source, self.open_merge)?;
        recheck(snapshot, &fresh.snapshot)
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
        // The typed writer regenerates the conf-integrity marker over the
        // final manifest and the copied lock bytes (design §4.1).
        artifact::write_manifest(&plan.destination, &self.capture.manifest).map_err(|error| {
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
