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
    credential_helpers: CredentialHelperPolicy,
    identity: Option<identity::SelectedIdentity>,
    attempt: Option<TransportAttempt>,
) -> git2::FetchOptions<'static> {
    fetch_options_with_progress(credential_helpers, identity, attempt, None)
}

pub(crate) fn fetch_options_with_progress<'a>(
    credential_helpers: CredentialHelperPolicy,
    identity: Option<identity::SelectedIdentity>,
    attempt: Option<TransportAttempt>,
    progress: Option<&'a dyn Fn(crate::GitTransferProgress)>,
) -> git2::FetchOptions<'a> {
    let mut callbacks = remote_callbacks(credential_helpers, identity, attempt);
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

pub(crate) fn remote_push_options(
    credential_helpers: CredentialHelperPolicy,
    identity: Option<identity::SelectedIdentity>,
    attempt: Option<TransportAttempt>,
    rejected: &std::cell::RefCell<Vec<(String, String)>>,
) -> git2::PushOptions<'_> {
    let mut callbacks = remote_callbacks(credential_helpers, identity, attempt);
    callbacks.push_update_reference(|refname, status| {
        if let Some(message) = status {
            rejected
                .borrow_mut()
                .push((refname.to_owned(), message.to_owned()));
        }
        Ok(())
    });
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(callbacks);
    options
}

pub(crate) fn remote_callbacks<'a>(
    credential_helpers: CredentialHelperPolicy,
    identity: Option<identity::SelectedIdentity>,
    attempt: Option<TransportAttempt>,
) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
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
            let kind = git2::CredentialType::from_bits_truncate(credential.credtype());
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
            return Err(git2::Error::from_str(
                "selected SSH identity was rejected or unavailable (encrypted file keys require exact-agent support, which is unavailable); no agent fallback was attempted",
            ));
        }
        *ssh_attempts = 1;
        return git2::Cred::ssh_key(username, None, &identity.path, None);
    }
    if allowed_types.is_username() {
        return git2::Cred::username(username);
    }
    Err(git2::Error::from_str(
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
            return Err(git2::Error::from_str(
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
    Err(git2::Error::from_str(
        "GWZ could not acquire credentials for the requested remote",
    ))
}
