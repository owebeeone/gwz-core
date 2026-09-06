//! `gwz_local_disposal::DisposalPorts` over the real libraries: the store
//! and the inspector behind `observe_target`, `gwz-history-check` behind
//! `check_history`, and the ordinary recursive remover behind
//! `remove_directory`.
//!
//! # What `observe_target` covers (design §5.1; LCM2.1)
//!
//! Every repository in the deletion tree -- the root, every manifest
//! member, every unmanaged nested repository (a `.git` entry or a bare Git
//! directory) -- through [`included_repositories`], each inspected with
//! `gwz-repo-inspect`: layout, work (staged, unstaged, untracked and
//! ignored entries; status-suppressed paths compared byte for byte; valid
//! sparse absence recorded explicitly; the unfinished native operation;
//! native stash entries) and the history inventory (every ref, HEAD,
//! reflog-only objects, annotated tags, stash entries). A repository whose
//! layout the inspector refuses or cannot read is an [`UnknownReason`] in
//! `TargetEvidence::unknown`, never omitted, and the library refuses on it.
//!
//! **Structural entries are not work.** A workspace root's own status
//! reports GWZ's runtime directory (`.gwz/`, the family pointer and marker,
//! the merge store, the stash records) and every separately inventoried
//! repository beneath it (a member, a nested repository) as ignored or
//! untracked entries, because that is how the managed exclude block and
//! Git's own nested-repository rule present them. Those entries are removed
//! from the root's observation here: the runtime directory is GWZ's, not
//! user data, and its coordination records reach the classifier through the
//! GWZ evidence channel instead; a repository beneath is inspected on its
//! own. Nothing else is filtered -- "ignored does not mean disposable" holds
//! for every ignored entry a user put there.
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
//! same manifest id for `RepoKey::Member`; a nested repository has no pair
//! and is therefore unpreserved until the operator names the loss.
//!
//! # GWZ evidence
//!
//! Core decodes GWZ evidence for the work detector. This build reads the
//! merge store's envelope (through the dispatch slot's probe) and the
//! presence of stash records: an open merge is `Open`; present stash
//! records are `Unknown` -- decoding them into coordination roots and
//! checking their surviving copies is deferred (LCM2.1 remainder) -- so a
//! target carrying one refuses ordinary deletion, with a message that says
//! why and that `--keep` still detaches, rather than being read as clean.

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
    LayoutError, ObjectId, Observation, ProtectedRoot, RepoInspector, RepoKey, UnknownKind,
    UnknownReason, WorkObservation,
};
use gwz_repo_inspect::{LocalObjectReader, LocalRepoInspector};
use gwz_work_detector::{EvidenceState, GwzEvidence};

use super::install::OpenMergeProbe;
use super::inventory::{IncludedRepository, included_repositories};
use super::removal::remove_tree;
use crate::artifact;
use crate::workspace::{RUNTIME_DIR, WORKSPACE_DIR};

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
                            "{records} gwz stash record(s) under {}: this build does not decode \
                             gwz stash coordination records (design §5.1 needs their surviving \
                             copies and referenced objects verified), so the lane cannot be \
                             deleted while any exists; pop or drop the gwz stash in the lane \
                             first, or `gwz local dispose {} --keep` detaches the lane and \
                             retains every file",
                            bundles.display(),
                            self.name
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

/// The unknown kind an uninspectable repository is reported under: a
/// hazard or a non-repository is a layout the deletion cannot reason about,
/// a read failure is unreadable evidence, an inspector that does not inspect
/// is a build gap. None of them is ever omitted.
fn layout_unknown_kind(error: &LayoutError) -> UnknownKind {
    match error {
        LayoutError::Unsupported { .. } | LayoutError::NotARepository { .. } => {
            UnknownKind::UnsupportedLayout
        }
        LayoutError::ReadFailed { .. } => UnknownKind::Unreadable,
        LayoutError::Unimplemented { .. } => UnknownKind::Unimplemented,
    }
}

/// The worktree-relative paths inside `repository` that are GWZ's own
/// structure rather than its work: the runtime directory and the
/// manifest's scratch directory (root only), and every other inventoried
/// repository beneath it, which is inspected on its own.
fn structural_paths(repository: &IncludedRepository, all: &[IncludedRepository]) -> Vec<PathBuf> {
    let mut structural = Vec::new();
    if repository.key == RepoKey::Root {
        structural.push(PathBuf::from(RUNTIME_DIR));
        structural.push(Path::new(WORKSPACE_DIR).join(".tmp"));
    }
    for other in all {
        if other.relative == repository.relative {
            continue;
        }
        if let Ok(below) = other.relative.strip_prefix(&repository.relative)
            && !below.as_os_str().is_empty()
        {
            structural.push(below.to_path_buf());
        }
    }
    structural
}

