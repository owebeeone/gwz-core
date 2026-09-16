//! What the destination looks like before installation, and the
//! construction the ports are asked to perform.

use std::path::PathBuf;

use gwz_family_model::{CloneMode, PointerObservation};

use crate::*;

/// What stands at the destination, as the port observed it. Installation
/// reads it twice with two rule sets: admission before reservation, and
/// design §4.1's completion column before publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestinationObservation {
    /// The path exists.
    pub exists: bool,
    /// It exists and holds at least one entry.
    pub nonempty: bool,
    /// It is already a GWZ workspace (a discoverable manifest or `.gwz/`).
    pub is_workspace: bool,
    /// `.gwz/local-family.yml` is present: the path is a family root.
    pub family_index: bool,
    /// The clone pointer, classified against this family and this root.
    pub pointer: PointerObservation,
    /// `.gwz/merge/` is present, empty or not.
    pub merge_store: bool,
    /// Entries from design §4.1's "absent at ready" column that are still
    /// present, as workspace-relative paths.
    pub residual: Vec<PathBuf>,
    /// Why the destination's Git metadata is not independent of the source
    /// (design §4.0 dest-complete). Empty means independent.
    pub dependencies: Vec<String>,
}

impl DestinationObservation {
    /// A path that holds nothing: the destination admission expects it, and
    /// every completion rule refuses it.
    pub fn absent() -> Self {
        Self {
            exists: false,
            nonempty: false,
            is_workspace: false,
            family_index: false,
            pointer: PointerObservation::Absent,
            merge_store: false,
            residual: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    /// A finished destination that passes every completion rule: a matching
    /// pointer, no index, no merge store, nothing residual and no external
    /// dependency.
    pub fn complete() -> Self {
        Self {
            exists: true,
            nonempty: true,
            is_workspace: true,
            pointer: PointerObservation::Matches,
            ..Self::absent()
        }
    }
}

impl Default for DestinationObservation {
    fn default() -> Self {
        Self::absent()
    }
}

/// What the construction port must build for clean and bare modes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructionRequest {
    pub mode: CloneMode,
    pub branch: Option<String>,
    pub snapshot: SourceSnapshot,
    pub destination: PathBuf,
}
