//! The verdict and the hazard vocabulary it is built from.

use gwz_repo_contract::{BytePath, WorkKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkVerdict {
    Clean,
    Dirty,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HazardKind {
    /// On-disk work of the named kind.
    Work(WorkKind),
    /// A tracked path whose suppressed status was observed to differ or
    /// could not be observed.
    Suppressed,
    OpenNativeOperation,
    OpenGwzMerge,
    OpenGwzStash,
    /// Evidence that exists but cannot be interpreted.
    UninterpretableEvidence,
    /// Native stash entries present.
    NativeStash,
    /// Another open GWZ coordination record; `Hazard::detail` names it.
    OpenGwzRecord,
}

impl HazardKind {
    /// The `gwz local dispose --force <name>` waiver that covers this hazard
    /// (design §5.2), or `None` for a hazard that no waiver covers because it
    /// always arrives with an unknown reason and an unknown work inventory
    /// refuses instead. `open-merge` is the design's only open-operation
    /// name, so an unfinished native operation maps to it too. Disposal owns
    /// the waiver vocabulary; this is the classifier's side of the
    /// one-to-one mapping, kept exhaustive so a new hazard cannot be added
    /// without deciding how it refuses.
    pub fn force_name(&self) -> Option<&'static str> {
        match self {
            Self::Work(_) | Self::Suppressed | Self::NativeStash => Some("dirty"),
            Self::OpenNativeOperation
            | Self::OpenGwzMerge
            | Self::OpenGwzStash
            | Self::OpenGwzRecord => Some("open-merge"),
            Self::UninterpretableEvidence => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hazard {
    pub kind: HazardKind,
    pub path: Option<BytePath>,
    pub detail: String,
}