/// Drop the work entries that name a structural path or anything beneath
/// one. Suppressed entries, sparse absences and the per-path unknowns are
/// tracked paths and are never structural, so they are left as observed.
fn strip_structural_work(work: &mut Observation<WorkObservation>, structural: &[PathBuf]) {
    let Observation::Known(known) = work else {
        return;
    };
    let structural: Vec<Vec<u8>> = structural
        .iter()
        .map(|path| {
            path.components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
                .into_bytes()
        })
        .collect();
    known.entries.retain(|entry| {
        let path = entry
            .path
            .strip_suffix(b"/")
            .unwrap_or(entry.path.as_slice());
        !structural.iter().any(|prefix| {
            path == prefix.as_slice()
                || (path.len() > prefix.len()
                    && path.starts_with(prefix)
                    && path[prefix.len()] == b'/')
        })
    });
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
        for repository in &repositories {
            let info = match self.inspector.inspect_layout(&repository.path) {
                Ok(info) => info,
                Err(error) => {
                    unknown.push(UnknownReason::new(
                        layout_unknown_kind(&error),
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
            let mut work = self.inspector.observe_work(&info);
            strip_structural_work(&mut work, &structural_paths(repository, &repositories));
            observed.push(RepositoryEvidence {
                key: repository.key.clone(),
                work,
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

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_repo_contract::{WorkEntry, WorkKind};

    fn included(key: RepoKey, relative: &str, bare: bool) -> IncludedRepository {
        IncludedRepository {
            key,
            relative: PathBuf::from(relative),
            path: PathBuf::from("/ws").join(relative),
            bare,
        }
    }

    fn entry(path: &[u8], kind: WorkKind) -> WorkEntry {
        WorkEntry {
            path: path.to_vec(),
            kind,
            binary: Some(false),
        }
    }

    /// The root's structural set is the runtime directory, the manifest's
    /// scratch directory and every repository beneath it; a member's is
    /// only the nested repositories beneath *it*; a nested repository's is
    /// empty. Entries at or under a structural path go; every other entry
    /// -- ignored included -- stays.
    #[test]
    fn structural_entries_are_stripped_and_every_other_entry_stays() {
        let all = vec![
            included(RepoKey::Root, "", false),
            included(
                RepoKey::Member {
                    id: "mem_app".to_owned(),
                },
                "app",
                false,
            ),
            included(
                RepoKey::Member {
                    id: "nested:app/vendor/thing".to_owned(),
                },
                "app/vendor/thing",
                false,
            ),
            included(
                RepoKey::Member {
                    id: "nested:vendor/mirror.git".to_owned(),
                },
                "vendor/mirror.git",
                true,
            ),
        ];
        assert_eq!(
            structural_paths(&all[0], &all),
            vec![
                PathBuf::from(".gwz"),
                PathBuf::from("gwz.conf/.tmp"),
                PathBuf::from("app"),
                PathBuf::from("app/vendor/thing"),
                PathBuf::from("vendor/mirror.git"),
            ]
        );
        assert_eq!(
            structural_paths(&all[1], &all),
            vec![PathBuf::from("vendor/thing")]
        );
        assert!(structural_paths(&all[2], &all).is_empty());
        assert!(structural_paths(&all[3], &all).is_empty());

        let mut work = Observation::Known(WorkObservation {
            entries: vec![
                entry(b".gwz/", WorkKind::Ignored),
                entry(b".gwzx", WorkKind::Untracked),
                entry(b"app/", WorkKind::Ignored),
                entry(b"application.txt", WorkKind::Untracked),
                entry(b"vendor/mirror.git/HEAD", WorkKind::Untracked),
                entry(b"vendor/mirror.git/objects/ab/cd", WorkKind::Untracked),
                entry(b"vendor/notes.txt", WorkKind::Untracked),
                entry(b"build/", WorkKind::Ignored),
                entry(b"README", WorkKind::Unstaged),
            ],
            ..WorkObservation::default()
        });
        strip_structural_work(&mut work, &structural_paths(&all[0], &all));
        let kept: Vec<&[u8]> = work
            .known()
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.path.as_slice())
            .collect();
        assert_eq!(
            kept,
            vec![
                &b".gwzx"[..],
                b"application.txt",
                b"vendor/notes.txt",
                b"build/",
                b"README",
            ],
            "a user's ignored build tree and every other entry stay"
        );

        // An unknown observation is left alone: it refuses as it is.
        let mut unknown =
            Observation::Unknown(vec![UnknownReason::new(UnknownKind::Unreadable, "x")]);
        strip_structural_work(&mut unknown, &structural_paths(&all[0], &all));
        assert!(unknown.is_unknown());
    }

    #[test]
    fn every_layout_error_is_an_unknown_reason_of_its_own_kind() {
        let path = PathBuf::from("/ws/app");
        assert_eq!(
            layout_unknown_kind(&LayoutError::Unsupported {
                path: path.clone(),
                hazards: Vec::new()
            }),
            UnknownKind::UnsupportedLayout
        );
        assert_eq!(
            layout_unknown_kind(&LayoutError::NotARepository { path: path.clone() }),
            UnknownKind::UnsupportedLayout
        );
        assert_eq!(
            layout_unknown_kind(&LayoutError::ReadFailed {
                path,
                detail: "EIO".to_owned()
            }),
            UnknownKind::Unreadable
        );
        assert_eq!(
            layout_unknown_kind(&LayoutError::Unimplemented { operation: "x" }),
            UnknownKind::Unimplemented
        );
    }
}
