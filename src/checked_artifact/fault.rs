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
    BeforeAnchorRoundTrip,
    #[cfg(windows)]
    AfterAnchorOutboundRename,
    #[cfg(windows)]
    AfterAnchorReturnRename,
    #[cfg(windows)]
    AfterAnchorReobservation,
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
