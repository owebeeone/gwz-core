//! `gwz-work-detector`: pure classification of unsaved work (lane W).
//!
//! [`classify_work`] turns one repository's observed on-disk work
//! (`gwz_repo_contract::WorkObservation`) and core's decoded GWZ evidence
//! ([`GwzEvidence`]) into a [`WorkReport`]: clean, dirty with named hazards,
//! or unknown with reasons. It reads nothing and holds no core or protocol
//! type; core decodes merge/stash records into plain evidence first, and an
//! unsupported or malformed record is [`EvidenceState::Unknown`], never
//! silently clean. Diagnostic truncation never removes a hazard.
//!
//! # What the verdict means
//!
//! - [`WorkVerdict::Clean`]: every supplied input was known and named no
//!   hazard. An empty *known* observation is clean; establishing that an
//!   observation is known at all is the observer's job — never the
//!   classifier's — and [`classify_observed_work`] refuses an unknown one.
//!   "Git status was empty" is not an input this crate can see.
//! - [`WorkVerdict::Dirty`]: at least one hazard, each named with its cause.
//! - [`WorkVerdict::Unknown`]: at least one input was unknown, unsupported
//!   or suppressed without a physical observation. Unknown dominates dirty,
//!   and the hazards found alongside it are still listed.
//!
//! # Status-suppression flags (design §5.1, architecture §4)
//!
//! A tracked path carrying `assume-unchanged`, `skip-worktree` or an index
//! flag the observer does not interpret is classified from its *physical*
//! state, never from status. It is never silently clean:
//!
//! | Physical state | assume-unchanged / skip-worktree | other flag |
//! |---|---|---|
//! | `Differs` | dirty ([`HazardKind::Suppressed`]) | dirty |
//! | `MatchesIndex` | clean | unknown (`UnsupportedIndexFlag`) |
//! | `Absent`, in `sparse_absent` | clean (valid sparse absence) | unknown |
//! | `Absent`, not recorded sparse | assume-unchanged: dirty; skip-worktree: unknown | unknown |
//! | `Unobservable` | unknown (`Unreadable`) | unknown |
//!
//! # Hazard vocabulary and the disposal force names (design §5.2)
//!
//! Every hazard maps one-to-one onto a `gwz local dispose --force` name:
//! [`HazardKind::OpenGwzMerge`], [`HazardKind::OpenGwzRecord`] and
//! [`HazardKind::OpenNativeOperation`] are `open-merge`; [`HazardKind::Work`],
//! [`HazardKind::Suppressed`] and [`HazardKind::NativeStash`] are `dirty`.
//! [`HazardKind::UninterpretableEvidence`] has no force name: it always
//! arrives with an unknown reason, so the verdict is `Unknown` and refusal
//! is not waivable.
//!
//! Hazard and reason order is deterministic: work entries in input order,
//! then suppressed paths, the unfinished native operation, native stashes,
//! the observer's own per-path unknowns (`WorkObservation::unknown`, lane W
//! proposal W1: a known observation whose listed paths could not be
//! established -- each is an unknown reason here, never clean), and finally
//! the GWZ merge, stash and other records.

#![forbid(unsafe_code)]

use std::collections::HashSet;

use gwz_repo_contract::{
    BytePath, NativeOperation, Observation, PhysicalState, SuppressionFlag, UnknownKind,
    UnknownReason, WorkKind, WorkObservation,
};

