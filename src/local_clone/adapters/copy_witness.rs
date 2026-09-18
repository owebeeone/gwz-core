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
//! # Regenerable data is a different question
//!
//! What a tool made and the same tool remakes is not the lane's work
//! whatever the record says, so [`mark_regenerable`] runs for every lane
//! and its answer outranks the comparison's (R5 to R7, plan S2.3).
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
use gwz_repo_inspect::{LocalRepoInspector, regenerable};
use gwz_work_detector::{CopyBaseline, Provenance};

use super::disposal::{strip_structural_work, structural_paths};
use super::inventory::{IncludedRepository, included_repositories};
use crate::local_clone::copy_record::{self, CopyRecord, Fingerprint};

/// One surviving family repository as a baseline: which entries it still
/// reports, and which object ids it still protects.
struct FamilyRepository {
    /// The repository's worktree, so an entry the lane holds can be
    /// compared with the family's own copy of it (R3).
    path: PathBuf,
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
        Some(FamilyRepository {
            path: repository.path.clone(),
            entries,
            roots,
        })
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
            Some(
                &recorded
                    .roots
                    .iter()
                    .filter(|root| matches!(root.source, RootSource::Stash { .. }))
                    .map(|root| root.oid.to_hex())
                    .collect(),
            ),
            family,
        ));
        repositories.push(CopiedRepository {
            key: evidence.key.clone(),
            baseline,
        });
    }
    (!repositories.is_empty()).then_some(CopyWitness { repositories })
}

/// The witness dispose derives itself, for a lane no record covers (R3):
/// a lane made by a gwz older than the record, or copied outside gwz
/// altogether. Nothing is assumed about it -- each ignored and untracked
/// entry is compared with the family's own entry at the same path, and an
/// entry the family does not report, or reports differently, is the lane's.
///
/// The **history** half of R3 -- "the commits from `rev-list --all`, the
/// reflog and the stash that no surviving family repository holds" -- needs
/// no witness of its own and gets none here: `check_history` already walks
/// the lane's whole protected inventory against each surviving family
/// repository's own object store under `WitnessPolicy::IdenticalCopy`, at
/// the identical object id and with a whole-subgraph proof, and a root no
/// witness holds is unpreserved (plan §8, the S1.4 adjustment). What is
/// left for the witness is the native stash's *work* hazard.
pub(super) fn live_witness(
    lane_repositories: &[IncludedRepository],
    observed: &[RepositoryEvidence],
    pairs: &mut FamilyPairs<'_>,
) -> Option<CopyWitness> {
    let mut repositories = Vec::new();
    for evidence in observed {
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
        if let Observation::Known(work) = &evidence.work {
            for entry in &work.entries {
                if !matches!(entry.kind, WorkKind::Ignored | WorkKind::Untracked) {
                    continue;
                }
                if family.entries.get(entry.path.as_slice()) != Some(&entry.kind) {
                    // The family does not report it at all, or reports it
                    // as something else: it is the lane's (R0.1).
                    continue;
                }
                let identical = copy_record::entry_path(&path, &entry.path)
                    .zip(copy_record::entry_path(&family.path, &entry.path))
                    .is_some_and(|(lane, family)| same_entry(&lane, &family, &mut Budget::new()));
                if identical {
                    baseline.set(entry.path.clone(), Provenance::UnchangedCopy);
                } else {
                    baseline.set(entry.path.clone(), Provenance::ChangedCopy);
                }
            }
        }
        baseline.set_stash(stash_provenance(evidence, None, family));
        repositories.push(CopiedRepository {
            key: evidence.key.clone(),
            baseline,
        });
    }
    (!repositories.is_empty()).then_some(CopyWitness { repositories })
}

