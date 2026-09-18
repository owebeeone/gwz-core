//! `gwz local dispose <name> [--keep | --force <hazard,...>]` and `gwz
//! local disband` (LCM1.1 wiring, LCM2.1/LCM2.2 ordinary deletion; design
//! §3.1, §5, §5.1, §5.2, §8.4, §8.6).
//!
//! Both dispose paths are `gwz_local_disposal::dispose` over the real ports
//! ([`CoreDisposalPorts`]); the library owns the policy and this module
//! translates and reports.
//!
//! `keep` runs it with `DisposePolicy::Keep`: under the family lock the
//! library validates the name, the root and the working directory, then
//! removes the pointer and the marker and the row -- in that order, the
//! store's only recoverable one -- and consults no port: every file stays,
//! including an incomplete or interrupted tree the ordinary path would
//! refuse (design §5.2 step 2, §12 "Dirty/open/incomplete lane with keep").
//!
//! `delete` runs it with `DisposePolicy::Delete` and the operator's named
//! waivers: after the same validation the library takes fresh evidence of
//! every repository in the deletion tree, classifies the work, asks the
//! surviving family repositories whether every protected root is preserved
//! whole, and refuses on any known hazard not named by `--force` and on any
//! unknown evidence whatever was named (the operator's standing default,
//! design §5); only then does it write `disposing`, remove the validated
//! directory once, and remove the row. An absent target is the stale-row
//! exit (§5.2 step 5); an interrupted removal stops and is reported, never
//! replayed. Every refusal is rendered here with the typed code
//! (`errors::dispose_error_code`), every finding, and the recovery.
//!
//! `disband` is core's own composition over the store session, as the
//! disposal crate's documentation requires (it has no entry point for it):
//! under the family lock, every member's pointer and marker are removed
//! through `remove_pointer`, then `FamilyChange::Disband` removes the index.
//! Every tree stays. It is repeatable on explicit invocation: a pointer
//! already gone is not an error, a family already disbanded is a no-op,
//! and a pointer the store cannot remove stops it with the index intact so
//! an explicit repeat can finish (design §3.1 "interrupted pointer-only
//! detach/disband").

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gwz_family_model::{FamilyChange, FamilyId, FamilyView, MemberName};
use gwz_family_store::YamlFamilyStore;
use gwz_family_store_contract::{
    FamilyLocation, FamilyObservation, FamilySession, FamilyStore, MetadataEffect, StoreError,
};
use gwz_local_disposal::{
    CategoryReport, DisposeEffect, DisposeError, DisposeFailure, DisposePolicy, DisposeReport,
    DisposeRequest, HazardFinding, HazardWaiver, categorise, dispose, required_waivers,
};
use gwz_repo_contract::UnknownReason;

use super::adapters::disposal::CoreDisposalPorts;
use super::adapters::install::OpenMergeProbe;
use super::errors;
use super::family_merge::family_store;
use super::wait::lock_family;
use crate::model::{ErrorCode, ModelError, ModelResult};

/// Findings and reasons past this many items are counted, not listed, so a
/// refusal over a large tree stays one readable line.
const MAX_LISTED: usize = 16;

/// What `dispose --keep` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeepReport {
    /// The detached member's recorded path, resolved against the root.
    pub retained_at: PathBuf,
    pub effects: Vec<DisposeEffect>,
}

impl KeepReport {
    pub fn message(&self, name: &MemberName) -> String {
        let pointer = if self.effects.contains(&DisposeEffect::PointerRemoved) {
            "its pointer and marker removed"
        } else {
            "no pointer of this family stood there"
        };
        format!(
            "detached local clone `{name}`: row removed, {pointer}; every file at {} is retained",
            self.retained_at.display()
        )
    }
}

/// What ordinary `dispose` did: the directory removed and the row with it,
/// or -- when nothing stood at the recorded path -- the stale row alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteReport {
    /// The validated target: the recorded path resolved against the root.
    pub target: PathBuf,
    pub effects: Vec<DisposeEffect>,
    /// The hazards the operator named, in the order given.
    pub waivers: Vec<HazardWaiver>,
    /// The findings the fresh inspection raised under a named waiver: what
    /// the deletion was actually forced past (decision D9).
    pub waived: Vec<HazardFinding>,
}

