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
    pub(crate) ssh: super::transport_binding::Runtime,
    /// The `GIT_*` variables the native commit and tag route reads, captured
    /// when the backend is built.
    pub(crate) environment: super::native_commit::GitEnvironment,
    pub(crate) route_policy: super::native_commit::RoutePolicy,
    /// Where a commit or tag says why it ran the Git CLI, if the caller gave a reader.
    pub(crate) route_notes: Option<super::native_commit::RouteNotes>,
    /// The spawn gate of the host this backend's operation runs under (WH2 contract C9): held by every spawn of the
    /// backend's that names no handle list, so none lands in a launch's inheritance window. A backend with no host
    /// has a gate of its own, which nothing else holds.
    pub(crate) spawn_gate: crate::git::spawn_gate::SpawnGate,
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
            ssh: Default::default(),
            environment: super::native_commit::GitEnvironment::process(),
            route_policy: Default::default(),
            route_notes: None,
            spawn_gate: crate::git::spawn_gate::SpawnGate::ungated(),
        }
    }

    /// A copy of this backend that tells `notes` why a commit or tag ran the Git CLI
    /// (the CLI gives a reader for `--verbose` in human output).
    pub fn with_route_notes(&self, notes: super::native_commit::RouteNotes) -> Self {
        let mut backend = self.clone();
        backend.route_notes = Some(notes);
        backend
    }

    pub fn without_credential_helpers() -> Self {
        super::transport_support::ensure_server_timeout();
        Self {
            filesystem: crate::filesystem::native_filesystem(),
            credential_helpers: CredentialHelperPolicy::Disabled,
            identities: Default::default(),
            observations: Default::default(),
            ssh: Default::default(),
            environment: super::native_commit::GitEnvironment::process(),
            route_policy: Default::default(),
            route_notes: None,
            spawn_gate: crate::git::spawn_gate::SpawnGate::ungated(),
        }
    }

    cfg_if::cfg_if! {
        if #[cfg(all(gwz_transport_candidate, any(unix, windows)))] {
            pub(crate) fn with_host_context(&self, context: crate::transport_host::RequestContext) -> Self {
                let mut backend = self.clone();
                backend.spawn_gate = context.spawn_gate();
                backend.ssh = backend.ssh.with_host_context(context);
                backend
            }
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
