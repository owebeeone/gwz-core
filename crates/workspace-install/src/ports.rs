//! The ports installation drives, their plans and receipts, and the error
//! any of them may return.

use std::fmt;
use std::path::{Path, PathBuf};

use gwz_family_model::CloneMode;
use gwz_repo_contract::LayoutError;

use crate::*;

/// What the configuration port must install. Recapture runs first and the
/// final manifest last; installation calls them in that order rather than
/// asking one port call to keep it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationPlan {
    pub destination: PathBuf,
    pub mode: CloneMode,
    pub branch: Option<String>,
    pub snapshot: SourceSnapshot,
}

/// What recapture did. Design §4.2: the destination lock must be recaptured
/// to the destination HEAD before dest-complete may be true, and generated
/// `gwz.conf/` changes are reported rather than hidden in a commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigurationReport {
    pub lock_recaptured: bool,
    /// Workspace-relative `gwz.conf/` paths this install generated or
    /// changed relative to the frozen commit.
    pub generated_changes: Vec<PathBuf>,
}

/// What publishing the final manifest did. The conf-integrity marker is
/// regenerated for the final manifest and lock bytes; a copied marker
/// vouching for superseded bytes is never accepted (design §4.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestReceipt {
    pub marker_regenerated: bool,
}

/// What installing the destination's Git configuration removed (design
/// §4.1's last exclusion row: filesystem and credential-bearing remote URLs
/// copied from the source go away in install, and ordinary non-credential
/// https/ssh origins remain).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitInstallReport {
    /// The remotes whose URL was removed, as `<repository>: <remote>`.
    pub removed_remotes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallPortError {
    Layout(LayoutError),
    /// The source changed since the snapshot; publication stops.
    Drift {
        detail: String,
    },
    Construction {
        detail: String,
    },
    Configuration {
        detail: String,
    },
    /// The destination could not be allocated or observed.
    Destination {
        path: PathBuf,
        detail: String,
    },
    Unimplemented {
        operation: &'static str,
    },
}

impl fmt::Display for InstallPortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "{error}"),
            Self::Drift { detail } => write!(f, "source drift: {detail}"),
            Self::Construction { detail } => write!(f, "construction: {detail}"),
            Self::Configuration { detail } => write!(f, "configuration: {detail}"),
            Self::Destination { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Unimplemented { operation } => write!(f, "{operation} is not implemented"),
        }
    }
}

impl std::error::Error for InstallPortError {}

/// The narrow ports installation consumes. Core implements them over the
/// repository inspector, `gwz-repo-factory` and the sanctioned existing
/// installation helpers; installation never calls those crates directly.
///
/// **Call order.** Installation calls them in exactly this order, and the
/// order is the contract a real adapter may rely on: `snapshot_source`,
/// `observe_destination`, `allocate_destination`, then either the tree
/// copier or `construct_repositories`, then `install_destination_git`,
/// `observe_destination` again, `recheck_source`,
/// `recapture_configuration` and `publish_manifest`. Nothing follows
/// `publish_manifest` but the row's move to `ready`.
pub trait InstallPorts {
    /// Inventory every included repository of `source` (root, members,
    /// nested) and capture HEADs, branches and remotes. Refuses design §4.0
    /// hazards, aggregated, as [`InstallPortError::Layout`].
    fn snapshot_source(&mut self, source: &Path) -> Result<SourceSnapshot, InstallPortError>;

    /// Observe what stands at `destination`. Called before reservation and
    /// again before publication; it never writes.
    fn observe_destination(
        &mut self,
        destination: &Path,
    ) -> Result<DestinationObservation, InstallPortError>;

    /// Create the destination directory (design §4 step 2, "allocate the
    /// destination"). It is empty when this returns, because the copier
    /// admits only a new or empty destination.
    fn allocate_destination(&mut self, destination: &Path) -> Result<(), InstallPortError>;

    /// Build clean or bare repositories at the destination from the
    /// captured vector, creating `-b <branch>` at the frozen commit
    /// (verbatim mode never calls this).
    fn construct_repositories(
        &mut self,
        request: &ConstructionRequest,
    ) -> Result<(), InstallPortError>;

    /// Install the destination's own Git configuration: remove the
    /// filesystem and credential-bearing remote URLs a verbatim copy
    /// inherited, keeping ordinary non-credential https/ssh origins. Runs
    /// for every mode -- clean and bare keep the source's non-`file:`
    /// `origin` URLs (design §4.2), and having nothing to remove is a
    /// report, not a special case -- and always before the independence
    /// check that has to see the result.
    fn install_destination_git(
        &mut self,
        destination: &Path,
    ) -> Result<GitInstallReport, InstallPortError>;

    /// Verify the source still matches `snapshot`.
    fn recheck_source(&mut self, snapshot: &SourceSnapshot) -> Result<(), InstallPortError>;

    /// Recapture the destination lock to the destination HEAD and the
    /// destination's desired branches, before the manifest.
    fn recapture_configuration(
        &mut self,
        plan: &ConfigurationPlan,
    ) -> Result<ConfigurationReport, InstallPortError>;

    /// Write the final manifest and regenerate the conf-integrity marker
    /// over the final manifest and lock bytes. Nothing follows it.
    fn publish_manifest(
        &mut self,
        plan: &ConfigurationPlan,
    ) -> Result<ManifestReceipt, InstallPortError>;
}