/// Regenerable data, stamped onto whatever witness the comparison built
/// (R5, R6, R8; plan S2.3).
///
/// This runs for **every** lane, with a record or without one, and it is
/// the last word: `CopyBaseline::set` keeps
/// [`Provenance::Regenerable`] whatever a comparison said before or says
/// after. That is R7 -- the recogniser asks what the data *is*, and a
/// cache the lane rebuilt, or one the lane created that the family never
/// had, is still a cache. Everything the recogniser does **not** claim is
/// left exactly as the comparison classified it, so ignored data that is
/// neither regenerable nor an unchanged copy still refuses (R8).
///
/// The lane's own root is the workspace boundary the convenience-link rule
/// asks about: a `bazel-out` in the lane points at the build tool's output
/// base outside the copy, which is what makes it a convenience link and not
/// data.
pub(super) fn mark_regenerable(
    lane: &Path,
    lane_repositories: &[IncludedRepository],
    observed: &[RepositoryEvidence],
    witness: Option<CopyWitness>,
) -> Option<CopyWitness> {
    let mut repositories = witness
        .map(|witness| witness.repositories)
        .unwrap_or_default();
    for evidence in observed {
        let Some(path) = lane_repositories
            .iter()
            .find(|repository| repository.key == evidence.key)
            .map(|repository| repository.path.clone())
        else {
            continue;
        };
        let Observation::Known(work) = &evidence.work else {
            continue;
        };
        let mut recognised = Vec::new();
        for entry in &work.entries {
            if !matches!(entry.kind, WorkKind::Ignored | WorkKind::Untracked) {
                continue;
            }
            let Some(host) = copy_record::entry_path(&path, &entry.path) else {
                continue;
            };
            if regenerable::recognise_under(lane, &path, &host).is_some() {
                recognised.push(entry.path.clone());
            }
        }
        if recognised.is_empty() {
            continue;
        }
        let slot = match repositories
            .iter()
            .position(|repository| repository.key == evidence.key)
        {
            Some(found) => found,
            None => {
                repositories.push(CopiedRepository {
                    key: evidence.key.clone(),
                    baseline: CopyBaseline::default(),
                });
                repositories.len() - 1
            }
        };
        for path in recognised {
            repositories[slot]
                .baseline
                .set(path, Provenance::Regenerable);
        }
    }
    (!repositories.is_empty()).then_some(CopyWitness { repositories })
}

/// How much of one entry the live comparison will look at before it gives
/// up and calls the entry the lane's. A lane of this workspace holds tens
/// of thousands of ignored files under a handful of reported directories,
/// and an unbounded walk is exactly what R13 forbids; exceeding the budget
/// degrades to a refusal the operator can still waive, never to a silent
/// pass and never to unwaivable unknown evidence.
struct Budget(u32);

impl Budget {
    const MAX_ENTRIES: u32 = 4096;
    /// Files at or below this size are compared byte for byte; a larger
    /// one is compared by its size and its modification time, which is the
    /// same fingerprint R1 records.
    const MAX_COMPARED_BYTES: u64 = 1 << 20;

    fn new() -> Self {
        Self(Self::MAX_ENTRIES)
    }

    fn spend(&mut self) -> bool {
        self.0 = self.0.saturating_sub(1);
        self.0 > 0
    }
}

/// Whether the entry at `lane` is the family's entry at `family`,
/// unchanged: the same kind, and the same link target, the same bytes or
/// the same size and modification time. A directory is compared by its
/// names, recursively, within [`Budget`]. Anything unreadable is not a
/// match: the comparison proves sameness or it refuses.
fn same_entry(lane: &Path, family: &Path, budget: &mut Budget) -> bool {
    if !budget.spend() {
        return false;
    }
    let (Ok(here), Ok(there)) = (
        std::fs::symlink_metadata(lane),
        std::fs::symlink_metadata(family),
    ) else {
        return false;
    };
    if here.is_symlink() && there.is_symlink() {
        return match (std::fs::read_link(lane), std::fs::read_link(family)) {
            (Ok(here), Ok(there)) => here == there,
            _ => false,
        };
    }
    if here.is_dir() && there.is_dir() {
        let (Some(here), Some(there)) = (child_names(lane), child_names(family)) else {
            return false;
        };
        if here != there {
            return false;
        }
        return here
            .iter()
            .all(|name| same_entry(&lane.join(name), &family.join(name), budget));
    }
    if !here.is_file() || !there.is_file() || here.len() != there.len() {
        return false;
    }
    if here.len() > Budget::MAX_COMPARED_BYTES {
        return Fingerprint::of(lane)
            .zip(Fingerprint::of(family))
            .is_some_and(|(here, there)| {
                here.mtime_secs == there.mtime_secs && here.mtime_nanos == there.mtime_nanos
            });
    }
    match (std::fs::read(lane), std::fs::read(family)) {
        (Ok(here), Ok(there)) => here == there,
        _ => false,
    }
}

