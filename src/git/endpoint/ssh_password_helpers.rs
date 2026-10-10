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

pub(crate) struct Helpers {
    config: https_auth::Config,
    slots: https_auth::HelperSlots,
    endpoint: Arc<Semaphore>,
}
impl Helpers {
    pub(crate) fn new(config: https_auth::Config, slots: https_auth::HelperSlots) -> Self {
        Self {
            config,
            slots,
            endpoint: Arc::new(Semaphore::new(8)),
        }
    }
    cfg_if::cfg_if! { if #[cfg(unix)] {
    pub(crate) fn lookup(
        &self,
        key: &Key,
        opening: &Opening,
        control: &Control,
    ) -> io::Result<https_auth::Secret> {
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
                url.as_str(),
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
        url: &str,
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
            &self.config,
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
    } else {
        /// Configured password helpers need WH2's helper runner on Windows (Job Object owner, paths, framing;
        /// plan steps 4.1 to 4.5), so a setup that reaches one is refused, naming it.
        pub(crate) fn lookup(&self, _: &Key, _: &Opening, _: &Control) -> io::Result<https_auth::Secret> {
            Err(io::Error::new(io::ErrorKind::Unsupported, WINDOWS_HELPERS_UNSUPPORTED))
        }
    } }
}
cfg_if::cfg_if! {
    if #[cfg(not(unix))] {
        const WINDOWS_HELPERS_UNSUPPORTED: &str =
            "configured password helpers are not supported on Windows until WH2";
    }
}
cfg_if::cfg_if! {
    if #[cfg(all(test, windows))] {
        mod windows_tests;
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
