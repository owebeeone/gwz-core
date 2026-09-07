//! Bounded library capability probe. Only a loopback server with an exact
//! generated host-key pin is admitted; this is not a product transport fallback.
use std::{cell::Cell, path::Path, rc::Rc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("GWZ_PROBE_URL")?;
    let parsed = url::Url::parse(&url)?;
    if parsed.scheme() != "ssh" || parsed.host_str() != Some("127.0.0.1") {
        return Err("probe requires a loopback SSH URL".into());
    }
    let expected = std::env::var("GWZ_PROBE_HOST_SHA256")?;
    if expected.len() != 64 {
        return Err("probe requires an exact host-key SHA-256 pin".into());
    }
    let mode = std::env::var("GWZ_PROBE_MODE")?;
    let key = std::env::var("GWZ_PROBE_KEY").ok();
    if !matches!(mode.as_str(), "agent" | "file") {
        return Err("unknown probe mode".into());
    }
    gwz_core::git::set_server_timeout_ms(5000);
    let calls = Rc::new(Cell::new(0));
    let callback_calls = calls.clone();
    let callback_mode = mode.clone();
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.certificate_check(move |certificate, host| {
        let actual = certificate
            .as_hostkey()
            .and_then(|key| key.hash_sha256())
            .map(|bytes| {
                bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            });
        if host == "127.0.0.1" && actual.as_deref() == Some(expected.as_str()) {
            Ok(git2::CertificateCheckStatus::CertificateOk)
        } else {
            Err(git2::Error::from_str("fixture host key mismatch"))
        }
    });
    callbacks.credentials(move |_, username, allowed| {
        let username = username.ok_or_else(|| git2::Error::from_str("missing fixture username"))?;
        if allowed.is_username() && !allowed.is_ssh_key() {
            return git2::Cred::username(username);
        }
        callback_calls.set(callback_calls.get() + 1);
        if callback_calls.get() > 1 || !allowed.is_ssh_key() {
            return Err(git2::Error::from_str(
                "probe refuses a second credential attempt",
            ));
        }
        if callback_mode == "agent" {
            git2::Cred::ssh_key_from_agent(username)
        } else {
            let path = key
                .as_deref()
                .ok_or_else(|| git2::Error::from_str("missing explicit fixture key"))?;
            git2::Cred::ssh_key(username, None, Path::new(path), None)
        }
    });
    let repo = git2::Repository::init_bare(std::env::var("GWZ_PROBE_REPOSITORY")?)?;
    let mut remote = repo.remote_anonymous(&url)?;
    let connected = remote.connect_auth(git2::Direction::Fetch, Some(callbacks), None);
    let authenticated = connected.is_ok();
    drop(connected);
    println!(
        "{{\"mode\":\"{mode}\",\"credential_callbacks\":{},\"authenticated\":{authenticated}}}",
        calls.get()
    );
    Ok(())
}
