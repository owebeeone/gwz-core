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
    if !matches!(
        mode.as_str(),
        "agent" | "file" | "remote-file" | "configured-file"
    ) {
        return Err("unknown probe mode".into());
    }
    gwz_core::git::set_server_timeout_ms(5000);
    if std::env::var_os("GWZ_PROBE_PRODUCT").is_some() {
        return product_probe(&url, &mode, key.as_deref());
    }
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

fn product_probe(
    url: &str,
    mode: &str,
    key: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use gwz_core::git::{Git2Backend, GitBackend};
    let path = std::path::PathBuf::from(std::env::var("GWZ_PROBE_REPOSITORY")?);
    let repo = git2::Repository::init_bare(&path)?;
    repo.remote("origin", url)?;
    let fixture_home = std::ffi::CString::new(std::env::var("GWZ_PROBE_HOME")?)?;
    // SAFETY: this standalone fixture process has no worker threads. Only
    // libgit2's test-process home is changed, never HOME or the user's files.
    let result = unsafe {
        libgit2_sys::git_libgit2_opts(libgit2_sys::GIT_OPT_SET_HOMEDIR as _, fixture_home.as_ptr())
    };
    if result != 0 {
        return Err("could not configure isolated known-hosts fixture".into());
    }
    let base = Git2Backend::without_credential_helpers();
    if mode == "configured-file" {
        base.set_remote_identity(&path, "origin", Some(key.ok_or("missing key")?))?;
    }
    let options = gwz_core::TransportOptions {
        url_scheme: None,
        default_identity: if mode == "file" {
            Some(key.ok_or("missing key")?.to_owned())
        } else {
            None
        },
        remote_identities: if mode == "remote-file" {
            vec![gwz_core::RemoteSshIdentity {
                remote: "origin".into(),
                private_key_path: key.ok_or("missing key")?.into(),
            }]
        } else {
            vec![]
        },
    };
    let scoped = base.with_transport(&path, Some(&options));
    let (authenticated, observations) = match scoped {
        Ok(scoped) => {
            let backend = scoped.as_ref().unwrap_or(&base);
            let authenticated = backend.ls_remote(&path, "origin").is_ok();
            let rows = backend.transport_observations().unwrap().snapshot();
            let observations: Vec<_> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "credential_method": format!("{:?}", row.credential_method),
                        "selection_source": format!("{:?}", row.selection_source),
                        "credential_offered": row.credential_offered,
                        "authenticated": row.authenticated,
                        "public_key_fingerprint": row.public_key_fingerprint,
                    })
                })
                .collect();
            (authenticated, observations)
        }
        Err(_) => (false, Vec::new()),
    };
    println!(
        "{}",
        serde_json::json!({ "mode": mode, "authenticated": authenticated, "observations": observations })
    );
    Ok(())
}
