use super::*;

/// Set libgit2's server (SSH/network) read timeout, process-wide, in milliseconds.
/// libssh2/libgit2 default to NO timeout, so a stalled SSH handshake — an empty ssh-agent
/// or an unreachable host — hangs forever; a positive value makes it a fast `Timeout`
/// error (libgit2 feeds it to `libssh2_session_set_timeout`). `0` disables it. Call ONCE
/// at startup before any network op spawns threads (mutates a libgit2 global without
/// synchronization).
pub fn set_server_timeout_ms(ms: i32) {
    // SAFETY: invoked once from CLI startup, before any backend operation / thread spawn.
    unsafe {
        let _ = git2::opts::set_server_timeout_in_milliseconds(ms);
    }
}

pub(crate) fn remote_fetch_options(
    credential_helpers: CredentialHelperPolicy,
) -> git2::FetchOptions<'static> {
    fetch_options_with_progress(credential_helpers, None)
}

pub(crate) fn fetch_options_with_progress<'a>(
    credential_helpers: CredentialHelperPolicy,
    progress: Option<&'a dyn Fn(crate::GitTransferProgress)>,
) -> git2::FetchOptions<'a> {
    let mut callbacks = remote_callbacks(credential_helpers);
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
) -> git2::PushOptions<'static> {
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(remote_callbacks(credential_helpers));
    options
}

pub(crate) fn remote_callbacks<'a>(
    credential_helpers: CredentialHelperPolicy,
) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    // libgit2 re-invokes this after each auth rejection; track SSH attempts so we offer
    // the agent once and then fail, instead of re-offering a dead credential forever.
    let mut ssh_attempts = 0u32;
    callbacks.credentials(move |url, username_from_url, allowed_types| {
        remote_credential(
            url,
            username_from_url,
            allowed_types,
            credential_helpers,
            &mut ssh_attempts,
        )
    });
    callbacks
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
