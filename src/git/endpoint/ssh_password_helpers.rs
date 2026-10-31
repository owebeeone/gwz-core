//! The accepted ambient password-only route. One lookup per setup, using the
//! endpoint's captured configuration and the host's shared helper admissions.
use super::{
    agent_job::Control,
    https_auth::{self, AuthError},
    ssh_pool::Opening,
    ssh_setup_context::SetupContext,
};
use gwz_transport::{
    pool::{Key, LocalPhase},
    protocol::{AuthMethod, Facts},
};
use std::{io, sync::Arc, time::Duration};
use tokio::{sync::Semaphore, time::Instant};
use tokio_util::sync::CancellationToken;

/// Whose credential helper a lookup runs.
enum Source {
    /// The one helper the runtime that built the endpoint was configured with: the legacy session endpoint's, whose
    /// runtime is its one operation's. It retires with the legacy pool (plan step 3.3).
    Runtime(https_auth::Config),
    /// The helper of the context of the operation the connection is being opened for (`Opening::auth`; the connection
    /// host's endpoint instances are shared by operations and hold no helper of their own, Phase 1 Code P2-2).
    Operation,
}
pub(crate) struct Helpers {
    source: Source,
    slots: https_auth::HelperSlots,
    endpoint: Arc<Semaphore>,
}
impl Helpers {
    /// Helpers that run `config`, whichever operation asks: for an endpoint built for one operation's runtime.
    pub(crate) fn new(config: https_auth::Config, slots: https_auth::HelperSlots) -> Self {
        Self::from_source(Source::Runtime(config), slots)
    }
    /// Helpers that run the helper of the context of the operation each opening serves: for the connection host's
    /// shared endpoint instances.
    pub(crate) fn per_operation(slots: https_auth::HelperSlots) -> Self {
        Self::from_source(Source::Operation, slots)
    }
    fn from_source(source: Source, slots: https_auth::HelperSlots) -> Self {
        Self {
            source,
            slots,
            endpoint: Arc::new(Semaphore::new(8)),
        }
    }
    /// The helper `opening`'s lookup runs: the runtime's own, or the one its operation's context holds. An opening
    /// with no context, or one of another provider, has none, and the lookup is refused.
    fn helper_for<'a>(&'a self, opening: &'a Opening) -> io::Result<&'a https_auth::Config> {
        match &self.source {
            Source::Runtime(config) => Ok(config),
            Source::Operation => opening
                .auth
                .as_ref()
                .and_then(https_auth::OperationAuth::of)
                .map(https_auth::OperationAuth::helper)
                .ok_or_else(|| io::ErrorKind::InvalidInput.into()),
        }
    }
    pub(crate) fn lookup(
        &self,
        key: &Key,
        opening: &Opening,
        control: &Control,
    ) -> io::Result<https_auth::Secret> {
        let helper = self.helper_for(opening)?;
        let setup = opening.setup.as_ref().ok_or(io::ErrorKind::InvalidInput)?;
        let interaction_ms = opening.interaction_ms.min(120_000);
        if interaction_ms == 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut url = url::Url::parse(&format!(
            "ssh://{}:{}/",
            if key.host.contains(':') {
                format!("[{}]", key.host)
            } else {
                key.host.clone()
            },
            key.port
        ))
        .map_err(|_| io::ErrorKind::InvalidInput)?;
        url.set_username(key.username.as_deref().unwrap_or(""))
            .map_err(|_| io::ErrorKind::InvalidInput)?;
        url.set_path(&opening.path);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let owner = https_auth::AuthOwner::new(self.slots.clone());
            let cancelled = CancellationToken::new();
            let work = self.lookup_async(
                &owner,
                (helper, url.as_str()),
                setup,
                opening.allocation_ms,
                interaction_ms,
                &cancelled,
            );
            tokio::pin!(work);
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            let result = loop {
                tokio::select! {
                    result = &mut work => break result,
                    _ = tick.tick() => { if control.check().is_err() { cancelled.cancel(); } }
                }
            };
            // Retained physical work continues to own the setup and admissions.
            while owner.pending_cleanup_count() != 0 {
                owner
                    .reap_pending(Instant::now() + Duration::from_millis(20))
                    .await;
                if owner.pending_cleanup_count() != 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            let secret = match result {
                Ok(secret) => secret,
                Err(error) => {
                    setup.check()?;
                    let facts = Facts {
                        method: AuthMethod::Gh,
                        ..Default::default()
                    };
                    return Err(setup.terminate_failure(super::https_worker::helper_timeout(
                        error,
                        &facts,
                        opening.allocation_ms as i64,
                        interaction_ms as i64,
                    )));
                }
            };
            setup.resume().await?;
            control.check()?;
            Ok(secret)
        })
    }
    async fn lookup_async(
        &self,
        owner: &https_auth::AuthOwner,
        (helper, url): (&https_auth::Config, &str),
        setup: &SetupContext,
        allocation_ms: u64,
        interaction_ms: u64,
        cancelled: &CancellationToken,
    ) -> Result<https_auth::Secret, AuthError> {
        let until = setup
            .enter(LocalPhase::Admission, allocation_ms)
            .await
            .map_err(|_| AuthError::Cancelled)?;
        let until = Instant::from_std(until);
        let endpoint = tokio::select! {
            _ = cancelled.cancelled() => return Err(AuthError::Cancelled),
            permit = tokio::time::timeout_at(until, self.endpoint.clone().acquire_owned()) => permit.map_err(|_| AuthError::AllocationTimeout)?.map_err(|_| AuthError::Cancelled)?,
        };
        let mut remaining = until.saturating_duration_since(Instant::now());
        https_auth::lookup_setup(
            owner,
            helper,
            (url, setup),
            &mut remaining,
            Duration::from_millis(interaction_ms),
            cancelled,
            https_auth::LookupAdmission {
                until,
                endpoint_slot: Some(endpoint),
            },
        )
        .await
    }
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl Helpers {
            pub(crate) async fn hold_endpoint(&self) -> tokio::sync::OwnedSemaphorePermit { self.endpoint.clone().acquire_many_owned(8).await.unwrap() }
            pub(crate) fn available(&self) -> usize { self.endpoint.available_permits() }
        }
    }
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod operation_tests;
    }
}
