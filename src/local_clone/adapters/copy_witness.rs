//! What the lane inherited, established for disposal: the clone copy
//! record read back, corroborated against the surviving family, and turned
//! into `gwz_local_disposal::CopyWitness` (`GwzLaneCleanFixes.md` R2, R8;
//! plan `GwzLaneCleanFixesPlan.md` S1.5).
//!
//! # The two halves of the question, and why only one is here
//!
//! A verbatim lane inherits its source's **history** (reflog entries,
//! native stash entries, every commit they still reach) and its **work**
//! (every ignored and untracked entry). The history half needs no record:
//! `gwz-history-check`'s `WitnessPolicy::IdenticalCopy` asks the surviving
//! family repository for the identical object id and proves its whole
//! subgraph, which is a better answer than any record could give (plan §8,
//! the S1.4 adjustment). This module is the work half.
//!
//! # Evidence, not authority
//!
//! The record stands inside the tree being deleted, so on its own it can
//! certify nothing. An entry is cleared only when **both** hold (R2):
//!
//! 1. the record lists it, with the same kind and the same one-`stat`
//!    fingerprint it has now -- the lane has not touched it since the copy;
//!    and
//! 2. the **surviving family's** paired repository still reports an entry
//!    at that path -- deleting the lane therefore loses nothing.
//!
//! Either half failing leaves the entry refusing, as `changed-copy` when
//! the record knew it and as `unique` when it did not. A record that
//! belongs to an earlier tenant of the path (its allocation is not the
//! row's) is not this lane's baseline and is ignored, which leaves every
//! hazard the lane's own -- the conservative answer. A record that exists
//! and cannot be decoded is unknown evidence, never silently "no record".
//!
//! # What is deliberately not cleared
//!
//! Only **ignored and untracked** entries are: they are what a verbatim
//! copy inherits wholesale. A staged or unstaged change, a conflict, a
//! rename or a deletion is tracked work, and it stays reported whatever
//! the record says.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gwz_local_disposal::{CopiedRepository, CopyWitness, RepositoryEvidence};
use gwz_repo_contract::{
    Observation, RepoInspector, RepoKey, RootSource, UnknownKind, UnknownReason, WorkKind,
};
use gwz_repo_inspect::LocalRepoInspector;
use gwz_work_detector::{CopyBaseline, Provenance};

use super::disposal::{strip_structural_work, structural_paths};
use super::inventory::{IncludedRepository, included_repositories};
use crate::local_clone::copy_record::{self, CopyRecord, Fingerprint};

/// One surviving family repository as a baseline: which entries it still
/// reports, and which object ids it still protects.
struct FamilyRepository {
    entries: BTreeMap<Vec<u8>, WorkKind>,
    roots: BTreeSet<String>,
}

/// The family's own repositories, inspected at most once each.
pub(super) struct FamilyPairs<'a> {
    inspector: &'a LocalRepoInspector,
    root: PathBuf,
    inventory: Option<Vec<IncludedRepository>>,
    observed: BTreeMap<RepoKey, Option<FamilyRepository>>,
}

impl<'a> FamilyPairs<'a> {
    pub(super) fn new(inspector: &'a LocalRepoInspector, root: &Path) -> Self {
        Self {
            inspector,
            root: root.to_path_buf(),
            inventory: None,
            observed: BTreeMap::new(),
        }
    }

    /// The family repository paired with `key` by identity, observed once
    /// and cached. `None` when the family has no such repository, or when
    /// it could not be inspected: neither is unknown *evidence* -- it only
    /// means nothing corroborates the lane's copy, so every entry stays the
    /// lane's own, which is what gwz did before any record existed.
    fn pair(&mut self, key: &RepoKey) -> Option<&FamilyRepository> {
        if !self.observed.contains_key(key) {
            let observed = self.observe(key);
            self.observed.insert(key.clone(), observed);
        }
        self.observed.get(key).and_then(Option::as_ref)
    }

    fn observe(&mut self, key: &RepoKey) -> Option<FamilyRepository> {
        if self.inventory.is_none() {
            self.inventory = Some(included_repositories(&self.root, &[]).ok()?);
        }
        let inventory = self.inventory.as_ref()?;
        let repository = inventory.iter().find(|candidate| &candidate.key == key)?;
        let info = self.inspector.inspect_layout(&repository.path).ok()?;
        let mut work = self.inspector.observe_work(&info);
        strip_structural_work(&mut work, &structural_paths(repository, inventory));
        let entries = match work {
            Observation::Known(known) => known
                .entries
                .into_iter()
                .map(|entry| (entry.path, entry.kind))
                .collect(),
            Observation::Unknown(_) => BTreeMap::new(),
        };
        let roots = match self.inspector.inventory_history(&info) {
            Observation::Known(protected) => protected
                .roots
                .iter()
                .map(|root| root.oid.to_hex())
                .collect(),
            Observation::Unknown(_) => BTreeSet::new(),
        };
        Some(FamilyRepository { entries, roots })
    }
}

