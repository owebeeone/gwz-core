//! Hazard vocabulary: the named waivers, the unknown-name error and the
//! per-repository finding a refusal reports.

use std::fmt;

use gwz_repo_contract::RepoKey;
use gwz_work_detector::{Hazard, Provenance};

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

/// The categories a refusal separates (R9). They are the **report's**
/// vocabulary, not the wire's: a category says where the hazard's data came
/// from, while a [`HazardWaiver`] says what `--force` spells. In this
/// release several categories still share one waiver name -- narrowing the
/// waiver vocabulary so each category has its own is R11, plan
/// `GwzLaneCleanFixesPlan.md` S3.1 -- which is exactly why the refusal
/// prints the waiver command itself (R10) instead of leaving the operator
/// to work it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HazardCategory {
    /// Rebuildable by the tool that made it. Phase 2 fills it; in Phase 1
    /// it is reported and always empty.
    Regenerable,
    /// The copy brought it, the lane has not touched it and the family
    /// still holds it. Reported, and does not refuse.
    UnchangedCopy,
    /// The copy brought it and it is not the family's any more.
    ChangedCopy,
    /// The lane alone holds it.
    Unique,
}

impl HazardCategory {
    /// Every category, in report order: least the lane's own first.
    pub const ALL: [HazardCategory; 4] = [
        Self::Regenerable,
        Self::UnchangedCopy,
        Self::ChangedCopy,
        Self::Unique,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Regenerable => "regenerable",
            Self::UnchangedCopy => "unchanged copy",
            Self::ChangedCopy => "changed copy",
            Self::Unique => "unique to the lane",
        }
    }

    pub const fn of(provenance: Provenance) -> Self {
        match provenance {
            Provenance::Regenerable => Self::Regenerable,
            Provenance::UnchangedCopy => Self::UnchangedCopy,
            Provenance::ChangedCopy => Self::ChangedCopy,
            Provenance::Unique => Self::Unique,
        }
    }

    /// Whether data in this category refuses a disposal of its own accord.
    pub const fn refuses(self) -> bool {
        match self {
            Self::Unique | Self::ChangedCopy => true,
            Self::UnchangedCopy | Self::Regenerable => false,
        }
    }
}

/// One thing a refusal found, under one category: the repository it is in,
/// its path where it has one, and the reason in words. A protected root
/// that no surviving repository holds has no path -- its `detail` names the
/// object ids instead (R9, "the paths **or object ids**").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryItem {
    pub repository: RepoKey,
    pub path: Option<gwz_repo_contract::BytePath>,
    pub detail: String,
}

/// One category as a refusal reports it: what it holds, in the order the
/// findings were classified in. An empty category is still reported, so a
/// refusal always says what it did *not* find as well.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryReport {
    pub category: HazardCategory,
    pub items: Vec<CategoryItem>,
}

impl CategoryReport {
    pub fn count(&self) -> usize {
        self.items.len()
    }
}

/// `findings` sorted into every category, empty ones included (R9).
pub fn categorise(findings: &[HazardFinding]) -> Vec<CategoryReport> {
    let mut reports: Vec<CategoryReport> = HazardCategory::ALL
        .into_iter()
        .map(|category| CategoryReport {
            category,
            items: Vec::new(),
        })
        .collect();
    let mut push = |category: HazardCategory, item: CategoryItem| {
        if let Some(report) = reports
            .iter_mut()
            .find(|report| report.category == category)
        {
            report.items.push(item);
        }
    };
    for finding in findings {
        if let Some(detail) = &finding.detail {
            // An unpreserved history is, by definition, what no surviving
            // repository holds.
            push(
                HazardCategory::Unique,
                CategoryItem {
                    repository: finding.repository.clone(),
                    path: None,
                    detail: detail.clone(),
                },
            );
        }
        for hazard in &finding.hazards {
            push(
                HazardCategory::of(hazard.provenance),
                CategoryItem {
                    repository: finding.repository.clone(),
                    path: hazard.path.clone(),
                    detail: hazard.detail.clone(),
                },
            );
        }
    }
    reports
}

/// The waiver names that must be given to delete despite `findings`, in
/// this vocabulary's own order and each named once: exactly what R10's
/// printed command carries. A finding that refuses nothing needs no
/// waiver and contributes none.
pub fn required_waivers(findings: &[HazardFinding]) -> Vec<HazardWaiver> {
    HazardWaiver::ALL
        .into_iter()
        .filter(|waiver| {
            findings
                .iter()
                .any(|finding| &finding.waiver == waiver && finding.refuses())
        })
        .collect()
}
