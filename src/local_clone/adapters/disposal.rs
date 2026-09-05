//! `gwz_local_disposal::DisposalPorts` over the real libraries: the store
//! and the inspector behind `observe_target`, `gwz-history-check` behind
//! `check_history`, and the ordinary recursive remover behind
//! `remove_directory`.
//!
//! # One `check_history` call per witness store (lane H proposal H2)
//!
//! `check_history` here calls `gwz_history_check::check_history` **once per
//! surviving family repository**, each call with an `ObjectReader` that
//! serves exactly that repository's object store, and combines the
//! per-witness outcomes: a protected root is preserved when some single
//! witness preserves it whole. It never hands the verifier a union reader
//! spanning several witnesses, because a union could complete one witness's
//! graph with another witness's objects and so certify a root as preserved
//! in a repository that does not hold its whole subgraph (design §5.1, §11
//! item 9). Witnesses are paired with the target repository by identity:
//! the family root's repository for `RepoKey::Root`, the member with the
//! same manifest id for `RepoKey::Member`; a nested repository has no pair.
//!
//! # GWZ evidence
//!
//! Core decodes GWZ evidence for the work detector. This build reads only
//! the merge store's envelope (through the dispatch slot's probe) and the
//! presence of stash records: an open merge is `Open`, present stash records
//! are `Unknown` -- decoding them into coordination roots is LCM2.1's work
//! -- so a target carrying either refuses ordinary deletion rather than
//! being read as clean.

use std::fs;
use std::path::{Path, PathBuf};

use gwz_family_model::{FamilyView, MemberName, MemberState, ROOT_NAME, TargetObservation};
use gwz_family_store::YamlFamilyStore;
use gwz_history_check::{HistoryOutcome, Limits, NeverCancelled, Witness, check_history};
use gwz_local_disposal::{
    DisposalPorts, HistoryAnswer, HistoryQuery, PortError, RemovalFailure, RepositoryEvidence,
    TargetEvidence,
};
use gwz_repo_contract::{
    ObjectId, ProtectedRoot, RepoInspector, RepoKey, UnknownKind, UnknownReason,
};
use gwz_repo_inspect::{LocalObjectReader, LocalRepoInspector};
use gwz_work_detector::{EvidenceState, GwzEvidence};

use super::install::OpenMergeProbe;
use super::inventory::included_repositories;
use super::removal::remove_tree;
use crate::artifact;

/// The real disposal ports for one `dispose` of member `name`.
pub struct CoreDisposalPorts {
    inspector: LocalRepoInspector,
    store: YamlFamilyStore,
    /// The family root, canonical.
    root: PathBuf,
    view: FamilyView,
    name: MemberName,
    open_merge: OpenMergeProbe,
}

impl CoreDisposalPorts {
    pub fn new(
        root: PathBuf,
        view: FamilyView,
        name: MemberName,
        open_merge: OpenMergeProbe,
    ) -> Self {
        Self {
            inspector: LocalRepoInspector::new(),
            store: YamlFamilyStore::new(),
            root,
            view,
            name,
            open_merge,
        }
    }

