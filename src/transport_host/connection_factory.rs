//! gwz-core's production `EndpointFactory` (plan step 1.3b; contract section 4.1 of dev-docs/GwzConnectionApiContract.md):
//! what `TransportRuntime::build_native` gives its connection host to make endpoint adapters with.
//!
//! The factory is made from the runtime's endpoint settings once, and each `create` makes one adapter from them,
//! allocating metadata only: no thread, network, key, agent or helper (contract section 5). For SSH the adapter is
//! `git::endpoint::ssh_adapter::SshAdapter` over today's connector (`ssh_local::native_connector`), on the runtime's
//! known-hosts file, agent, SSH helpers and a shared reservation of the host's own. It holds no pool: the host does.
//!
//! **The seam for step 1.3c (HTTPS).** Today an adapter serves `Scheme::Ssh` only: an HTTPS connect completes at once
//! with `UnsupportedOperation` (in the SSH adapter's worker, `ssh_adapter/worker.rs`, `Worker::connect`). Step 1.3c adds
//! an `https` input here beside `ssh` (built in `build_native` from the `HttpsEndpointConfig` and `HelperSlots` that
//! today go only to `Session::endpoint_with_https_native`), and `create` returns an adapter that routes each connect
//! by its key's scheme to the SSH adapter or the HTTPS one; nothing else in the host changes.
use crate::git::endpoint::{
    ssh_adapter::{AdapterSettings, SshAdapter},
    ssh_key_snapshot::Registry,
    ssh_local::{self, NativeInputs},
};
use gwz_connection::{
    CleanupStatus, EndpointAdapter, EndpointFactory, EndpointSpec, FactoryEnv, FactoryError,
    FactoryReason,
};

/// The SSH half of the factory: the connector's inputs, the key registry its selected keys are admitted to, and the
/// budgets of its connections.
#[derive(Clone)]
pub(crate) struct SshInputs {
    pub(crate) native: NativeInputs,
    pub(crate) registry: Registry,
    pub(crate) settings: AdapterSettings,
}
pub(crate) struct CoreFactory {
    ssh: Option<SshInputs>,
}
impl CoreFactory {
    pub(crate) fn new(ssh: Option<SshInputs>) -> Self {
        Self { ssh }
    }
    /// The registry the factory's adapters look selected keys up in.
    pub(crate) fn registry(&self) -> Option<Registry> {
        self.ssh.as_ref().map(|ssh| ssh.registry.clone())
    }
}
impl EndpointFactory for CoreFactory {
    fn create(
        &self,
        _spec: &EndpointSpec,
        _env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        let Some(ssh) = self.ssh.clone() else {
            // A runtime with no SSH settings has no endpoint to make (until step 1.3c adds HTTPS here).
            return Err(FactoryError::new(
                FactoryReason::Unavailable,
                CleanupStatus::Complete,
            ));
        };
        let SshInputs {
            native,
            registry,
            settings,
        } = ssh;
        Ok(Box::new(SshAdapter::new(
            move |origin| ssh_local::native_connector(origin, registry, native),
            settings,
        )))
    }
}
