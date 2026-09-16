//! Step 3 of design §5.2: gather fresh work and history evidence through
//! the ports and classify it into hazards.

use std::collections::BTreeMap;

use gwz_repo_contract::{Observation, UnknownKind, UnknownReason};
use gwz_work_detector::{WorkVerdict, classify_observed_work};

use crate::*;

/// Design §5.1 and §5.2 step 3: every repository in the deletion tree is
/// inspected, `Unknown` dominates and is never waivable, and a known hazard
/// refuses unless its own name was given.
pub(crate) fn inspect(
    plan: &Plan,
    evidence: &TargetEvidence,
    waivers: &[HazardWaiver],
    ports: &mut dyn DisposalPorts,
) -> Result<(), DisposeError> {
    if evidence.repositories.is_empty() {
        return Err(DisposeError::Unknown(vec![UnknownReason::new(
            UnknownKind::UnsupportedLayout,
            format!(
                "no repository was observed in {}; the target is not a recognised clone",
                plan.target.display()
            ),
        )]));
    }
    let mut unknown = Vec::new();
    let mut findings = Vec::new();
    for repository in &evidence.repositories {
        // Nothing outside the validated target is ever considered, and an
        // observation that reached out of it is the visible evidence of a
        // symlink entry into an external tree (design §5.2 step 4).
        for (label, path) in [
            ("worktree", &repository.info.path),
            ("git directory", &repository.info.git_dir),
            ("object store", &repository.info.common_dir),
        ] {
            if !resolve(path).starts_with(&plan.target) {
                return Err(DisposeError::PathMismatch {
                    expected: plan.target.clone(),
                    observed: format!(
                        "the {label} of `{}` is {}, outside the deletion tree",
                        repository.key,
                        path.display()
                    ),
                });
            }
        }

        let report = classify_observed_work(&repository.work, &repository.gwz);
        if report.verdict == WorkVerdict::Unknown && report.unknown.is_empty() {
            unknown.push(UnknownReason::new(
                UnknownKind::Unimplemented,
                format!("`{}`: an unknown verdict with no reason", repository.key),
            ));
        }
        unknown.extend(report.unknown);
        let mut grouped: BTreeMap<HazardWaiver, Vec<Hazard>> = BTreeMap::new();
        for hazard in report.hazards {
            // The classifier's force name and this crate's waiver vocabulary
            // are one map. A hazard this vocabulary cannot spell is unknown,
            // never silently dropped and never waivable.
            match hazard.kind.force_name().and_then(HazardWaiver::parse) {
                Some(waiver) => grouped.entry(waiver).or_default().push(hazard),
                None => unknown.push(UnknownReason::new(
                    UnknownKind::UnsupportedEvidence,
                    format!(
                        "`{}`: {:?} has no waiver ({})",
                        repository.key, hazard.kind, hazard.detail
                    ),
                )),
            }
        }
        findings.extend(grouped.into_iter().map(|(waiver, hazards)| HazardFinding {
            waiver,
            repository: repository.key.clone(),
            hazards,
            detail: None,
        }));

        match &repository.history {
            Observation::Unknown(reasons) => unknown.extend(reasons.iter().cloned()),
            Observation::Known(protected) => {
                let query = HistoryQuery {
                    target: repository.key.clone(),
                    protected: protected.clone(),
                };
                match ports.check_history(&query) {
                    HistoryAnswer::Preserved => {}
                    HistoryAnswer::Unpreserved { detail } => findings.push(HazardFinding {
                        waiver: HazardWaiver::UnpreservedHistory,
                        repository: repository.key.clone(),
                        hazards: Vec::new(),
                        detail: Some(detail),
                    }),
                    HistoryAnswer::Unknown { reasons } => unknown.extend(reasons),
                }
            }
        }
    }
    // Unknown dominates: a force name waives a *known* loss, never an
    // observation that was never established (design §5.1, §12).
    if !unknown.is_empty() {
        return Err(DisposeError::Unknown(unknown));
    }
    let unwaived: Vec<HazardFinding> = findings
        .into_iter()
        .filter(|finding| !waivers.contains(&finding.waiver))
        .collect();
    if !unwaived.is_empty() {
        return Err(DisposeError::Hazards(unwaived));
    }
    Ok(())
}