impl DeleteReport {
    pub fn message(&self, name: &MemberName) -> String {
        let mut message = if self.effects.contains(&DisposeEffect::DirectoryRemoved) {
            format!(
                "deleted local clone `{name}`: {} removed, its row removed",
                self.target.display()
            )
        } else {
            format!(
                "removed the stale row of local clone `{name}`: nothing stood at {}; no file \
                 was removed",
                self.target.display()
            )
        };
        // D9: what was forced past is what the inspection raised and a
        // named waiver covered -- a hazard that would have refused -- in the
        // waiver vocabulary's own order, not the names the operator gave.
        let used = required_waivers(&self.waived);
        if !used.is_empty() {
            let _ = write!(message, "; forced past: {}", waiver_names(&used));
        }
        let unused: Vec<HazardWaiver> = self
            .waivers
            .iter()
            .copied()
            .filter(|waiver| !used.contains(waiver))
            .collect();
        if !unused.is_empty() {
            let _ = write!(message, "; unused waiver: {}", waiver_names(&unused));
        }
        message
    }
}

/// The wire spelling of each waiver, comma-separated.
fn waiver_names(waivers: &[HazardWaiver]) -> String {
    waivers
        .iter()
        .map(|waiver| waiver.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The family, opened for one dispose of `name`: the observation, then the
/// lock, then the locked truth.
struct Opened {
    session: <YamlFamilyStore as FamilyStore>::Session,
    /// The registering root, canonical.
    root: PathBuf,
    view: FamilyView,
    /// The member's recorded path resolved against the root (the name
    /// itself when the row is absent, for the refusal's message).
    target: PathBuf,
}

fn open(
    what: &str,
    workspace: &Path,
    name: &MemberName,
    wait: Option<Duration>,
) -> ModelResult<Opened> {
    let store = family_store();
    let workspace = canonical(workspace)?;
    if store
        .read_view(&FamilyLocation::new(&workspace))
        .map_err(|error| errors::store_in(what, &error))?
        .view()
        .is_none()
    {
        return Err(errors::store_in(
            what,
            &StoreError::NoFamily {
                workspace: workspace.clone(),
            },
        ));
    }
    // R21: a busy lock is retried until `wait` is spent (and refused at
    // once without one), and the `reread` below is what makes a wait safe:
    // the locked truth is read after the wait, never before it.
    let mut session = lock_family(&store, &FamilyLocation::new(&workspace), wait)
        .map_err(|error| errors::store_in(what, &error))?;
    let root = canonical(session.root())?;
    let view = session
        .reread()
        .map_err(|error| errors::store_in(what, &error))?
        .ok_or_else(|| {
            errors::store_in(
                what,
                &StoreError::NoFamily {
                    workspace: root.clone(),
                },
            )
        })?;
    let target = view
        .members
        .get(name)
        .map(|row| normalised(&root.join(&row.path)))
        .unwrap_or_else(|| root.join(name.as_str()));
    Ok(Opened {
        session,
        root,
        view,
        target,
    })
}

/// The recorded path resolved against the canonical root, lexically -- the
/// same resolution the library validates with -- so a message names
/// `/lanes/root-A`, not `/lanes/root/../root-A`.
fn normalised(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match resolved.components().next_back() {
                Some(Component::Normal(_)) => {
                    resolved.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => resolved.push(".."),
            },
            other => resolved.push(other),
        }
    }
    resolved
}

/// The invoking process's working directory, canonical when it resolves,
/// so the library's lexical containment check holds.
fn working_directory(start: &Path) -> PathBuf {
    fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf())
}

/// Detach `name`, retaining every file.
pub(crate) fn keep(
    start: &Path,
    workspace: &Path,
    name: &MemberName,
    wait: Option<Duration>,
    open_merge: OpenMergeProbe,
) -> ModelResult<KeepReport> {
    const WHAT: &str = "local dispose --keep";
    let Opened {
        mut session,
        root,
        view,
        target,
    } = open(WHAT, workspace, name, wait)?;
    let request = DisposeRequest {
        name: name.clone(),
        policy: DisposePolicy::Keep,
        root: root.clone(),
        cwd: working_directory(start),
    };
    let mut ports = CoreDisposalPorts::new(root, view, name.clone(), open_merge);
    match dispose(&request, &mut session, &mut ports) {
        Ok(DisposeReport { effects, .. }) => Ok(KeepReport {
            retained_at: target,
            effects,
        }),
        Err(failure) => Err(failure_error(
            &format!("local dispose `{name}` --keep at {}", target.display()),
            name,
            &failure,
        )),
    }
}

