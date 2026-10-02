//! Bounded credential subprocess interaction.
use super::*;

pub(crate) struct LookupAdmission {
    pub(crate) until: Instant,
    pub(crate) endpoint_slot: Option<OwnedSemaphorePermit>,
}

pub(crate) async fn lookup_until(
    owner: &AuthOwner,
    config: &Config,
    destination: &Destination,
    allocation: &mut Duration,
    interaction: Duration,
    cancelled: &CancellationToken,
    admission: LookupAdmission,
) -> Result<Secret, AuthError> {
    lookup_url_until(
        owner,
        config,
        (destination.url.as_str(), None),
        allocation,
        interaction,
        cancelled,
        admission,
    )
    .await
}

pub(crate) async fn lookup_setup(
    owner: &AuthOwner,
    config: &Config,
    (url, setup): (&str, &super::super::ssh_setup_context::SetupContext),
    allocation: &mut Duration,
    interaction: Duration,
    cancelled: &CancellationToken,
    admission: LookupAdmission,
) -> Result<Secret, AuthError> {
    lookup_url_until(
        owner,
        config,
        (url, Some(setup)),
        allocation,
        interaction,
        cancelled,
        admission,
    )
    .await
}

async fn lookup_url_until(
    owner: &AuthOwner,
    config: &Config,
    (url, setup): (&str, Option<&super::super::ssh_setup_context::SetupContext>),
    allocation: &mut Duration,
    interaction: Duration,
    cancelled: &CancellationToken,
    admission: LookupAdmission,
) -> Result<Secret, AuthError> {
    if config.executable.as_os_str().is_empty() {
        return Err(AuthError::MissingExecutable);
    }
    if cancelled.is_cancelled() || owner.inner.cancelled.is_cancelled() {
        return Err(AuthError::Cancelled);
    }
    let allowance = Duration::from_millis(allocation.as_millis() as u64);
    if allowance.is_zero() || Instant::now() >= admission.until {
        return Err(AuthError::AllocationTimeout);
    }
    owner.reap_ready();
    let admission_started = Instant::now();
    let admitted = tokio::select! {
        _ = cancelled.cancelled() => Err(AuthError::Cancelled),
        _ = owner.inner.cancelled.cancelled() => Err(AuthError::Cancelled),
        result = tokio::time::timeout_at(admission.until,
            owner.inner.helper_slots.0.clone().acquire_owned()) => {
            match result {
                Ok(permit) => permit.map_err(|_| AuthError::Cancelled),
                Err(_) => Err(AuthError::AllocationTimeout),
            }
        }
    };
    *allocation = allocation.saturating_sub(admission_started.elapsed());
    let helper_slot = admitted?;
    if Instant::now() >= admission.until {
        return Err(AuthError::AllocationTimeout);
    }
    if interaction.is_zero() {
        return Err(AuthError::Timeout);
    }
    let interaction = interaction.min(Duration::from_secs(120));
    let deadline = if let Some(setup) = setup {
        Instant::from_std(
            setup
                .enter(
                    gwz_transport::pool::LocalPhase::Interaction,
                    interaction.as_millis() as u64,
                )
                .await
                .map_err(|_| AuthError::Cancelled)?,
        )
    } else {
        Instant::now() + interaction
    };

    let executable = super::executable::resolve(config)?;
    let permits = Arc::new(super::owner::AdmissionPermits {
        _helper_slot: helper_slot,
        _endpoint_slot: admission.endpoint_slot,
    });
    owner.inner.active.fetch_add(1, Ordering::AcqRel);
    let _active = ActiveGuard {
        active: owner.inner.active.clone(),
    };
    let runner = super::runner::Runner {
        owner,
        config,
        executable: &executable,
        permits,
        cancelled,
        deadline,
        setup,
    };
    let parameters = super::view::prepare(&runner).await?;
    let request = SecretBuffer(format!("url={url}\n\n").into_bytes());
    let output = runner
        .run(
            &["-c", "core.askPass=", "credential", "fill"],
            &request.0,
            Some(&parameters.0),
            OUTPUT_LIMIT,
            false,
        )
        .await?;
    parse_secret(&output.0)
}

/// Writes the lookup request to the helper's input and closes it.
///
/// A helper may exit without reading its input. Git ignores SIGPIPE while it
/// writes to a credential helper, so a broken pipe here is not a failure: the
/// helper's exit status and output decide the lookup. Any other write error is.
pub(super) async fn write_request<W>(mut input: W, request: &[u8]) -> Result<(), AuthError>
where
    W: AsyncWrite + Unpin,
{
    let written = match input.write_all(request).await {
        Ok(()) => input.shutdown().await,
        Err(error) => Err(error),
    };
    match written {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            Err(AuthError::Pipe(error.kind()))
        }
        _ => Ok(()),
    }
}

pub(super) async fn discard_stderr<R: AsyncRead + Unpin>(mut reader: R) -> Result<(), AuthError> {
    let mut scratch = SecretBuffer(vec![0; 4096]);
    loop {
        if reader
            .read(&mut scratch.0)
            .await
            .map_err(|error| AuthError::Pipe(error.kind()))?
            == 0
        {
            return Ok(());
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        pub(super) fn configure_process_group(command: &mut Command) {
            command.process_group(0);
        }
    } else {
        pub(super) fn configure_process_group(_command: &mut Command) {}
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub(crate) async fn lookup_with_budget(
            owner: &AuthOwner, config: &Config, destination: &Destination,
            allocation: &mut Duration, interaction: Duration, cancelled: &CancellationToken,
        ) -> Result<Secret, AuthError> {
            let until = Instant::now() + Duration::from_millis(allocation.as_millis() as u64);
            lookup_until(owner, config, destination, allocation, interaction, cancelled,
                LookupAdmission { until, endpoint_slot: None }).await
        }
        pub(crate) async fn lookup_owned(
            owner: &AuthOwner, config: &Config, destination: &Destination,
            deadline: Instant, cancelled: &CancellationToken,
        ) -> Result<Secret, AuthError> {
            let allowance = deadline.saturating_duration_since(Instant::now());
            let mut allocation = allowance;
            lookup_with_budget(owner, config, destination, &mut allocation, allowance, cancelled).await
        }
    }
}
