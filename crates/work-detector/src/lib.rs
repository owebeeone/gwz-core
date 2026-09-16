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

use gwz_repo_contract::{Observation, WorkObservation};

mod builder;
mod evidence;
mod hazard;
mod report;

#[cfg(test)]
mod tests;

pub use evidence::*;
pub use hazard::*;
pub use report::*;

pub(crate) use builder::*;

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
