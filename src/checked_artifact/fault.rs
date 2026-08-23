#[cfg(test)]
use crate::model::ModelError;
use crate::model::{ErrorCode, ModelResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckedArtifactFault {
    BeforeAuthorityScratchCreate,
    AfterAuthorityScratchCreate,
    AfterAuthorityScratchWrite,
    AfterAuthorityScratchFlush,
    AfterAuthorityPublication,
    AfterAuthorityParentBarrier,
    BeforeGoalScratchCreate,
    AfterGoalScratchCreate,
    AfterGoalScratchWrite,
    AfterGoalScratchFlush,
    AfterGoalPublication,
    AfterGoalParentBarrier,
    BeforeDestinationDurability,
    AfterDestinationDurability,
    BeforeSourceRetirement,
    AfterSourceRetirement,
    BeforeManagedDestinationDurability,
    AfterManagedDestinationDurability,
    BeforeQuarantineSourceRetirement,
    AfterQuarantineSourceRetirement,
    BeforeSourceCleanup,
    AfterSourceCleanup,
    BeforeAuthorityCleanup,
    AfterAuthorityCleanup,
    #[cfg(windows)]
    AfterDestinationPathDerivation,
    // The ten boundaries of the closed durability-anchor protocol (R2-D Phase 4
    // Step 4.2; `GwzM5-8R2DInterfaceFreeze.md` §4.3 row E22). The four that
    // pre-date this step were `cfg(windows)` because the anchor was. The
    // protocol is now portable code with a Windows-only production caller, so
    // the cfg states exactly that: the boundaries exist wherever the protocol
    // is reachable — in production on Windows, and under test everywhere, which
    // is what lets every platform execute their interruption/restart rows.
    #[cfg(any(windows, test))]
    BeforeAnchorScratchCreate,
    #[cfg(any(windows, test))]
    AfterAnchorScratchWrite,
    #[cfg(any(windows, test))]
    AfterAnchorScratchFlush,
    #[cfg(any(windows, test))]
    AfterAnchorPublication,
    #[cfg(any(windows, test))]
    BeforeAnchorAliasRetirement,
    #[cfg(any(windows, test))]
    AfterAnchorAliasRetirement,
    #[cfg(any(windows, test))]
    BeforeAnchorRoundTrip,
    #[cfg(any(windows, test))]
    AfterAnchorOutboundRename,
    #[cfg(any(windows, test))]
    AfterAnchorReturnRename,
    #[cfg(any(windows, test))]
    AfterAnchorReobservation,
    // The window between a legacy leaf edge's exact proof and the sealed
    // source-associated publication that executes it (R2-D Phase 4 Step 4.1;
    // `GwzM5-8R2DInterfaceFreeze.md` §4.3 rows E18-E21), one variant per edge.
    // Step 4.1 announced all four sites from a single variant, which
    // `fail_next_checked_artifact_at` could only ever address at a drive's first
    // crossing; the Step-4.1 review's [P3-4] asked for the split here, and it is
    // free — this enum is census-free, unlike `CheckedArtifactFaultKeyV1`.
    /// Edge E21, `residue::publish_scratch`: the authority record's publication.
    BeforeAuthorityPublication,
    /// Edge E20, `residue::ensure_goal`: the staged goal's publication.
    BeforeGoalPublication,
    /// Edge E18, `transition::detach_existing`: the managed source's detachment.
    BeforeDetachPublication,
    /// Edge E19, `transition::publish_goal`: the goal's managed publication.
    BeforeManagedPublication,
    BeforeFinalCheck,
    AfterFinalProof,
    AfterDetach,
    AfterMutation,
    BeforeDurability,
    AfterDurability,
}

#[cfg(test)]
type FaultHook = (CheckedArtifactFault, Box<dyn FnOnce()>);

#[cfg(test)]
type FaultPoint = (CheckedArtifactFault, Option<&'static str>);

#[cfg(test)]
thread_local! {
    static NEXT_FAULT: std::cell::Cell<Option<FaultPoint>> = const { std::cell::Cell::new(None) };
    static NEXT_HOOK: std::cell::RefCell<Option<FaultHook>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_checked_artifact_at(boundary: CheckedArtifactFault) {
    NEXT_FAULT.set(Some((boundary, None)));
}

#[cfg(test)]
pub(crate) fn fail_next_checked_artifact_at_for(
    label: &'static str,
    boundary: CheckedArtifactFault,
) {
    NEXT_FAULT.set(Some((boundary, Some(label))));
}

#[cfg(test)]
pub(crate) fn run_next_checked_artifact_at(
    boundary: CheckedArtifactFault,
    hook: impl FnOnce() + 'static,
) {
    NEXT_HOOK.with_borrow_mut(|next| *next = Some((boundary, Box::new(hook))));
}

pub(super) fn fault(
    boundary: CheckedArtifactFault,
    code: ErrorCode,
    label: &str,
) -> ModelResult<()> {
    #[cfg(test)]
    {
        NEXT_HOOK.with_borrow_mut(|next| {
            if next.as_ref().is_some_and(|(at, _)| *at == boundary) {
                let (_, hook) = next.take().expect("matching checked-artifact hook exists");
                hook();
            }
        });
        if NEXT_FAULT.get().is_some_and(|(at, expected_label)| {
            at == boundary && expected_label.is_none_or(|expected| expected == label)
        }) {
            NEXT_FAULT.set(None);
            return Err(ModelError::new(
                code,
                format!("checked {label}: injected failure at {boundary:?}"),
            ));
        }
    }
    #[cfg(not(test))]
    let _ = (boundary, code, label);
    Ok(())
}
