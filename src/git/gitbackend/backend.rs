#[cfg(test)]
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialHelperPolicy {
    Disabled,
    AllowConfigured,
}

#[derive(Clone)]
pub struct Git2Repository {
    pub(crate) filesystem: std::sync::Arc<dyn crate::filesystem::FileSystem>,
    pub(crate) credential_helpers: CredentialHelperPolicy,
    pub(crate) identities: super::transport_support::identity::Selection,
    pub(crate) observations: super::transport_observations::TransportObservations,
}

/// Compatibility name for existing callers.
pub type Git2Backend = Git2Repository;

impl std::fmt::Debug for Git2Repository {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Git2Repository")
            .field("credential_helpers", &self.credential_helpers)
            .field("identities", &self.identities)
            .field("observations", &self.observations)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Git2Backend {
    fn eq(&self, other: &Self) -> bool {
        self.credential_helpers == other.credential_helpers && self.identities == other.identities
    }
}
impl Eq for Git2Backend {}

impl Git2Backend {
    pub fn new() -> Self {
        super::transport_support::ensure_server_timeout();
        Self {
            filesystem: crate::filesystem::native_filesystem(),
            credential_helpers: CredentialHelperPolicy::AllowConfigured,
            identities: Default::default(),
            observations: Default::default(),
        }
    }

    pub fn without_credential_helpers() -> Self {
        super::transport_support::ensure_server_timeout();
        Self {
            filesystem: crate::filesystem::native_filesystem(),
            credential_helpers: CredentialHelperPolicy::Disabled,
            identities: Default::default(),
            observations: Default::default(),
        }
    }

    /// Compose the filesystem and repository services for one operation from
    /// this exact production backend.
    ///
    /// Command drivers pass the returned opaque value to core entry points;
    /// they do not select a fake or native implementation through global
    /// process state.
    pub fn operation_services(&self) -> crate::operation_context::OperationServices {
        crate::operation_context::OperationServices::from_services(
            self.filesystem.clone(),
            std::sync::Arc::new(self.clone()),
        )
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

    #[cfg(test)]
    pub(crate) fn before_next_preservation_stash(callback: impl FnOnce() + 'static) {
        BEFORE_PRESERVATION_STASH.with(|slot| {
            assert!(
                slot.borrow_mut().replace(Box::new(callback)).is_none(),
                "a preservation-stash callback is already installed"
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
    static BEFORE_PRESERVATION_STASH: RefCell<Option<Box<dyn FnOnce()>>> =
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

#[cfg(test)]
pub(super) fn run_before_preservation_stash() {
    if let Some(callback) = BEFORE_PRESERVATION_STASH.with(|slot| slot.borrow_mut().take()) {
        callback();
    }
}

#[cfg(not(test))]
pub(super) fn run_before_preservation_stash() {}

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