/// Core-decoded evidence of GWZ coordination state for one repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GwzEvidence {
    /// The workspace's merge record, as it concerns this repository.
    pub merge: EvidenceState,
    /// GWZ stash bundle records naming this repository.
    pub stash: EvidenceState,
    /// Other coordination records core chose to surface.
    pub other: Vec<EvidenceItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum EvidenceState {
    /// No record.
    #[default]
    None,
    /// An open or unfinished record.
    Open { detail: String },
    /// A record exists but could not be interpreted; a hazard, not a pass.
    Unknown { detail: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceItem {
    pub kind: String,
    pub state: EvidenceState,
}

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

/// Classify one repository's known work observation and its decoded GWZ
/// evidence. Pure and deterministic: the same inputs always produce the same
/// report, in the same order.
pub fn classify_work(observation: &WorkObservation, evidence: &GwzEvidence) -> WorkReport {
    let mut report = Builder::default();
    report.observation(observation);
    report.evidence(evidence);
    report.finish()
}

/// Classify an observation that may itself be unknown, as `observe_work`
/// returns it. An unknown observation carries its reasons through unchanged
/// — it is never clean and never dirty — and the evidence is still
/// classified beside it, so an open merge is named even when the work
/// inventory could not be completed.
pub fn classify_observed_work(
    observation: &Observation<WorkObservation>,
    evidence: &GwzEvidence,
) -> WorkReport {
    match observation {
        Observation::Known(known) => classify_work(known, evidence),
        Observation::Unknown(reasons) => {
            let mut report = Builder {
                unknown: reasons.clone(),
                ..Builder::default()
            };
            report.evidence(evidence);
            report.finish()
        }
    }
}

/// Accumulates hazards and unknown reasons in input order.
#[derive(Default)]
struct Builder {
    hazards: Vec<Hazard>,
    unknown: Vec<UnknownReason>,
}

impl Builder {
    fn hazard(&mut self, kind: HazardKind, path: Option<BytePath>, detail: impl Into<String>) {
        self.hazards.push(Hazard {
            kind,
            path,
            detail: detail.into(),
        });
    }

    fn unknown(&mut self, kind: UnknownKind, path: Option<BytePath>, detail: impl Into<String>) {
        self.unknown.push(UnknownReason {
            kind,
            path,
            detail: detail.into(),
        });
    }

    fn observation(&mut self, observation: &WorkObservation) {
        for entry in &observation.entries {
            self.hazard(
                HazardKind::Work(entry.kind),
                Some(entry.path.clone()),
                work_detail(entry.kind, entry.binary),
            );
        }

        let sparse: HashSet<&[u8]> = observation
            .sparse_absent
            .iter()
            .map(Vec::as_slice)
            .collect();
        for entry in &observation.suppressed {
            self.suppressed(entry.flag, entry.physical, &entry.path, &sparse);
        }

        if let Some(operation) = observation.native_operation {
            self.hazard(
                HazardKind::OpenNativeOperation,
                None,
                native_operation_detail(operation),
            );
        }

        if observation.stash_entries > 0 {
            let count = observation.stash_entries;
            let plural = if count == 1 { "y" } else { "ies" };
            self.hazard(
                HazardKind::NativeStash,
                None,
                format!("{count} native stash entr{plural}"),
            );
        }

        // The observer established the inventory except for these paths;
        // each is carried through as it was reported (kind, path, detail).
        self.unknown.extend(observation.unknown.iter().cloned());
    }

    /// One status-suppressed tracked path, classified from its physical
    /// state (never from status). See the table in the crate documentation.
    fn suppressed(
        &mut self,
        flag: SuppressionFlag,
        physical: PhysicalState,
        path: &BytePath,
        sparse: &HashSet<&[u8]>,
    ) {
        let name = flag_name(flag);
        let uninterpreted = flag == SuppressionFlag::Other;
        match physical {
            PhysicalState::Unobservable => self.unknown(
                UnknownKind::Unreadable,
                Some(path.clone()),
                format!("{name} path: the worktree bytes could not be observed"),
            ),
            PhysicalState::Differs => self.hazard(
                HazardKind::Suppressed,
                Some(path.clone()),
                format!("{name} path: the worktree bytes differ from the index"),
            ),
            PhysicalState::MatchesIndex => {
                if uninterpreted {
                    self.unknown(
                        UnknownKind::UnsupportedIndexFlag,
                        Some(path.clone()),
                        format!(
                            "{name}: the bytes match the index, but what else the flag \
                             suppresses is not interpreted"
                        ),
                    );
                }
            }
            PhysicalState::Absent => {
                if uninterpreted {
                    self.unknown(
                        UnknownKind::UnsupportedIndexFlag,
                        Some(path.clone()),
                        format!("{name}: the path is absent and the flag is not interpreted"),
                    );
                } else if sparse.contains(path.as_slice()) {
                    // Valid sparse absence, recorded explicitly: not dirt.
                } else if flag == SuppressionFlag::AssumeUnchanged {
                    self.hazard(
                        HazardKind::Suppressed,
                        Some(path.clone()),
                        format!("{name} path: absent from the worktree"),
                    );
                } else {
                    self.unknown(
                        UnknownKind::UnsupportedIndexFlag,
                        Some(path.clone()),
                        format!("{name} path: absent with no recorded valid sparse absence"),
                    );
                }
            }
        }
    }

    fn evidence(&mut self, evidence: &GwzEvidence) {
        self.record(
            "gwz merge record",
            &evidence.merge,
            HazardKind::OpenGwzMerge,
        );
        self.record(
            "gwz stash record",
            &evidence.stash,
            HazardKind::OpenGwzStash,
        );
        for item in &evidence.other {
            self.record(&item.kind, &item.state, HazardKind::OpenGwzRecord);
        }
    }

    /// One decoded record. An uninterpretable record is a named hazard *and*
    /// an unknown reason: it is never a pass, and it is never waivable.
    fn record(&mut self, label: &str, state: &EvidenceState, open: HazardKind) {
        match state {
            EvidenceState::None => {}
            EvidenceState::Open { detail } => self.hazard(open, None, format!("{label}: {detail}")),
            EvidenceState::Unknown { detail } => {
                self.hazard(
                    HazardKind::UninterpretableEvidence,
                    None,
                    format!("{label}: {detail}"),
                );
                self.unknown(
                    UnknownKind::UnsupportedEvidence,
                    None,
                    format!("{label}: {detail}"),
                );
            }
        }
    }

    fn finish(mut self) -> WorkReport {
        let hazards_cut = drop_details(&mut self.hazards, |hazard| &mut hazard.detail);
        let unknown_cut = drop_details(&mut self.unknown, |reason| &mut reason.detail);
        let verdict = if self.unknown.is_empty() {
            if self.hazards.is_empty() {
                WorkVerdict::Clean
            } else {
                WorkVerdict::Dirty
            }
        } else {
            WorkVerdict::Unknown
        };
        WorkReport {
            verdict,
            hazards: self.hazards,
            unknown: self.unknown,
            truncated: hazards_cut || unknown_cut,
        }
    }
}

/// Cut diagnostic text past [`MAX_DETAILED_ENTRIES`], keeping every entry
/// with its kind and path. Truncation never removes a hazard or a reason.
fn drop_details<T>(items: &mut [T], detail: impl Fn(&mut T) -> &mut String) -> bool {
    if items.len() <= MAX_DETAILED_ENTRIES {
        return false;
    }
    for item in &mut items[MAX_DETAILED_ENTRIES..] {
        detail(item).clear();
    }
    true
}

fn work_detail(kind: WorkKind, binary: Option<bool>) -> String {
    let cause = match kind {
        WorkKind::Staged => "staged change",
        WorkKind::Unstaged => "unstaged change",
        WorkKind::Untracked => "untracked file",
        WorkKind::Ignored => "ignored user data (ignored does not mean disposable)",
        WorkKind::Conflict => "conflict stages in the index",
        WorkKind::ModeChange => "file mode change",
        WorkKind::LinkChange => "symbolic link change",
        WorkKind::Renamed => "rename",
        WorkKind::Deleted => "deletion",
    };
    match binary {
        Some(true) => format!("{cause}, binary content"),
        Some(false) => format!("{cause}, text content"),
        None => cause.to_owned(),
    }
}

fn native_operation_detail(operation: NativeOperation) -> &'static str {
    match operation {
        NativeOperation::Merge => "unfinished native git merge",
        NativeOperation::Rebase => "unfinished native git rebase",
        NativeOperation::CherryPick => "unfinished native git cherry-pick",
        NativeOperation::Revert => "unfinished native git revert",
        NativeOperation::Bisect => "unfinished native git bisect",
        NativeOperation::ApplyMailbox => "unfinished native git mailbox application",
        NativeOperation::Other => "unfinished native git operation",
    }
}

fn flag_name(flag: SuppressionFlag) -> &'static str {
    match flag {
        SuppressionFlag::AssumeUnchanged => "assume-unchanged",
        SuppressionFlag::SkipWorktree => "skip-worktree",
        SuppressionFlag::Other => "uninterpreted index flag",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_repo_contract::{SuppressedEntry, WorkEntry};

    fn path(name: &str) -> BytePath {
        name.as_bytes().to_vec()
    }

    fn entry(name: &str, kind: WorkKind) -> WorkEntry {
        WorkEntry {
            path: path(name),
            kind,
            binary: None,
        }
    }

    fn observation(entries: Vec<WorkEntry>) -> WorkObservation {
        WorkObservation {
            entries,
            ..WorkObservation::default()
        }
    }

    fn suppressed(name: &str, flag: SuppressionFlag, physical: PhysicalState) -> SuppressedEntry {
        SuppressedEntry {
            path: path(name),
            flag,
            physical,
        }
    }

    fn suppressed_observation(entries: Vec<SuppressedEntry>) -> WorkObservation {
        WorkObservation {
            suppressed: entries,
            ..WorkObservation::default()
        }
    }

    fn kinds(report: &WorkReport) -> Vec<HazardKind> {
        report.hazards.iter().map(|h| h.kind.clone()).collect()
    }

    /// W1 (LCM1.0c follow-up 2): a known observation may name paths it
    /// could not establish. Each is an unknown reason in the report, with
    /// its path and detail as reported, so the verdict is `Unknown` -- never
    /// clean -- while the hazards found beside it are still listed.
    #[test]
    fn per_path_unknowns_in_a_known_observation_are_unknown_reasons_beside_the_hazards() {
        let observed = WorkObservation {
            entries: vec![entry("src/a.rs", WorkKind::Unstaged)],
            unknown: vec![UnknownReason {
                kind: UnknownKind::Unreadable,
                path: Some(path("vendor/blob.bin")),
                detail: "the worktree entry could not be read".to_owned(),
            }],
            ..WorkObservation::default()
        };
        let report = classify_work(&observed, &GwzEvidence::default());
        assert_eq!(report.verdict, WorkVerdict::Unknown);
        assert_eq!(kinds(&report), vec![HazardKind::Work(WorkKind::Unstaged)]);
        assert_eq!(unknown_kinds(&report), vec![UnknownKind::Unreadable]);
        assert_eq!(
            report.unknown[0].path.as_deref(),
            Some(b"vendor/blob.bin".as_slice())
        );
        assert!(report.unknown[0].detail.contains("could not be read"));

        // An empty list is the ordinary known observation: nothing changes.
        assert_eq!(
            classify_work(&observation(Vec::new()), &GwzEvidence::default()).verdict,
            WorkVerdict::Clean
        );
    }

    fn unknown_kinds(report: &WorkReport) -> Vec<UnknownKind> {
        report.unknown.iter().map(|reason| reason.kind).collect()
    }

    fn details(report: &WorkReport) -> String {
        format!("{:?} {:?}", report.hazards, report.unknown)
    }

    // --- the all-clean case -------------------------------------------------

    #[test]
    fn an_empty_known_observation_with_no_evidence_is_clean() {
        let report = classify_work(&WorkObservation::default(), &GwzEvidence::default());
        assert_eq!(report.verdict, WorkVerdict::Clean);
        assert!(report.hazards.is_empty());
        assert!(report.unknown.is_empty());
        assert!(!report.truncated);
        assert_eq!(report, WorkReport::clean());
    }

    // --- on-disk work -------------------------------------------------------

    #[test]
    fn staged_and_unstaged_versions_of_the_same_file_are_two_hazards() {
        let report = classify_work(
            &observation(vec![
                entry("src/main.rs", WorkKind::Staged),
                entry("src/main.rs", WorkKind::Unstaged),
            ]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(
            kinds(&report),
            vec![
                HazardKind::Work(WorkKind::Staged),
                HazardKind::Work(WorkKind::Unstaged),
            ]
        );
        assert!(
            report
                .hazards
                .iter()
                .all(|hazard| hazard.path.as_deref() == Some(b"src/main.rs".as_slice()))
        );
        assert!(report.unknown.is_empty());
        assert!(!report.truncated);
    }

    #[test]
    fn a_binary_edit_is_named_and_unknown_binaryness_claims_nothing() {
        let mut binary = entry("assets/logo.png", WorkKind::Unstaged);
        binary.binary = Some(true);
        let mut text = entry("README.md", WorkKind::Unstaged);
        text.binary = Some(false);
        let undetermined = entry("data.bin", WorkKind::Untracked);
        let report = classify_work(
            &observation(vec![binary, text, undetermined]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert!(report.hazards[0].detail.contains("binary"));
        assert!(report.hazards[1].detail.contains("text"));
        assert!(!report.hazards[2].detail.contains("binary"));
        assert!(!report.hazards[2].detail.contains("text"));
    }

    #[test]
    fn renames_deletions_and_mode_and_link_changes_are_each_hazards() {
        let report = classify_work(
            &observation(vec![
                entry("old.rs", WorkKind::Renamed),
                entry("gone.rs", WorkKind::Deleted),
                entry("run.sh", WorkKind::ModeChange),
                entry("link", WorkKind::LinkChange),
            ]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(
            kinds(&report),
            vec![
                HazardKind::Work(WorkKind::Renamed),
                HazardKind::Work(WorkKind::Deleted),
                HazardKind::Work(WorkKind::ModeChange),
                HazardKind::Work(WorkKind::LinkChange),
            ]
        );
        assert!(report.unknown.is_empty());
    }

    #[test]
    fn ignored_user_data_is_a_hazard_because_ignored_does_not_mean_disposable() {
        let report = classify_work(
            &observation(vec![entry("data/user.sqlite", WorkKind::Ignored)]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(kinds(&report), vec![HazardKind::Work(WorkKind::Ignored)]);
        assert!(report.hazards[0].detail.contains("ignored"));
    }

    #[test]
    fn an_ignored_nested_git_directory_is_a_hazard_with_its_bytes_intact() {
        // Not UTF-8: paths are bytes.
        let nested = WorkEntry {
            path: b"vendor/n\xffsted/.git".to_vec(),
            kind: WorkKind::Ignored,
            binary: None,
        };
        let report = classify_work(&observation(vec![nested]), &GwzEvidence::default());
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(
            report.hazards[0].path.as_deref(),
            Some(b"vendor/n\xffsted/.git".as_slice())
        );
    }

    #[test]
    fn conflict_stages_are_hazards() {
        let report = classify_work(
            &observation(vec![entry("merge.rs", WorkKind::Conflict)]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(kinds(&report), vec![HazardKind::Work(WorkKind::Conflict)]);
        assert!(report.hazards[0].detail.contains("conflict"));
    }

    // --- open operations ----------------------------------------------------

    #[test]
    fn an_unfinished_native_operation_is_an_open_operation_hazard() {
        for (operation, word) in [
            (NativeOperation::Merge, "merge"),
            (NativeOperation::Rebase, "rebase"),
            (NativeOperation::CherryPick, "cherry-pick"),
            (NativeOperation::Revert, "revert"),
            (NativeOperation::Bisect, "bisect"),
            (NativeOperation::ApplyMailbox, "mailbox"),
            (NativeOperation::Other, "operation"),
        ] {
            let report = classify_work(
                &WorkObservation {
                    native_operation: Some(operation),
                    ..WorkObservation::default()
                },
                &GwzEvidence::default(),
            );
            assert_eq!(report.verdict, WorkVerdict::Dirty, "{operation:?}");
            assert_eq!(kinds(&report), vec![HazardKind::OpenNativeOperation]);
            assert!(
                report.hazards[0].detail.contains(word),
                "{operation:?}: {}",
                report.hazards[0].detail
            );
        }
    }

    #[test]
    fn native_stash_entries_are_a_hazard() {
        let report = classify_work(
            &WorkObservation {
                stash_entries: 3,
                ..WorkObservation::default()
            },
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(kinds(&report), vec![HazardKind::NativeStash]);
        assert!(report.hazards[0].detail.contains('3'));
        assert_eq!(
            classify_work(&WorkObservation::default(), &GwzEvidence::default()).verdict,
            WorkVerdict::Clean
        );
    }

    #[test]
    fn an_open_gwz_merge_and_stash_record_are_hazards() {
        let evidence = GwzEvidence {
            merge: EvidenceState::Open {
                detail: "merge/state.yml stage=apply".to_owned(),
            },
            stash: EvidenceState::Open {
                detail: "bundle b17".to_owned(),
            },
            other: Vec::new(),
        };
        let report = classify_work(&WorkObservation::default(), &evidence);
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(
            kinds(&report),
            vec![HazardKind::OpenGwzMerge, HazardKind::OpenGwzStash]
        );
        assert!(report.hazards[0].detail.contains("stage=apply"));
        assert!(report.hazards[1].detail.contains("b17"));
        assert!(report.unknown.is_empty());
    }

    #[test]
    fn another_open_gwz_record_is_a_hazard_that_names_its_kind() {
        let evidence = GwzEvidence {
            other: vec![EvidenceItem {
                kind: "exchange".to_owned(),
                state: EvidenceState::Open {
                    detail: "transfer t9".to_owned(),
                },
            }],
            ..GwzEvidence::default()
        };
        let report = classify_work(&WorkObservation::default(), &evidence);
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(kinds(&report), vec![HazardKind::OpenGwzRecord]);
        assert!(report.hazards[0].detail.contains("exchange"));
        assert!(report.hazards[0].detail.contains("t9"));
    }

    // --- unknown evidence ---------------------------------------------------

    #[test]
    fn uninterpretable_gwz_evidence_is_unknown_and_named_never_clean() {
        let evidence = GwzEvidence {
            merge: EvidenceState::Unknown {
                detail: "record version 9".to_owned(),
            },
            ..GwzEvidence::default()
        };
        let report = classify_work(&WorkObservation::default(), &evidence);
        assert_eq!(report.verdict, WorkVerdict::Unknown);
        assert_ne!(report.verdict, WorkVerdict::Clean);
        assert_eq!(
            unknown_kinds(&report),
            vec![UnknownKind::UnsupportedEvidence]
        );
        assert!(report.unknown[0].detail.contains("record version 9"));
        assert_eq!(kinds(&report), vec![HazardKind::UninterpretableEvidence]);
    }

    #[test]
    fn external_gitfile_and_alternates_layout_evidence_is_unknown() {
        let evidence = GwzEvidence {
            other: vec![
                EvidenceItem {
                    kind: "layout/gitfile".to_owned(),
                    state: EvidenceState::Unknown {
                        detail: ".git is a file pointing outside the tree".to_owned(),
                    },
                },
                EvidenceItem {
                    kind: "layout/alternates".to_owned(),
                    state: EvidenceState::Unknown {
                        detail: "objects/info/alternates present".to_owned(),
                    },
                },
            ],
            ..GwzEvidence::default()
        };
        let report = classify_work(&WorkObservation::default(), &evidence);
        assert_eq!(report.verdict, WorkVerdict::Unknown);
        assert_eq!(
            unknown_kinds(&report),
            vec![
                UnknownKind::UnsupportedEvidence,
                UnknownKind::UnsupportedEvidence
            ]
        );
        let text = details(&report);
        assert!(text.contains("gitfile"), "{text}");
        assert!(text.contains("alternates"), "{text}");
        assert_eq!(
            kinds(&report),
            vec![
                HazardKind::UninterpretableEvidence,
                HazardKind::UninterpretableEvidence
            ]
        );
    }

    // --- status-suppression flags ------------------------------------------

    #[test]
    fn an_edited_assume_unchanged_path_is_dirty() {
        let report = classify_work(
            &suppressed_observation(vec![suppressed(
                "config.toml",
                SuppressionFlag::AssumeUnchanged,
                PhysicalState::Differs,
            )]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(kinds(&report), vec![HazardKind::Suppressed]);
        assert_eq!(report.hazards[0].path, Some(path("config.toml")));
        assert!(report.hazards[0].detail.contains("assume-unchanged"));
        assert!(report.unknown.is_empty());
    }

    #[test]
    fn a_present_skip_worktree_path_that_differs_is_dirty() {
        let report = classify_work(
            &suppressed_observation(vec![suppressed(
                "sparse/file.rs",
                SuppressionFlag::SkipWorktree,
                PhysicalState::Differs,
            )]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(kinds(&report), vec![HazardKind::Suppressed]);
        assert!(report.hazards[0].detail.contains("skip-worktree"));
    }

    #[test]
    fn a_suppressed_path_without_a_physical_observation_is_unknown_never_clean() {
        for flag in [
            SuppressionFlag::AssumeUnchanged,
            SuppressionFlag::SkipWorktree,
            SuppressionFlag::Other,
        ] {
            let report = classify_work(
                &suppressed_observation(vec![suppressed(
                    "opaque",
                    flag,
                    PhysicalState::Unobservable,
                )]),
                &GwzEvidence::default(),
            );
            assert_eq!(report.verdict, WorkVerdict::Unknown, "{flag:?}");
            assert_ne!(report.verdict, WorkVerdict::Clean, "{flag:?}");
            assert_eq!(unknown_kinds(&report), vec![UnknownKind::Unreadable]);
            assert_eq!(report.unknown[0].path, Some(path("opaque")));
        }
    }

    #[test]
    fn a_suppressed_path_physically_matching_the_index_is_clean() {
        for flag in [
            SuppressionFlag::AssumeUnchanged,
            SuppressionFlag::SkipWorktree,
        ] {
            let report = classify_work(
                &suppressed_observation(vec![suppressed(
                    "vendored.lock",
                    flag,
                    PhysicalState::MatchesIndex,
                )]),
                &GwzEvidence::default(),
            );
            assert_eq!(report.verdict, WorkVerdict::Clean, "{flag:?}");
            assert!(report.hazards.is_empty());
            assert!(report.unknown.is_empty());
        }
    }

    #[test]
    fn an_assume_unchanged_path_absent_without_a_sparse_record_is_dirty() {
        let report = classify_work(
            &suppressed_observation(vec![suppressed(
                "deleted.txt",
                SuppressionFlag::AssumeUnchanged,
                PhysicalState::Absent,
            )]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(kinds(&report), vec![HazardKind::Suppressed]);
        assert!(report.hazards[0].detail.contains("absent"));
    }

    #[test]
    fn skip_worktree_absence_is_clean_only_with_a_recorded_valid_sparse_absence() {
        let unjustified = classify_work(
            &suppressed_observation(vec![suppressed(
                "sparse/absent.rs",
                SuppressionFlag::SkipWorktree,
                PhysicalState::Absent,
            )]),
            &GwzEvidence::default(),
        );
        assert_eq!(unjustified.verdict, WorkVerdict::Unknown);
        assert_eq!(
            unknown_kinds(&unjustified),
            vec![UnknownKind::UnsupportedIndexFlag]
        );
        assert_eq!(unjustified.unknown[0].path, Some(path("sparse/absent.rs")));

        let justified = classify_work(
            &WorkObservation {
                suppressed: vec![suppressed(
                    "sparse/absent.rs",
                    SuppressionFlag::SkipWorktree,
                    PhysicalState::Absent,
                )],
                sparse_absent: vec![path("sparse/absent.rs")],
                ..WorkObservation::default()
            },
            &GwzEvidence::default(),
        );
        assert_eq!(justified.verdict, WorkVerdict::Clean);
        assert!(justified.hazards.is_empty());
        assert!(justified.unknown.is_empty());
    }

    #[test]
    fn an_uninterpreted_index_flag_is_unknown_unless_the_bytes_differ() {
        let differs = classify_work(
            &suppressed_observation(vec![suppressed(
                "flagged",
                SuppressionFlag::Other,
                PhysicalState::Differs,
            )]),
            &GwzEvidence::default(),
        );
        assert_eq!(differs.verdict, WorkVerdict::Dirty);
        assert_eq!(kinds(&differs), vec![HazardKind::Suppressed]);
        assert!(differs.unknown.is_empty());

        for physical in [PhysicalState::MatchesIndex, PhysicalState::Absent] {
            let report = classify_work(
                &WorkObservation {
                    suppressed: vec![suppressed("flagged", SuppressionFlag::Other, physical)],
                    sparse_absent: vec![path("flagged")],
                    ..WorkObservation::default()
                },
                &GwzEvidence::default(),
            );
            assert_eq!(report.verdict, WorkVerdict::Unknown, "{physical:?}");
            assert_eq!(
                unknown_kinds(&report),
                vec![UnknownKind::UnsupportedIndexFlag]
            );
        }
    }

    #[test]
    fn valid_sparse_absence_on_its_own_is_not_dirt() {
        let report = classify_work(
            &WorkObservation {
                sparse_absent: vec![path("sparse/a.rs"), path("sparse/b.rs")],
                ..WorkObservation::default()
            },
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Clean);
        assert!(report.hazards.is_empty());
        assert!(report.unknown.is_empty());
    }

    // --- truncation ---------------------------------------------------------

    #[test]
    fn truncated_diagnostics_keep_every_hazard() {
        let count = MAX_DETAILED_ENTRIES + 5;
        let entries = (0..count)
            .map(|index| entry(&format!("f{index}"), WorkKind::Untracked))
            .collect();
        let report = classify_work(&observation(entries), &GwzEvidence::default());
        assert_eq!(report.verdict, WorkVerdict::Dirty);
        assert_eq!(report.hazards.len(), count);
        assert!(report.truncated);
        assert!(!report.hazards[MAX_DETAILED_ENTRIES - 1].detail.is_empty());
        assert!(report.hazards[MAX_DETAILED_ENTRIES].detail.is_empty());
        assert_eq!(
            report.hazards[count - 1].path,
            Some(path(&format!("f{}", count - 1)))
        );
        assert!(
            report
                .hazards
                .iter()
                .all(|hazard| hazard.kind == HazardKind::Work(WorkKind::Untracked))
        );
    }

    #[test]
    fn truncated_diagnostics_keep_every_unknown_reason() {
        let count = MAX_DETAILED_ENTRIES + 2;
        let entries = (0..count)
            .map(|index| {
                suppressed(
                    &format!("u{index}"),
                    SuppressionFlag::SkipWorktree,
                    PhysicalState::Unobservable,
                )
            })
            .collect();
        let report = classify_work(&suppressed_observation(entries), &GwzEvidence::default());
        assert_eq!(report.verdict, WorkVerdict::Unknown);
        assert_eq!(report.unknown.len(), count);
        assert!(report.truncated);
        assert!(!report.unknown[MAX_DETAILED_ENTRIES - 1].detail.is_empty());
        assert!(report.unknown[MAX_DETAILED_ENTRIES].detail.is_empty());
        assert_eq!(
            report.unknown[count - 1].path,
            Some(path(&format!("u{}", count - 1)))
        );
    }

    // --- precedence and ordering -------------------------------------------

    #[test]
    fn unknown_dominates_dirty_and_the_hazards_are_still_listed() {
        let observed = WorkObservation {
            entries: vec![entry("a.rs", WorkKind::Staged)],
            suppressed: vec![suppressed(
                "b.rs",
                SuppressionFlag::SkipWorktree,
                PhysicalState::Unobservable,
            )],
            ..WorkObservation::default()
        };
        let report = classify_work(&observed, &GwzEvidence::default());
        assert_eq!(report.verdict, WorkVerdict::Unknown);
        assert_eq!(kinds(&report), vec![HazardKind::Work(WorkKind::Staged)]);
        assert_eq!(unknown_kinds(&report), vec![UnknownKind::Unreadable]);
    }

    #[test]
    fn the_hazard_order_is_deterministic_and_follows_the_input() {
        let observed = WorkObservation {
            entries: vec![
                entry("a.rs", WorkKind::Staged),
                entry("b.rs", WorkKind::Untracked),
            ],
            suppressed: vec![suppressed(
                "c.rs",
                SuppressionFlag::AssumeUnchanged,
                PhysicalState::Differs,
            )],
            sparse_absent: Vec::new(),
            native_operation: Some(NativeOperation::Rebase),
            stash_entries: 1,
            unknown: Vec::new(),
        };
        let evidence = GwzEvidence {
            merge: EvidenceState::Open {
                detail: "open".to_owned(),
            },
            stash: EvidenceState::Open {
                detail: "open".to_owned(),
            },
            other: vec![EvidenceItem {
                kind: "exchange".to_owned(),
                state: EvidenceState::Open {
                    detail: "open".to_owned(),
                },
            }],
        };
        let expected = vec![
            HazardKind::Work(WorkKind::Staged),
            HazardKind::Work(WorkKind::Untracked),
            HazardKind::Suppressed,
            HazardKind::OpenNativeOperation,
            HazardKind::NativeStash,
            HazardKind::OpenGwzMerge,
            HazardKind::OpenGwzStash,
            HazardKind::OpenGwzRecord,
        ];
        let first = classify_work(&observed, &evidence);
        let second = classify_work(&observed, &evidence);
        assert_eq!(kinds(&first), expected);
        assert_eq!(first, second);
        assert_eq!(first.verdict, WorkVerdict::Dirty);
    }

    // --- unknown observations ----------------------------------------------

    #[test]
    fn an_unreadable_entry_makes_the_whole_observation_unknown() {
        let reason = UnknownReason {
            kind: UnknownKind::Unreadable,
            path: Some(path("locked/dir")),
            detail: "permission denied".to_owned(),
        };
        let report = classify_observed_work(
            &Observation::Unknown(vec![reason.clone()]),
            &GwzEvidence::default(),
        );
        assert_eq!(report.verdict, WorkVerdict::Unknown);
        assert_ne!(report.verdict, WorkVerdict::Clean);
        assert_eq!(report.unknown, vec![reason]);
        assert!(report.hazards.is_empty());
    }

    #[test]
    fn an_unknown_observation_still_reports_the_evidence_hazards() {
        let evidence = GwzEvidence {
            merge: EvidenceState::Open {
                detail: "open".to_owned(),
            },
            ..GwzEvidence::default()
        };
        let report = classify_observed_work(
            &Observation::Unknown(vec![UnknownReason::new(
                UnknownKind::LimitExceeded,
                "entry budget reached",
            )]),
            &evidence,
        );
        assert_eq!(report.verdict, WorkVerdict::Unknown);
        assert_eq!(kinds(&report), vec![HazardKind::OpenGwzMerge]);
        assert_eq!(unknown_kinds(&report), vec![UnknownKind::LimitExceeded]);
    }

    #[test]
    fn a_known_observation_classifies_the_same_through_either_entry_point() {
        let observed = observation(vec![entry("a.rs", WorkKind::Ignored)]);
        let evidence = GwzEvidence::default();
        assert_eq!(
            classify_observed_work(&Observation::Known(observed.clone()), &evidence),
            classify_work(&observed, &evidence)
        );
    }

    // --- the disposal force vocabulary --------------------------------------

    #[test]
    fn every_hazard_kind_maps_onto_one_dispose_force_name() {
        for kind in [
            HazardKind::Work(WorkKind::Staged),
            HazardKind::Work(WorkKind::Ignored),
            HazardKind::Suppressed,
            HazardKind::NativeStash,
        ] {
            assert_eq!(kind.force_name(), Some("dirty"), "{kind:?}");
        }
        for kind in [
            HazardKind::OpenNativeOperation,
            HazardKind::OpenGwzMerge,
            HazardKind::OpenGwzStash,
            HazardKind::OpenGwzRecord,
        ] {
            assert_eq!(kind.force_name(), Some("open-merge"), "{kind:?}");
        }
        assert_eq!(HazardKind::UninterpretableEvidence.force_name(), None);
    }

    #[test]
    fn a_hazard_with_no_force_name_never_stands_alone_as_dirty() {
        let evidence = GwzEvidence {
            merge: EvidenceState::Unknown {
                detail: "record version 9".to_owned(),
            },
            ..GwzEvidence::default()
        };
        let report = classify_work(
            &observation(vec![entry("a.rs", WorkKind::Staged)]),
            &evidence,
        );
        assert!(
            report
                .hazards
                .iter()
                .any(|hazard| hazard.kind.force_name().is_none())
        );
        assert_eq!(report.verdict, WorkVerdict::Unknown);
        assert!(!report.unknown.is_empty());
    }

    #[test]
    fn evidence_defaults_to_no_record() {
        assert_eq!(GwzEvidence::default().merge, EvidenceState::None);
        let unknown = EvidenceState::Unknown {
            detail: "record version 9".to_owned(),
        };
        assert_ne!(unknown, EvidenceState::None);
    }
}
