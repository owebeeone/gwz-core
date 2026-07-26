#[cfg(test)]
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialHelperPolicy {
    Disabled,
    AllowConfigured,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Git2Backend {
    pub(crate) credential_helpers: CredentialHelperPolicy,
}

impl Git2Backend {
    pub fn new() -> Self {
        Self {
            credential_helpers: CredentialHelperPolicy::AllowConfigured,
        }
    }

    pub fn without_credential_helpers() -> Self {
        Self {
            credential_helpers: CredentialHelperPolicy::Disabled,
        }
    }

    #[cfg(test)]
    pub(crate) fn reset_preparation_call_count() {
        PREPARATION_CALL_COUNT.set(0);
    }

    #[cfg(test)]
    pub(crate) fn preparation_call_count() -> usize {
        PREPARATION_CALL_COUNT.get()
    }

    #[cfg(test)]
    pub(crate) fn before_next_prepared_execution(callback: impl FnOnce() + 'static) {
        BEFORE_PREPARED_EXECUTION.with(|slot| {
            assert!(
                slot.borrow_mut().replace(Box::new(callback)).is_none(),
                "a prepared-execution callback is already installed"
            );
        });
    }

    #[cfg(test)]
    pub(crate) fn before_next_scoped_commit_ref_lock(callback: impl FnOnce() + 'static) {
        BEFORE_SCOPED_COMMIT_REF_LOCK.with(|slot| {
            assert!(
                slot.borrow_mut().replace(Box::new(callback)).is_none(),
                "a scoped-commit callback is already installed"
            );
        });
    }
}

#[cfg(test)]
thread_local! {
    static PREPARATION_CALL_COUNT: Cell<usize> = const { Cell::new(0) };
    static BEFORE_PREPARED_EXECUTION: RefCell<Option<Box<dyn FnOnce()>>> =
        const { RefCell::new(None) };
    static BEFORE_SCOPED_COMMIT_REF_LOCK: RefCell<Option<Box<dyn FnOnce()>>> =
        const { RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn record_preparation_call() {
    PREPARATION_CALL_COUNT.set(PREPARATION_CALL_COUNT.get() + 1);
}

#[cfg(not(test))]
pub(super) fn record_preparation_call() {}

#[cfg(test)]
pub(super) fn run_before_prepared_execution() {
    if let Some(callback) = BEFORE_PREPARED_EXECUTION.with(|slot| slot.borrow_mut().take()) {
        callback();
    }
}

#[cfg(not(test))]
pub(super) fn run_before_prepared_execution() {}

#[cfg(test)]
pub(super) fn run_before_scoped_commit_ref_lock() {
    if let Some(callback) = BEFORE_SCOPED_COMMIT_REF_LOCK.with(|slot| slot.borrow_mut().take()) {
        callback();
    }
}

#[cfg(not(test))]
pub(super) fn run_before_scoped_commit_ref_lock() {}

impl Default for Git2Backend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod merge_interface_tests {
    use super::*;

    #[test]
    fn merge_simulation_is_wired_to_the_production_backend() {
        let backend = Git2Backend::new();
        let error = backend
            .merge_simulate(Path::new("missing"), "before", "source")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::GitCommandFailed);
    }

    #[test]
    fn status_contract_distinguishes_recovery_relevant_dirt() {
        let status = GitStatus {
            staged: 1,
            unstaged: 2,
            untracked: 3,
            ignored: 4,
            unresolved: 5,
            ..GitStatus::default()
        };
        assert_eq!(
            (status.staged, status.unstaged, status.untracked),
            (1, 2, 3)
        );
        assert_eq!((status.ignored, status.unresolved), (4, 5));
    }
}