/// Delete `name` after fresh checks, with the operator's named waivers.
pub(crate) fn delete(
    start: &Path,
    workspace: &Path,
    name: &MemberName,
    waivers: &[HazardWaiver],
    wait: Option<Duration>,
    open_merge: OpenMergeProbe,
) -> ModelResult<DeleteReport> {
    const WHAT: &str = "local dispose";
    let Opened {
        mut session,
        root,
        view,
        target,
    } = open(WHAT, workspace, name, wait)?;
    let request = DisposeRequest {
        name: name.clone(),
        policy: DisposePolicy::Delete {
            waivers: waivers.to_vec(),
        },
        root: root.clone(),
        cwd: working_directory(start),
    };
    let mut ports = CoreDisposalPorts::new(root, view, name.clone(), open_merge);
    match dispose(&request, &mut session, &mut ports) {
        Ok(DisposeReport { effects, waived }) => Ok(DeleteReport {
            target,
            effects,
            waivers: waivers.to_vec(),
            waived,
        }),
        Err(failure) => Err(failure_error(
            &format!("local dispose `{name}` at {}", target.display()),
            name,
            &failure,
        )),
    }
}

/// What `disband` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisbandReport {
    pub family_id: FamilyId,
    /// Members whose pointer was removed.
    pub pointers_removed: Vec<String>,
    /// Members whose allocation marker was removed.
    pub markers_removed: Vec<String>,
    /// Members at whose recorded path no pointer of this family stood
    /// (already detached, directory gone, or foreign metadata retained).
    pub nothing_to_remove: Vec<String>,
    pub index_removed: bool,
}

impl DisbandReport {
    pub fn message(&self) -> String {
        format!(
            "disbanded local family {}: {} pointer(s) and {} marker(s) removed, {} member(s) \
             held none, index {}; every tree is retained",
            self.family_id,
            self.pointers_removed.len(),
            self.markers_removed.len(),
            self.nothing_to_remove.len(),
            if self.index_removed {
                "removed"
            } else {
                "retained"
            }
        )
    }
}

/// Remove every member's pointer and marker, then the index. `None` when
/// the workspace is in no family (a repeat after a completed disband).
pub(crate) fn disband(
    workspace: &Path,
    wait: Option<Duration>,
) -> ModelResult<Option<DisbandReport>> {
    const WHAT: &str = "local disband";
    let store = family_store();
    let workspace = canonical(workspace)?;
    match store
        .read_view(&FamilyLocation::new(&workspace))
        .map_err(|error| errors::store_in(WHAT, &error))?
    {
        FamilyObservation::NoFamily => return Ok(None),
        FamilyObservation::Family { .. } => {}
    }
    let mut session = lock_family(&store, &FamilyLocation::new(&workspace), wait)
        .map_err(|error| errors::store_in(WHAT, &error))?;
    let Some(view) = session
        .reread()
        .map_err(|error| errors::store_in(WHAT, &error))?
    else {
        return Ok(None);
    };
    let mut report = DisbandReport {
        family_id: view.family_id.clone(),
        pointers_removed: Vec::new(),
        markers_removed: Vec::new(),
        nothing_to_remove: Vec::new(),
        index_removed: false,
    };
    for name in view.members.keys() {
        let applied = session.remove_pointer(name).map_err(|error| {
            ModelError::new(
                errors::store(&error).code,
                format!(
                    "{WHAT}: removing member `{name}`'s pointer stopped: {error}; the index is \
                     retained and an explicit repeat may finish the disband"
                ),
            )
        })?;
        let mut removed_anything = false;
        for effect in &applied.effects {
            match effect {
                MetadataEffect::PointerRemoved { .. } => {
                    removed_anything = true;
                    report.pointers_removed.push(name.as_str().to_owned());
                }
                MetadataEffect::MarkerRemoved { .. } => {
                    removed_anything = true;
                    report.markers_removed.push(name.as_str().to_owned());
                }
                _ => {}
            }
        }
        if !removed_anything {
            report.nothing_to_remove.push(name.as_str().to_owned());
        }
    }
    session
        .apply(&FamilyChange::Disband)
        .map_err(|error| errors::store_in(WHAT, &error))?;
    report.index_removed = true;
    Ok(Some(report))
}

