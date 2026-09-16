//! The accumulator that walks one observation plus its GWZ evidence and
//! turns both into hazards, unknown reasons and diagnostics.

use std::collections::HashSet;

use gwz_repo_contract::{
    BytePath, NativeOperation, PhysicalState, SuppressionFlag, UnknownKind, UnknownReason,
    WorkKind, WorkObservation,
};

use crate::*;

/// Accumulates hazards and unknown reasons in input order.
#[derive(Default)]
pub(crate) struct Builder {
    pub(crate) hazards: Vec<Hazard>,
    pub(crate) unknown: Vec<UnknownReason>,
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

    pub(crate) fn observation(&mut self, observation: &WorkObservation) {
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

    pub(crate) fn evidence(&mut self, evidence: &GwzEvidence) {
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

    pub(crate) fn finish(mut self) -> WorkReport {
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
