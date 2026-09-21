use super::transport_observations::TransportAttempt;
use super::*;

pub(crate) mod identity;

#[derive(Default)]
struct TimeoutState {
    milliseconds: Option<i32>,
    frozen: bool,
}
static TIMEOUT_STATE: std::sync::Mutex<TimeoutState> = std::sync::Mutex::new(TimeoutState {
    milliseconds: None,
    frozen: false,
});

pub(super) fn ensure_server_timeout() {
    let mut state = TIMEOUT_STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if state.milliseconds.is_none() {
        apply_server_timeout(3000).expect("native transport timeout initialization failed");
        state.milliseconds = Some(3000);
    }
    state.frozen = true;
}

/// Legacy startup entrypoint. Must be called before constructing a backend.
/// Use `configure_server_timeout_ms` to handle an invalid or late change.
pub fn set_server_timeout_ms(ms: i32) {
    configure_server_timeout_ms(ms).expect("transport timeout must be configured at startup");
}

pub fn configure_server_timeout_ms(ms: i32) -> ModelResult<()> {
    if ms < 0 {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "transport timeout cannot be negative",
        ));
    }
    let mut state = TIMEOUT_STATE.lock().map_err(|_| {
        ModelError::new(ErrorCode::InternalError, "transport runtime lock poisoned")
    })?;
    if state.frozen && state.milliseconds != Some(ms) {
        return Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "transport timeout is process-wide and already in use; choose it before creating a backend",
        ));
    }
    if state.milliseconds != Some(ms) {
        apply_server_timeout(ms)?;
        state.milliseconds = Some(ms);
    }
    Ok(())
}

fn apply_server_timeout(ms: i32) -> ModelResult<()> {
    // SAFETY: the runtime lock serializes initialization; a different value is
    // refused once any backend exists, before native workers can observe it.
    unsafe {
        git2::opts::set_server_connect_timeout_in_milliseconds(ms).map_err(git_error)?;
        git2::opts::set_server_timeout_in_milliseconds(ms).map_err(git_error)?;
    }
    Ok(())
}

pub(crate) fn remote_fetch_options(
    backend: &Git2Backend,
    url: &str,
    identity: Option<identity::SelectedIdentity>,
    attempt: Option<TransportAttempt>,
) -> git2::FetchOptions<'static> {
    fetch_options_with_progress(backend, url, identity, attempt, None)
}

pub(crate) fn fetch_options_with_progress<'a>(
    backend: &Git2Backend,
    url: &str,
    identity: Option<identity::SelectedIdentity>,
    attempt: Option<TransportAttempt>,
    progress: Option<&'a dyn Fn(crate::GitTransferProgress)>,
) -> git2::FetchOptions<'a> {
    let mut callbacks = remote_callbacks(backend, url, identity, attempt);
    if let Some(progress) = progress {
        callbacks.transfer_progress(move |stats| {
            progress(git_transfer_progress(&stats));
            true
        });
    }
    let mut options = git2::FetchOptions::new();
    options.remote_callbacks(callbacks);
    options
}

/// What libgit2 reported through its callbacks for one push.
#[derive(Default)]
pub(crate) struct PushReport {
    /// Each destination ref the remote refused, with its message.
    pub(crate) rejected: std::cell::RefCell<Vec<(String, String)>>,
    /// Each destination ref the remote accepted, in report order.
    pub(crate) accepted: std::cell::RefCell<Vec<String>>,
    /// Each negotiated update: its destination ref and the object the push
    /// sets it to, zero for a deletion.
    pub(crate) updates: std::cell::RefCell<Vec<(String, git2::Oid)>>,
}

pub(crate) fn remote_push_options<'a>(
    backend: &Git2Backend,
    url: &str,
    identity: Option<identity::SelectedIdentity>,
    attempt: Option<TransportAttempt>,
    report: &'a PushReport,
) -> git2::PushOptions<'a> {
    let mut callbacks = remote_callbacks(backend, url, identity, attempt);
    callbacks.push_negotiation(|updates| {
        report
            .updates
            .borrow_mut()
            .extend(updates.iter().map(|update| {
                let destination = String::from_utf8_lossy(update.dst_refname_bytes()).into_owned();
                (destination, update.dst())
            }));
        Ok(())
    });
    callbacks.push_update_reference(|refname, status| {
        match status {
            Some(message) => {
                report
                    .rejected
                    .borrow_mut()
                    .push((refname.to_owned(), message.to_owned()));
            }
            None => {
                report.accepted.borrow_mut().push(refname.to_owned());
            }
        }
        Ok(())
    });
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(callbacks);
    options
}

