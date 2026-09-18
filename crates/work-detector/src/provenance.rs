//! Where a hazard's data came from: the provenance vocabulary and the
//! per-path baseline the caller establishes and hands in.
//!
//! `gwz local dispose` refuses a lane over data the lane did not make. A
//! verbatim lane inherits every ignored entry and every native stash entry
//! of the workspace it was copied from, and with no baseline the classifier
//! must call all of it the lane's own work (gwz-dev
//! `dev-docs/GwzLaneIssues.md` L1; `GwzLaneCleanFixes.md` R2, R8). This
//! module is the shape of that baseline.
//!
//! **The classifier never establishes provenance itself.** It reads
//! nothing. The caller compares the lane against its clone copy record and
//! against the surviving family and hands the answer in per path, exactly
//! as it hands in [`GwzEvidence`](crate::GwzEvidence). A caller that
//! supplies no baseline gets [`Provenance::Unique`] for everything, which
//! is what every classification before this one said.

use std::collections::BTreeMap;

use gwz_repo_contract::BytePath;

/// What is known about where one piece of a lane's data came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Provenance {
    /// Nothing says the data is anyone else's: no baseline covers it, or a
    /// baseline covers it and the surviving family does not hold it. This
    /// is the default, and the conservative answer.
    #[default]
    Unique,
    /// The copy brought it, it is unchanged since, and the family still
    /// holds it. Deleting the lane loses nothing (R2).
    UnchangedCopy,
    /// The copy brought it and the lane changed it, or the family no
    /// longer holds it. Still the lane's to lose (R8).
    ChangedCopy,
    /// Regenerable: a build cache, a `__pycache__`, a tool's convenience
    /// symlink (R5, R6). **Nothing produces this yet**: the recogniser is
    /// plan `GwzLaneCleanFixesPlan.md` S2.1 and its wiring is S2.3. The
    /// variant exists so a Phase 1 report can name the category and show
    /// it empty (S1.7).
    Regenerable,
}

impl Provenance {
    /// Every provenance, in report order: least the lane's own first.
    pub const ALL: [Provenance; 4] = [
        Self::Regenerable,
        Self::UnchangedCopy,
        Self::ChangedCopy,
        Self::Unique,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unique => "unique",
            Self::UnchangedCopy => "unchanged-copy",
            Self::ChangedCopy => "changed-copy",
            Self::Regenerable => "regenerable",
        }
    }

    /// Whether data of this provenance refuses a disposal of its own
    /// accord. Only what the lane alone holds does: an unchanged copy is
    /// still in the family after the deletion, and a cache is rebuilt.
    pub const fn refuses(self) -> bool {
        match self {
            Self::Unique | Self::ChangedCopy => true,
            Self::UnchangedCopy | Self::Regenerable => false,
        }
    }
}

/// What the caller established about one repository's inherited data.
///
/// Paths are the observation's own: repository-relative, Git's raw bytes,
/// with the trailing `/` of a directory reported whole. A path the baseline
/// does not name is [`Provenance::Unique`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CopyBaseline {
    entries: BTreeMap<BytePath, Provenance>,
    stash: Provenance,
}

impl CopyBaseline {
    /// Record one entry's provenance. A repeated path keeps the more
    /// refusing answer, so two disagreeing comparisons never make a
    /// baseline less conservative than either of them.
    pub fn set(&mut self, path: BytePath, provenance: Provenance) {
        let slot = self.entries.entry(path).or_insert(provenance);
        if provenance.refuses() {
            *slot = provenance;
        }
    }

    /// Record what the repository's native stash entries are, taken
    /// together: they are counted as one hazard, so they have one
    /// provenance.
    pub fn set_stash(&mut self, provenance: Provenance) {
        self.stash = provenance;
    }

    pub fn entry(&self, path: &[u8]) -> Provenance {
        self.entries.get(path).copied().unwrap_or_default()
    }

    pub fn stash(&self) -> Provenance {
        self.stash
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.stash == Provenance::Unique
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default is the conservative answer, only lane-held data
    /// refuses, and every provenance names itself exactly once.
    #[test]
    fn provenance_defaults_to_unique_and_only_lane_data_refuses() {
        assert_eq!(Provenance::default(), Provenance::Unique);
        assert!(Provenance::Unique.refuses());
        assert!(Provenance::ChangedCopy.refuses());
        assert!(!Provenance::UnchangedCopy.refuses());
        assert!(!Provenance::Regenerable.refuses());
        let mut names: Vec<&str> = Provenance::ALL.iter().map(|p| p.as_str()).collect();
        let listed = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), listed, "each category names itself once");
    }

    /// An unnamed path is unique, a named one answers as recorded, and a
    /// disagreement keeps the refusing answer.
    #[test]
    fn a_baseline_answers_by_path_and_never_softens_a_refusal() {
        let mut baseline = CopyBaseline::default();
        assert!(baseline.is_empty());
        assert_eq!(baseline.entry(b"target/"), Provenance::Unique);
        assert_eq!(baseline.stash(), Provenance::Unique);

        baseline.set(b"target/".to_vec(), Provenance::UnchangedCopy);
        baseline.set(b"notes.txt".to_vec(), Provenance::Unique);
        assert_eq!(baseline.entry(b"target/"), Provenance::UnchangedCopy);
        assert!(!baseline.is_empty());

        baseline.set(b"target/".to_vec(), Provenance::ChangedCopy);
        assert_eq!(
            baseline.entry(b"target/"),
            Provenance::ChangedCopy,
            "the refusing answer wins"
        );
        baseline.set(b"target/".to_vec(), Provenance::UnchangedCopy);
        assert_eq!(
            baseline.entry(b"target/"),
            Provenance::ChangedCopy,
            "and is not softened afterwards"
        );

        baseline.set_stash(Provenance::UnchangedCopy);
        assert_eq!(baseline.stash(), Provenance::UnchangedCopy);
    }
}
