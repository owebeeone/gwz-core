//! Shared builders for observations, entries and suppressed records.

use super::*;

pub(crate) fn path(name: &str) -> BytePath {
    name.as_bytes().to_vec()
}

pub(crate) fn entry(name: &str, kind: WorkKind) -> WorkEntry {
    WorkEntry {
        path: path(name),
        kind,
        binary: None,
    }
}

pub(crate) fn observation(entries: Vec<WorkEntry>) -> WorkObservation {
    WorkObservation {
        entries,
        ..WorkObservation::default()
    }
}

pub(crate) fn suppressed(
    name: &str,
    flag: SuppressionFlag,
    physical: PhysicalState,
) -> SuppressedEntry {
    SuppressedEntry {
        path: path(name),
        flag,
        physical,
    }
}

pub(crate) fn suppressed_observation(entries: Vec<SuppressedEntry>) -> WorkObservation {
    WorkObservation {
        suppressed: entries,
        ..WorkObservation::default()
    }
}

pub(crate) fn kinds(report: &WorkReport) -> Vec<HazardKind> {
    report.hazards.iter().map(|h| h.kind.clone()).collect()
}

pub(crate) fn unknown_kinds(report: &WorkReport) -> Vec<UnknownKind> {
    report.unknown.iter().map(|reason| reason.kind).collect()
}

pub(crate) fn details(report: &WorkReport) -> String {
    format!("{:?} {:?}", report.hazards, report.unknown)
}

// --- the all-clean case -------------------------------------------------
