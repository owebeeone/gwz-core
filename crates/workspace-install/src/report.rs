//! What installation reports: the ordered steps, the effects applied, the
//! report itself and the progress sink.

use std::fmt;

use gwz_copy_contract::CopyReport;
use gwz_family_model::{FamilyChange, MemberName};
use gwz_family_store_contract::FamilySession;

use crate::*;

/// The ordered step a failure belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallStep {
    InventorySource,
    ObserveDestination,
    Reserve,
    AllocateDestination,
    CopyTree,
    ConstructRepositories,
    InstallDestinationGit,
    InstallPointer,
    CheckDestination,
    RecheckSource,
    RecaptureConfiguration,
    PublishManifest,
    MarkReady,
}

impl InstallStep {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InventorySource => "inventory source",
            Self::ObserveDestination => "observe destination",
            Self::Reserve => "reserve",
            Self::AllocateDestination => "allocate destination",
            Self::CopyTree => "copy tree",
            Self::ConstructRepositories => "construct repositories",
            Self::InstallDestinationGit => "install destination git configuration",
            Self::InstallPointer => "install pointer",
            Self::CheckDestination => "check destination",
            Self::RecheckSource => "recheck source",
            Self::RecaptureConfiguration => "recapture configuration",
            Self::PublishManifest => "publish manifest",
            Self::MarkReady => "mark ready",
        }
    }
}

impl fmt::Display for InstallStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One completed installation effect, in the order installation performs
/// them. A failure reports the prefix that actually happened; none of them
/// is undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallEffect {
    RowAllocated,
    DestinationAllocated,
    TreeCopied,
    RepositoriesConstructed,
    DestinationGitInstalled,
    /// The allocation marker and then the pointer, as the store writes them.
    PointerInstalled,
    ConfigurationInstalled,
    ManifestPublished,
    /// A diagnostic was recorded on the retained `creating` row.
    ErrorRecorded,
    RowReady,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallReport {
    pub copy: Option<CopyReport>,
    /// Remote URLs removed from the destination's Git configuration.
    pub git: Option<GitInstallReport>,
    /// Generated `gwz.conf/` changes and lock recapture, reported rather
    /// than hidden in a commit (design §4.2).
    pub configuration: Option<ConfigurationReport>,
    pub effects: Vec<InstallEffect>,
}

/// What has already happened, so a failure can report it.
#[derive(Debug, Default)]
pub(crate) struct Progress {
    pub(crate) effects: Vec<InstallEffect>,
    pub(crate) copy: Option<CopyReport>,
    pub(crate) git: Option<GitInstallReport>,
    pub(crate) configuration: Option<ConfigurationReport>,
}

impl Progress {
    pub(crate) fn did(&mut self, effect: InstallEffect) {
        self.effects.push(effect);
    }

    /// Stop. The `creating` row keeps its reservation and gains a
    /// diagnostic (design §3's `last_error`, §4's "diagnostic row"); that
    /// best-effort write is the only thing a failure does, it undoes
    /// nothing, and its own failure never masks `error`.
    pub(crate) fn stop(
        mut self,
        session: &mut dyn FamilySession,
        name: &MemberName,
        step: InstallStep,
        error: InstallError,
    ) -> InstallFailure {
        if self.effects.contains(&InstallEffect::RowAllocated)
            && !self.effects.contains(&InstallEffect::RowReady)
            && session
                .apply(&FamilyChange::RecordError {
                    name: name.clone(),
                    last_error: format!("{step}: {error}"),
                })
                .is_ok()
        {
            self.effects.push(InstallEffect::ErrorRecorded);
        }
        InstallFailure {
            step,
            error,
            effects: self.effects,
        }
    }
}