/// The directory's entry names, sorted; `None` when it cannot be listed.
fn child_names(directory: &Path) -> Option<Vec<std::ffi::OsString>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(directory).ok()? {
        names.push(entry.ok()?.file_name());
    }
    names.sort();
    Some(names)
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
    recorded: Option<&BTreeSet<String>>,
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
    if stashed.all(|oid| {
        recorded.is_none_or(|recorded| recorded.contains(&oid)) && family.roots.contains(&oid)
    }) {
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

    /// R3's comparison: the same bytes, the same names and the same link
    /// target are the same entry; anything else, anything unreadable and
    /// anything past the budget is the lane's.
    #[test]
    fn the_live_comparison_proves_sameness_or_refuses() {
        let base = std::env::temp_dir().join(format!("gwz-live-compare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (lane, family) = (base.join("lane"), base.join("family"));
        for side in [&lane, &family] {
            std::fs::create_dir_all(side.join("cache/nested")).unwrap();
            std::fs::write(side.join("cache/a.bin"), b"same bytes").unwrap();
            std::fs::write(side.join("cache/nested/b.bin"), b"also same").unwrap();
        }

        assert!(same_entry(
            &lane.join("cache"),
            &family.join("cache"),
            &mut Budget::new()
        ));
        assert!(
            !same_entry(&lane.join("cache"), &family.join("cache"), &mut Budget(2)),
            "a walk past its budget refuses rather than passing"
        );

        std::fs::write(lane.join("cache/nested/b.bin"), b"rebuilt!!").unwrap();
        assert!(
            !same_entry(
                &lane.join("cache"),
                &family.join("cache"),
                &mut Budget::new()
            ),
            "bytes that differ are not the same entry, however deep"
        );
        std::fs::write(lane.join("cache/nested/b.bin"), b"also same").unwrap();

        std::fs::write(lane.join("cache/only-here"), b"x").unwrap();
        assert!(!same_entry(
            &lane.join("cache"),
            &family.join("cache"),
            &mut Budget::new()
        ));
        std::fs::remove_file(lane.join("cache/only-here")).unwrap();

        // An entry the family does not have at all.
        assert!(!same_entry(
            &lane.join("cache"),
            &family.join("absent"),
            &mut Budget::new()
        ));

        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                std::os::unix::fs::symlink("cache/a.bin", lane.join("link")).unwrap();
                std::os::unix::fs::symlink("cache/a.bin", family.join("link")).unwrap();
                assert!(same_entry(
                    &lane.join("link"),
                    &family.join("link"),
                    &mut Budget::new()
                ));
                std::os::unix::fs::symlink("elsewhere", lane.join("other")).unwrap();
                std::os::unix::fs::symlink("cache/a.bin", family.join("other")).unwrap();
                assert!(
                    !same_entry(
                        &lane.join("other"),
                        &family.join("other"),
                        &mut Budget::new()
                    ),
                    "a link is its target, and never what it points at"
                );
                assert!(
                    !same_entry(
                        &lane.join("link"),
                        &family.join("cache/a.bin"),
                        &mut Budget::new()
                    ),
                    "a link is never the file it points at"
                );
            }
        }

        std::fs::remove_dir_all(&base).unwrap();
    }
}
