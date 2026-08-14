use std::cell::RefCell;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimeBootstrapFault {
    FinalLeaseOpen,
    FinalLeaseLock,
    CatalogPreparation,
    CatalogFinalLeaseOpen,
    CatalogFinalLeaseLock,
}

type Callback = Box<dyn FnOnce()>;

thread_local! {
    static NEXT: RefCell<Option<(RuntimeBootstrapFault, Callback)>> = RefCell::new(None);
}

pub(super) fn run_next_at(fault: RuntimeBootstrapFault, callback: impl FnOnce() + 'static) {
    NEXT.with(|next| {
        let previous = next.replace(Some((fault, Box::new(callback))));
        assert!(
            previous.is_none(),
            "runtime bootstrap fault already scheduled"
        );
    });
}

pub(super) fn run(observed: RuntimeBootstrapFault) {
    NEXT.with(|next| {
        let should_run = next
            .borrow()
            .as_ref()
            .is_some_and(|(scheduled, _)| *scheduled == observed);
        if should_run {
            let (_, callback) = next.borrow_mut().take().expect("scheduled callback");
            callback();
        }
    });
}
