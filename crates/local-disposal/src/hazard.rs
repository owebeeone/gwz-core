//! Hazard vocabulary: the named waivers, the unknown-name error and the
//! per-repository finding a refusal reports.

use std::fmt;

use gwz_repo_contract::RepoKey;
use gwz_work_detector::Hazard;

/// The named hazards an explicit `--force` may waive (design §5.2). Only
/// these spellings exist on the wire; core rejects any other name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HazardWaiver {
    OpenMerge,
    Dirty,
    UnpreservedHistory,
}

impl HazardWaiver {
    pub const ALL: [HazardWaiver; 3] = [Self::OpenMerge, Self::Dirty, Self::UnpreservedHistory];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenMerge => "open-merge",
            Self::Dirty => "dirty",
            Self::UnpreservedHistory => "unpreserved-history",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|waiver| waiver.as_str() == name)
    }

    /// Parse a request's hazard list: every name must be known and no name
    /// may repeat. Empty means no force.
    pub fn parse_all(names: &[String]) -> Result<Vec<Self>, UnknownHazard> {
        let mut waivers = Vec::new();
        for name in names {
            let waiver = Self::parse(name).ok_or_else(|| UnknownHazard { name: name.clone() })?;
            if waivers.contains(&waiver) {
                return Err(UnknownHazard {
                    name: format!("{name} (repeated)"),
                });
            }
            waivers.push(waiver);
        }
        Ok(waivers)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownHazard {
    pub name: String,
}

impl fmt::Display for UnknownHazard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown hazard `{}`; known hazards: {}",
            self.name,
            HazardWaiver::ALL
                .iter()
                .map(|waiver| waiver.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl std::error::Error for UnknownHazard {}

/// One repository's refusal under one waiver name. Findings are per
/// repository because the deletion tree holds several (design §5.1
/// inspects every one of them), and a report that named only the waiver
/// could not say which lane's history is unpreserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HazardFinding {
    pub waiver: HazardWaiver,
    /// The repository inside the deletion tree the finding belongs to.
    pub repository: RepoKey,
    /// The classifier's hazards, in classification order. Empty for a
    /// history finding, which the work detector never produces.
    pub hazards: Vec<Hazard>,
    /// The history verifier's detail, for
    /// [`HazardWaiver::UnpreservedHistory`] only.
    pub detail: Option<String>,
}

impl HazardFinding {
    /// Whether this finding refuses a disposal of its own accord: an
    /// unpreserved history always does, and work does when at least one of
    /// its hazards is over data the surviving family does not hold (R2,
    /// R8). A finding that refuses nothing is carried into the refusal only
    /// so the report can name its category (R9).
    pub fn refuses(&self) -> bool {
        self.detail.is_some()
            || self
                .hazards
                .iter()
                .any(|hazard| hazard.provenance.refuses())
    }
}