    fn gwz_evidence(&self, target: &Path) -> GwzEvidence {
        let merge = match (self.open_merge)(target) {
            Ok(Some(merge_id)) => EvidenceState::Open {
                detail: format!("merge `{merge_id}` is open"),
            },
            Ok(None) => EvidenceState::None,
            Err(error) => EvidenceState::Unknown {
                detail: error.message,
            },
        };
        let bundles = target.join(crate::stash::STASH_BUNDLE_DIR);
        let stash = match fs::read_dir(&bundles) {
            Ok(entries) => {
                let records = entries
                    .flatten()
                    .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "yaml"))
                    .count();
                if records == 0 {
                    EvidenceState::None
                } else {
                    EvidenceState::Unknown {
                        detail: format!(
                            "{records} gwz stash record(s) under {}; this build does not decode \
                             them (LCM2.1)",
                            bundles.display()
                        ),
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => EvidenceState::None,
            Err(error) => EvidenceState::Unknown {
                detail: format!("{}: {error}", bundles.display()),
            },
        };
        GwzEvidence {
            merge,
            stash,
            other: Vec::new(),
        }
    }

    /// The surviving repositories paired with `target` by identity: the
    /// root's, then every other ready member's, each as its own witness.
    fn witnesses(&self, target: &RepoKey) -> Vec<(String, PathBuf)> {
        let mut workspaces: Vec<(String, PathBuf)> =
            vec![(ROOT_NAME.to_owned(), self.root.clone())];
        for (name, row) in &self.view.members {
            if name == &self.name || row.state != MemberState::Ready {
                continue;
            }
            workspaces.push((name.as_str().to_owned(), self.root.join(&row.path)));
        }
        workspaces
            .into_iter()
            .filter_map(|(label, workspace)| {
                let path = match target {
                    RepoKey::Root => Some(workspace),
                    RepoKey::Member { id } => artifact::read_manifest(&workspace)
                        .ok()?
                        .members
                        .iter()
                        .find(|member| &member.id == id)
                        .map(|member| workspace.join(&member.path)),
                };
                path.map(|path| (label, path))
            })
            .collect()
    }
}

impl DisposalPorts for CoreDisposalPorts {
    fn observe_target(&mut self, target: &Path) -> Result<TargetEvidence, PortError> {
        let row = self
            .view
            .members
            .get(&self.name)
            .ok_or_else(|| PortError::Evidence {
                detail: format!("no family row named `{}`", self.name),
            })?;
        let observation = self
            .store
            .observe_member_target(&self.root, &self.view, row);
        if observation == TargetObservation::Missing {
            return Ok(TargetEvidence {
                target: observation,
                repositories: Vec::new(),
                unknown: Vec::new(),
            });
        }
        let mut unknown = Vec::new();
        let repositories = match included_repositories(target, &[]) {
            Ok(repositories) => repositories,
            Err(detail) => {
                unknown.push(UnknownReason::new(UnknownKind::Unreadable, detail));
                return Ok(TargetEvidence {
                    target: observation,
                    repositories: Vec::new(),
                    unknown,
                });
            }
        };
        let evidence = self.gwz_evidence(target);
        let mut observed = Vec::new();
        for repository in repositories {
            let info = match self.inspector.inspect_layout(&repository.path) {
                Ok(info) => info,
                Err(error) => {
                    unknown.push(UnknownReason::new(
                        UnknownKind::UnsupportedLayout,
                        format!("{}: {error}", repository.key),
                    ));
                    continue;
                }
            };
            let gwz = if repository.key == RepoKey::Root {
                evidence.clone()
            } else {
                GwzEvidence::default()
            };
            observed.push(RepositoryEvidence {
                key: repository.key,
                work: self.inspector.observe_work(&info),
                history: self.inspector.inventory_history(&info),
                info,
                gwz,
            });
        }
        Ok(TargetEvidence {
            target: observation,
            repositories: observed,
            unknown,
        })
    }

    fn check_history(&mut self, query: &HistoryQuery) -> HistoryAnswer {
        // I-2: an incomplete inventory is an unknown one.
        if !query.protected.is_complete() {
            return HistoryAnswer::Unknown {
                reasons: query.protected.unknown.clone(),
            };
        }
        let witnesses = self.witnesses(&query.target);
        if witnesses.is_empty() {
            return HistoryAnswer::Unpreserved {
                detail: format!(
                    "no surviving family repository is paired with {}",
                    query.target
                ),
            };
        }
        let mut uncovered: Vec<&ProtectedRoot> = query.protected.roots.iter().collect();
        let mut unknown: Vec<UnknownReason> = Vec::new();
        for (label, path) in witnesses {
            if uncovered.is_empty() {
                break;
            }
            let info = match self.inspector.inspect_layout(&path) {
                Ok(info) => info,
                Err(error) => {
                    unknown.push(UnknownReason::new(
                        UnknownKind::Unreadable,
                        format!("witness `{label}` at {}: {error}", path.display()),
                    ));
                    continue;
                }
            };
            let reader = LocalObjectReader::open(&info);
            let witness = Witness {
                repository: query.target.clone(),
                label: format!("{label} ({})", path.display()),
            };
            // One call, one witness store: this reader serves exactly one
            // surviving repository's objects.
            match check_history(
                &query.protected,
                &[witness],
                &reader,
                Limits::default(),
                &NeverCancelled,
            ) {
                HistoryOutcome::Verified(_) => uncovered.clear(),
                HistoryOutcome::Unpreserved(items) => {
                    let missing: Vec<&ObjectId> = items.iter().map(|item| &item.root.oid).collect();
                    uncovered.retain(|root| missing.contains(&&root.oid));
                }
                HistoryOutcome::Unknown(reasons) => unknown.extend(reasons),
            }
        }
        if uncovered.is_empty() {
            HistoryAnswer::Preserved
        } else if !unknown.is_empty() {
            HistoryAnswer::Unknown { reasons: unknown }
        } else {
            HistoryAnswer::Unpreserved {
                detail: format!(
                    "{} protected root(s) of {} are preserved whole in no surviving family \
                     repository: {}",
                    uncovered.len(),
                    query.target,
                    uncovered
                        .iter()
                        .map(|root| format!("{:?} {}", root.source, root.oid))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
    }

    fn remove_directory(&mut self, target: &Path) -> Result<(), RemovalFailure> {
        remove_tree(target)
    }
}