/// The lane's own record, if it has one this row may rely on.
///
/// `Err` is a record that exists and could not be read: the caller reports
/// it as unknown evidence, which refuses whatever `--force` names, because
/// an uninterpretable record is never "no record".
pub(super) fn read_record(
    lane: &Path,
    allocation: &str,
) -> Result<Option<CopyRecord>, UnknownReason> {
    let record = copy_record::read(lane).map_err(|error| {
        UnknownReason::new(
            UnknownKind::UnsupportedEvidence,
            format!(
                "the clone copy record of {} could not be read: {error}; it is evidence that \
                 exists and cannot be interpreted, so this disposal cannot tell what the lane \
                 inherited from what it made",
                lane.display()
            ),
        )
    })?;
    Ok(record.filter(|record| record.allocation_id == allocation))
}

/// The witness the record and the family together establish, per
/// repository. `None` when the record says nothing this disposal can use.
pub(super) fn recorded_witness(
    record: &CopyRecord,
    lane_repositories: &[IncludedRepository],
    observed: &[RepositoryEvidence],
    pairs: &mut FamilyPairs<'_>,
) -> Option<CopyWitness> {
    let mut repositories = Vec::new();
    for evidence in observed {
        let Some(recorded) = record
            .repositories
            .iter()
            .find(|repository| repository.key == evidence.key)
        else {
            continue;
        };
        let Some(path) = lane_repositories
            .iter()
            .find(|repository| repository.key == evidence.key)
            .map(|repository| repository.path.clone())
        else {
            continue;
        };
        let Some(family) = pairs.pair(&evidence.key) else {
            continue;
        };
        let mut baseline = CopyBaseline::default();
        let fingerprints: BTreeMap<&[u8], (WorkKind, Fingerprint)> = recorded
            .entries
            .iter()
            .map(|entry| (entry.path.as_slice(), (entry.kind, entry.fingerprint)))
            .collect();
        if let Observation::Known(work) = &evidence.work {
            for entry in &work.entries {
                if !matches!(entry.kind, WorkKind::Ignored | WorkKind::Untracked) {
                    continue;
                }
                let Some((kind, recorded)) = fingerprints.get(entry.path.as_slice()) else {
                    continue;
                };
                baseline.set(
                    entry.path.clone(),
                    entry_provenance(
                        *kind == entry.kind
                            && copy_record::entry_path(&path, &entry.path)
                                .and_then(|host| Fingerprint::of(&host))
                                .is_some_and(|now| recorded.unchanged_since(&now)),
                        family.entries.contains_key(entry.path.as_slice()),
                    ),
                );
            }
        }
        baseline.set_stash(stash_provenance(
            evidence,
            &recorded
                .roots
                .iter()
                .filter(|root| matches!(root.source, RootSource::Stash { .. }))
                .map(|root| root.oid.to_hex())
                .collect(),
            family,
        ));
        repositories.push(CopiedRepository {
            key: evidence.key.clone(),
            baseline,
        });
    }
    (!repositories.is_empty()).then_some(CopyWitness { repositories })
}

/// R2's two halves, as one answer. `unchanged` is "the lane has not
/// touched it since the copy"; `in_family` is "the family still holds it".
/// Both must hold, and the record having named the entry at all is what
/// makes the failure a *changed* copy rather than lane-made data.
fn entry_provenance(unchanged: bool, in_family: bool) -> Provenance {
    if unchanged && in_family {
        Provenance::UnchangedCopy
    } else {
        Provenance::ChangedCopy
    }
}

/// The native stash entries are counted as one hazard, so they take one
/// answer: the copy brought every one of them and the family still holds
/// every one of them, or they are the lane's. A `recorded` of `None` is the
/// live comparison (R3), which asks only the family.
fn stash_provenance(
    evidence: &RepositoryEvidence,
    recorded: &BTreeSet<String>,
    family: &FamilyRepository,
) -> Provenance {
    let Observation::Known(protected) = &evidence.history else {
        return Provenance::Unique;
    };
    let mut stashed = protected
        .roots
        .iter()
        .filter(|root| matches!(root.source, RootSource::Stash { .. }))
        .map(|root| root.oid.to_hex())
        .peekable();
    if stashed.peek().is_none() {
        return Provenance::Unique;
    }
    if stashed.all(|oid| recorded.contains(&oid) && family.roots.contains(&oid)) {
        Provenance::UnchangedCopy
    } else {
        Provenance::Unique
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R2 is a conjunction: only an entry the lane has not touched **and**
    /// the family still holds is cleared, and a record that knew the entry
    /// makes every other outcome a *changed* copy rather than lane data.
    #[test]
    fn an_entry_is_cleared_only_when_both_halves_hold() {
        assert_eq!(entry_provenance(true, true), Provenance::UnchangedCopy);
        assert_eq!(entry_provenance(false, true), Provenance::ChangedCopy);
        assert_eq!(entry_provenance(true, false), Provenance::ChangedCopy);
        assert_eq!(entry_provenance(false, false), Provenance::ChangedCopy);
        for provenance in [
            entry_provenance(false, true),
            entry_provenance(true, false),
            entry_provenance(false, false),
        ] {
            assert!(provenance.refuses(), "{provenance:?}");
        }
        assert!(!entry_provenance(true, true).refuses());
    }
}
