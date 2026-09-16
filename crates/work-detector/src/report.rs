//! The classification result: the verdict, its hazards, the unknown
//! reasons and the bounded diagnostic detail.

use gwz_repo_contract::UnknownReason;

use crate::*;

/// The classification. `hazards` is complete even when `truncated` says the
/// diagnostic list was cut for presentation: truncation drops detail text,
/// never entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkReport {
    pub verdict: WorkVerdict,
    pub hazards: Vec<Hazard>,
    pub unknown: Vec<UnknownReason>,
    pub truncated: bool,
}

/// Hazards and unknown reasons past this position keep their kind and path
/// but lose their diagnostic `detail`, and [`WorkReport::truncated`] says so.
pub const MAX_DETAILED_ENTRIES: usize = 128;

impl WorkReport {
    pub fn unknown(reasons: Vec<UnknownReason>) -> Self {
        Self {
            verdict: WorkVerdict::Unknown,
            hazards: Vec::new(),
            unknown: reasons,
            truncated: false,
        }
    }

    /// No hazard and nothing unknown.
    pub fn clean() -> Self {
        Self {
            verdict: WorkVerdict::Clean,
            hazards: Vec::new(),
            unknown: Vec::new(),
            truncated: false,
        }
    }
}

/// Cut diagnostic text past [`MAX_DETAILED_ENTRIES`], keeping every entry
/// with its kind and path. Truncation never removes a hazard or a reason.
pub(crate) fn drop_details<T>(items: &mut [T], detail: impl Fn(&mut T) -> &mut String) -> bool {
    if items.len() <= MAX_DETAILED_ENTRIES {
        return false;
    }
    for item in &mut items[MAX_DETAILED_ENTRIES..] {
        detail(item).clear();
    }
    true
}
