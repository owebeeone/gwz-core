#[cfg(test)]
use crate::model::ModelError;
use crate::model::{ErrorCode, ModelResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckedArtifactFault {
    BeforeFinalCheck,
    AfterMutation,
    BeforeDurability,
    AfterDurability,
}

#[cfg(test)]
type FaultHook = (CheckedArtifactFault, Box<dyn FnOnce()>);

#[cfg(test)]
thread_local! {
    static NEXT_FAULT: std::cell::Cell<Option<CheckedArtifactFault>> = const { std::cell::Cell::new(None) };
    static NEXT_HOOK: std::cell::RefCell<Option<FaultHook>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_checked_artifact_at(boundary: CheckedArtifactFault) {
    NEXT_FAULT.set(Some(boundary));
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
        if NEXT_FAULT.get() == Some(boundary) {
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