fn canonical(path: &Path) -> ModelResult<PathBuf> {
    fs::canonicalize(path).map_err(|error| {
        ModelError::new(
            ErrorCode::IoError,
            format!("{} does not resolve: {error}", path.display()),
        )
    })
}

/// A `DisposeFailure` as a `ModelError`: the code follows the typed cause
/// (`errors::dispose_error_code`), the message names the cause, every
/// finding, the recovery and every completed effect.
fn failure_error(context: &str, name: &MemberName, failure: &DisposeFailure) -> ModelError {
    ModelError::new(
        errors::dispose_error_code(&failure.error),
        format!(
            "{context}: {}; effects: {:?}",
            describe(&failure.error, name, &failure.effects),
            failure.effects
        ),
    )
}

/// The cause and its recovery, for the operator.
fn describe(error: &DisposeError, name: &MemberName, effects: &[DisposeEffect]) -> String {
    match error {
        DisposeError::Hazards(findings) => render_hazards(name, findings),
        DisposeError::Unknown(reasons) => format!(
            "unknown evidence: {}; no force name waives unknown evidence: make it \
             interpretable, or --keep to detach and retain every file; nothing was removed",
            render_reasons(reasons)
        ),
        DisposeError::RemovalStopped { remaining, detail } => format!(
            "removal stopped ({detail}); remaining: {}; the row is `disposing` and the \
             remainder is retained for inspection; there is no replay and a repeat is \
             refused: clean up by hand, then an explicit dispose removes the stale row, or \
             --keep detaches the remainder",
            remaining
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        DisposeError::Refused(gwz_family_model::Refusal::WrongState { .. }) => format!(
            "{error}; ordinary deletion needs an intact ready target and is not forced past \
             an incomplete or interrupted one; nothing was removed; --keep detaches it and \
             retains every file"
        ),
        DisposeError::PathMismatch { .. } => format!(
            "{error}; nothing was removed; `gwz local list` shows what was observed at the \
             recorded path; --keep detaches the row and retains every file"
        ),
        DisposeError::Store(_) if effects.contains(&DisposeEffect::DirectoryRemoved) => format!(
            "{error}; the directory is gone and the row is `disposing`: an explicit dispose \
             removes the stale row"
        ),
        other => other.to_string(),
    }
}

/// R9 and R10: what refused, by category, with each category's count and
/// its paths or object ids -- empty categories included, so the refusal
/// says what it did *not* find too -- and then the **exact** command that
/// waives exactly what was found, not a generic hint.
fn render_hazards(name: &MemberName, findings: &[HazardFinding]) -> String {
    let categories = categorise(findings);
    let waivers = required_waivers(findings);
    format!(
        "unwaived hazard(s) by category: {}; to delete anyway, naming every loss it waives: \
         `gwz local dispose {name} --force {}`; or --keep to detach and retain every file; \
         nothing was removed",
        categories
            .iter()
            .map(render_category)
            .collect::<Vec<_>>()
            .join("; "),
        waivers
            .iter()
            .map(|waiver| waiver.as_str())
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// `` <category> <count>: `<repository>` <detail> (<path>), ... ``, or
/// `` <category> 0 `` when the category is empty.
fn render_category(report: &CategoryReport) -> String {
    let name = report.category.as_str();
    let count = report.count();
    if count == 0 {
        return format!("{name} 0");
    }
    let items: Vec<String> = report
        .items
        .iter()
        .map(|item| match &item.path {
            Some(path) => format!(
                "`{}` {} ({})",
                item.repository,
                item.detail,
                String::from_utf8_lossy(path)
            ),
            None => format!("`{}` {}", item.repository, item.detail),
        })
        .collect();
    format!("{name} {count}: {}", listed(&items))
}

fn render_reasons(reasons: &[UnknownReason]) -> String {
    let items: Vec<String> = reasons
        .iter()
        .map(|reason| match &reason.path {
            Some(path) => format!(
                "{:?} `{}`: {}",
                reason.kind,
                String::from_utf8_lossy(path),
                reason.detail
            ),
            None => format!("{:?}: {}", reason.kind, reason.detail),
        })
        .collect();
    listed(&items)
}

fn listed(items: &[String]) -> String {
    if items.len() <= MAX_LISTED {
        return items.join(", ");
    }
    format!(
        "{}, and {} more",
        items[..MAX_LISTED].join(", "),
        items.len() - MAX_LISTED
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_local_disposal::PortError;
    use gwz_repo_contract::{RepoKey, UnknownKind};
    use gwz_work_detector::{Hazard, HazardKind, Provenance};

    fn name() -> MemberName {
        MemberName::parse("A").unwrap()
    }

    fn work_finding(waiver: HazardWaiver, provenance: Provenance) -> HazardFinding {
        HazardFinding {
            waiver,
            repository: RepoKey::Root,
            hazards: vec![Hazard {
                kind: HazardKind::Work(gwz_repo_contract::WorkKind::Untracked),
                path: Some(b"notes.txt".to_vec()),
                detail: "untracked (text)".to_owned(),
                provenance,
            }],
            detail: None,
        }
    }

    fn unpreserved() -> HazardFinding {
        HazardFinding {
            waiver: HazardWaiver::UnpreservedHistory,
            repository: RepoKey::Root,
            hazards: Vec::new(),
            detail: Some("1 protected root(s) are preserved nowhere".to_owned()),
        }
    }

    /// The success messages: a deletion names the target, the row and the
    /// hazards it was actually forced past (D9) -- never a waiver that was
    /// named and waived nothing, which is called unused instead; a
    /// stale-row removal says no file went.
    #[test]
    fn a_delete_report_names_what_went_and_what_was_forced() {
        let deleted = DeleteReport {
            target: PathBuf::from("/fam/ws-A"),
            effects: vec![
                DisposeEffect::RowDisposing,
                DisposeEffect::DirectoryRemoved,
                DisposeEffect::PointerRemoved,
                DisposeEffect::RowRemoved,
            ],
            waivers: vec![HazardWaiver::UnpreservedHistory, HazardWaiver::Dirty],
            waived: vec![
                unpreserved(),
                work_finding(HazardWaiver::Dirty, Provenance::Unique),
            ],
        };
        assert_eq!(
            deleted.message(&name()),
            "deleted local clone `A`: /fam/ws-A removed, its row removed; forced past: \
             dirty, unpreserved-history"
        );
        // The probe's case (F6): both names given, only the dirty work
        // raised. The history was preserved, so it was not forced past.
        let one_raised = DeleteReport {
            waived: vec![work_finding(HazardWaiver::Dirty, Provenance::Unique)],
            ..deleted.clone()
        };
        assert_eq!(
            one_raised.message(&name()),
            "deleted local clone `A`: /fam/ws-A removed, its row removed; forced past: \
             dirty; unused waiver: unpreserved-history"
        );
        // A finding that refuses nothing needed no waiver, so naming its
        // waiver forced past nothing.
        let none_raised = DeleteReport {
            waivers: vec![HazardWaiver::Dirty, HazardWaiver::OpenMerge],
            waived: vec![work_finding(HazardWaiver::Dirty, Provenance::UnchangedCopy)],
            ..deleted.clone()
        };
        assert_eq!(
            none_raised.message(&name()),
            "deleted local clone `A`: /fam/ws-A removed, its row removed; unused waiver: \
             dirty, open-merge"
        );
        let plain = DeleteReport {
            waivers: Vec::new(),
            waived: Vec::new(),
            ..deleted.clone()
        };
        assert_eq!(
            plain.message(&name()),
            "deleted local clone `A`: /fam/ws-A removed, its row removed"
        );
        let stale = DeleteReport {
            effects: vec![DisposeEffect::PointerRemoved, DisposeEffect::RowRemoved],
            ..plain
        };
        assert_eq!(
            stale.message(&name()),
            "removed the stale row of local clone `A`: nothing stood at /fam/ws-A; no file \
             was removed"
        );
    }

    /// R9 and R10: a refusal separates the categories it found -- naming
    /// the empty ones too -- with each one's count and its paths or object
    /// ids, then prints the **exact** command that waives exactly what
    /// refused, and says nothing was removed. Long lists are counted past
    /// the cap, never dropped silently.
    #[test]
    fn a_refusal_reports_by_category_and_prints_the_exact_waiver_command() {
        let findings = vec![
            HazardFinding {
                waiver: HazardWaiver::Dirty,
                repository: RepoKey::Member {
                    id: "mem_app".to_owned(),
                },
                hazards: vec![
                    Hazard {
                        kind: HazardKind::Work(gwz_repo_contract::WorkKind::Untracked),
                        path: Some(b"notes.txt".to_vec()),
                        detail: "untracked (text)".to_owned(),
                        provenance: Provenance::Unique,
                    },
                    Hazard {
                        kind: HazardKind::Work(gwz_repo_contract::WorkKind::Ignored),
                        path: Some(b"target/".to_vec()),
                        detail: "ignored user data".to_owned(),
                        provenance: Provenance::UnchangedCopy,
                    },
                    Hazard {
                        kind: HazardKind::Work(gwz_repo_contract::WorkKind::Ignored),
                        path: Some(b".venv/".to_vec()),
                        detail: "ignored user data".to_owned(),
                        provenance: Provenance::ChangedCopy,
                    },
                    Hazard {
                        kind: HazardKind::NativeStash,
                        path: None,
                        detail: "1 native stash entry".to_owned(),
                        provenance: Provenance::Unique,
                    },
                ],
                detail: None,
            },
            HazardFinding {
                waiver: HazardWaiver::UnpreservedHistory,
                repository: RepoKey::Root,
                hazards: Vec::new(),
                detail: Some(
                    "1 protected root(s) of @root are preserved whole in no \
                              surviving family repository: Head 0123abcd"
                        .to_owned(),
                ),
            },
        ];
        let error = failure_error(
            "local dispose `A` at /fam/ws-A",
            &name(),
            &DisposeFailure {
                error: DisposeError::Hazards(findings),
                effects: Vec::new(),
            },
        );
        assert_eq!(error.code, ErrorCode::UnwaivedHazard);
        assert_eq!(
            error.message,
            "local dispose `A` at /fam/ws-A: unwaived hazard(s) by category: regenerable 0; \
             unchanged copy 1: `mem_app` ignored user data (target/); changed copy 1: \
             `mem_app` ignored user data (.venv/); unique to the lane 3: `mem_app` untracked \
             (text) (notes.txt), `mem_app` 1 native stash entry, `@root` 1 protected root(s) \
             of @root are preserved whole in no surviving family repository: Head 0123abcd; \
             to delete anyway, naming every loss it waives: `gwz local dispose A \
             --force dirty,unpreserved-history`; or --keep to detach and retain every file; \
             nothing was removed; effects: []"
        );

        let reasons: Vec<UnknownReason> = (0..MAX_LISTED + 2)
            .map(|index| UnknownReason {
                kind: UnknownKind::Unreadable,
                path: Some(format!("path-{index}").into_bytes()),
                detail: "unreadable".to_owned(),
            })
            .collect();
        let error = failure_error(
            "local dispose `A` at /fam/ws-A",
            &name(),
            &DisposeFailure {
                error: DisposeError::Unknown(reasons),
                effects: Vec::new(),
            },
        );
        assert_eq!(error.code, ErrorCode::UnknownEvidence);
        assert!(
            error.message.contains("Unreadable `path-0`: unreadable"),
            "{}",
            error.message
        );
        assert!(error.message.contains(", and 2 more;"), "{}", error.message);
        assert!(
            !error.message.contains("path-16"),
            "the cap counts, it does not list: {}",
            error.message
        );
        assert!(
            error
                .message
                .contains("no force name waives unknown evidence"),
            "{}",
            error.message
        );

        let error = failure_error(
            "local dispose `A` at /fam/ws-A",
            &name(),
            &DisposeFailure {
                error: DisposeError::RemovalStopped {
                    remaining: vec![PathBuf::from("/fam/ws-A"), PathBuf::from("/fam/ws-A/held")],
                    detail: "/fam/ws-A/held: Permission denied".to_owned(),
                },
                effects: vec![DisposeEffect::RowDisposing],
            },
        );
        assert_eq!(error.code, ErrorCode::DisposalIncomplete);
        assert!(
            error
                .message
                .contains("remaining: /fam/ws-A, /fam/ws-A/held"),
            "{}",
            error.message
        );
        assert!(error.message.contains("no replay"), "{}", error.message);
        assert!(
            error.message.ends_with("effects: [RowDisposing]"),
            "{}",
            error.message
        );

        let error = failure_error(
            "local dispose `A` at /fam/ws-A",
            &name(),
            &DisposeFailure {
                error: DisposeError::Port(PortError::Unimplemented { operation: "x" }),
                effects: Vec::new(),
            },
        );
        assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    }
}