pub(crate) fn remote_callbacks<'a>(
    backend: &Git2Backend,
    url: &str,
    identity: Option<identity::SelectedIdentity>,
    attempt: Option<TransportAttempt>,
) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    super::transport_binding::configure(
        backend,
        url,
        identity.as_ref(),
        attempt.as_ref(),
        &mut callbacks,
    );
    let credential_helpers = backend.credential_helpers;
    // libgit2 re-invokes this after each auth rejection; track SSH attempts so we offer
    // the agent once and then fail, instead of re-offering a dead credential forever.
    let mut ssh_attempts = 0u32;
    callbacks.credentials(move |url, username_from_url, allowed_types| {
        if ssh_attempts != 0
            && allowed_types.is_ssh_key()
            && let Some(attempt) = &attempt
        {
            attempt.rejected();
        }
        let credential = if let Some(identity) = &identity {
            explicit_credential(
                identity,
                username_from_url,
                allowed_types,
                &mut ssh_attempts,
            )
        } else {
            remote_credential(
                url,
                username_from_url,
                allowed_types,
                credential_helpers,
                &mut ssh_attempts,
            )
        };
        if let (Some(attempt), Ok(credential)) = (&attempt, &credential) {
            // libgit2's C enum is signed on Windows and unsigned on Unix.
            #[allow(clippy::unnecessary_cast)]
            let kind = git2::CredentialType::from_bits_truncate(credential.credtype() as u32);
            if !kind.is_username() {
                let method = if identity.is_some() {
                    crate::TransportCredentialMethod::File
                } else if kind.is_ssh_key() {
                    crate::TransportCredentialMethod::Agent
                } else if kind.is_user_pass_plaintext() {
                    crate::TransportCredentialMethod::Helper
                } else {
                    crate::TransportCredentialMethod::Unknown
                };
                attempt.offered(method);
            }
        }
        credential
    });
    callbacks
}

/// Explicit authority never enters the ambient agent/helper credential path.
pub(crate) fn explicit_credential(
    identity: &identity::SelectedIdentity,
    username_from_url: Option<&str>,
    allowed_types: git2::CredentialType,
    ssh_attempts: &mut u32,
) -> Result<git2::Cred, git2::Error> {
    let username = username_from_url.unwrap_or("git");
    if allowed_types.is_ssh_key() {
        if *ssh_attempts != 0 {
            return Err(git2::Error::new(
                git2::ErrorCode::Auth,
                git2::ErrorClass::Callback,
                "selected SSH identity was rejected or unavailable (encrypted file keys require exact-agent support, which is unavailable); no agent fallback was attempted",
            ));
        }
        *ssh_attempts = 1;
        return git2::Cred::ssh_key(username, None, &identity.path, None);
    }
    if allowed_types.is_username() {
        return git2::Cred::username(username);
    }
    Err(git2::Error::new(
        git2::ErrorCode::Auth,
        git2::ErrorClass::Callback,
        "remote did not accept selected SSH authentication; no credential fallback was attempted",
    ))
}

pub(crate) fn remote_credential(
    url: &str,
    username_from_url: Option<&str>,
    allowed_types: git2::CredentialType,
    credential_helpers: CredentialHelperPolicy,
    ssh_attempts: &mut u32,
) -> Result<git2::Cred, git2::Error> {
    let username = username_from_url.unwrap_or("git");
    if allowed_types.is_ssh_key() {
        // Offer the ssh-agent once. If libgit2 asks again, that attempt was rejected and
        // we have nothing else — return an error so it stops rather than looping forever.
        *ssh_attempts += 1;
        if *ssh_attempts > 1 {
            return Err(git2::Error::new(
                git2::ErrorCode::Auth,
                git2::ErrorClass::Callback,
                "SSH key authentication failed (no usable identity in the ssh-agent); \
                 run `ssh-add` or check your SSH setup",
            ));
        }
        return git2::Cred::ssh_key_from_agent(username);
    }
    if allowed_types.is_username() {
        return git2::Cred::username(username);
    }
    if allowed_types.is_user_pass_plaintext()
        && credential_helpers == CredentialHelperPolicy::AllowConfigured
        && let Ok(config) = git2::Config::open_default()
        && let Ok(credential) = git2::Cred::credential_helper(&config, url, username_from_url)
    {
        return Ok(credential);
    }
    if allowed_types.is_default() {
        return git2::Cred::default();
    }
    Err(git2::Error::new(
        git2::ErrorCode::Auth,
        git2::ErrorClass::Callback,
        "GWZ could not acquire credentials for the requested remote",
    ))
}

cfg_if::cfg_if! {
    if #[cfg(all(unix, gwz_transport_candidate))] {
        pub(super) fn server_timeout_ms() -> u64 {
            TIMEOUT_STATE.lock().unwrap_or_else(|e| e.into_inner()).milliseconds.unwrap_or(3000) as u64
        }
    }
}
