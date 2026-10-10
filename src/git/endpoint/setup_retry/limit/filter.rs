//! A refusal judged against its target (§4.4) and the evidence filter (§4.5).
use super::windows::AttemptKind;

/// What a refused attempt's failure is, of §3.1's classes. Queue and Local
/// failures, and Transient and Permanent ones, have no variant: they are
/// attempts that ended with no verdict for this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signal {
    Throttle,
    Suspect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Judgement {
    Conclusive,
    Inconclusive,
    Fair,
    Unfair,
    NotALimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Evidence {
    RetryMachine,
    Inconclusive,
    HoldOnly,
    Overload,
    Confirm,
    UnfairTest,
    RefusedTest,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Refusal {
    pub(crate) kind: AttemptKind,
    pub(crate) signal: Signal,
    pub(crate) target: usize,
    pub(crate) lo: usize,
    pub(crate) hi: usize,
    pub(crate) adaptive: bool,
}

/// Judges a refusal of an attempt admitted at `target`, whose window had
/// `lo` and `hi` others (§4.4's table).
pub(crate) fn judge(kind: AttemptKind, target: usize, lo: usize, hi: usize) -> Judgement {
    match kind {
        AttemptKind::Ordinary | AttemptKind::Restore if hi < target => Judgement::Conclusive,
        AttemptKind::Ordinary | AttemptKind::Restore => Judgement::Inconclusive,
        // Nothing else was counted, so a refusal at k = 1 is no limit.
        AttemptKind::Confirming if target <= 1 => Judgement::NotALimit,
        AttemptKind::Probe | AttemptKind::Confirming if fair(target, lo) => Judgement::Fair,
        AttemptKind::Probe | AttemptKind::Confirming => Judgement::Unfair,
    }
}

/// Whether a test at `target` ran fair: every connection below it stayed
/// Connected through the window (`lo = target - 1`). A test that succeeds
/// unfair raises nothing (R2).
pub(crate) fn fair(target: usize, lo: usize) -> bool {
    lo + 1 == target
}

/// What a refusal is to the machine (§4.5, §4.8). `hi = 0` is the retry
/// machine's, before anything else.
pub(crate) fn evidence(refusal: &Refusal) -> Evidence {
    let Refusal {
        kind,
        signal,
        target,
        lo,
        hi,
        adaptive,
    } = *refusal;
    if !kind.is_test() && hi == 0 {
        return Evidence::RetryMachine;
    }
    // Nothing can be tested without a budget (`--max-retries 0`): a Suspect
    // is not evidence, and the retry machine counts it as it always did.
    if !adaptive && signal == Signal::Suspect {
        return Evidence::RetryMachine;
    }
    match (judge(kind, target, lo, hi), kind, signal) {
        (Judgement::NotALimit, _, _) => Evidence::RetryMachine,
        (Judgement::Inconclusive, _, _) => Evidence::Inconclusive,
        (Judgement::Unfair, _, _) => Evidence::UnfairTest,
        (Judgement::Fair, AttemptKind::Probe, _) => Evidence::RefusedTest,
        (Judgement::Fair, _, _) => Evidence::Overload,
        (Judgement::Conclusive, _, _) if !adaptive => Evidence::HoldOnly,
        (Judgement::Conclusive, _, Signal::Throttle) => Evidence::Overload,
        (Judgement::Conclusive, _, Signal::Suspect) => Evidence::Confirm,
    }
}
