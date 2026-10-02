//! The endpoint environment: what a transport runtime's endpoint
//! configuration is derived from. It is an environment snapshot, and nothing
//! here reads the process environment (1.1.0 S6.1, as amendment 2's §3.17
//! extends it; gwz-py `dev-docs/GwzPyPerOperationTransportDesign.md` §2.2).
//!
//! Both entries derive their runtime's configuration here: the cancellable
//! entry from the snapshot its caller passes, and `with_local_transport` from
//! the process environment it captures once, at its command's start.
//!
//! **The Windows seam.** The SSH home, the agent and the HTTPS proxy differ by
//! platform, so each comes from the `platform` module below. This file is
//! compiled on Unix only, with the rest of `transport_host`, and its one arm is
//! Unix's. TR1.8 (amendment 2 §3.5) designs the Windows arm: the SSH home in
//! libgit2's order, `HOME`, then `HOMEDRIVE` plus `HOMEPATH`, then
//! `USERPROFILE`; a visible Pageant window as the agent source before the pipe
//! `SSH_AUTH_SOCK` names, or the OpenSSH agent's default pipe; and WinHTTP's
//! default proxy configuration, read once per runtime beside the snapshot,
//! with the precedence TR1.8 states over the environment's proxy. S4.4, TR4.8
//! and TR4.9 write it as a sibling `mod platform` in the `cfg_if!` below when
//! S4.5 opens `transport_host` to Windows. Until then the `else` arm fails a
//! build that reaches it.

use super::{HttpsEndpointConfig, SshEndpointConfig, apply_native_timeout, invalid};
use crate::git::endpoint::{ca_bundle, https_auth, https_connection};
use crate::model::ModelResult;
use crate::session_host::EnvironmentSnapshot;
use gwz_transport::pool;
use std::{ffi::OsString, path::PathBuf};

/// The SSH and HTTPS endpoint configuration of one runtime, from
/// `environment`.
///
/// The TLS and proxy settings are checked first, as `with_local_transport`
/// always checked them: an unusable CA file or proxy refuses the operation
/// before any endpoint exists. The CA file is read, and each of its
/// certificates taken as a root (TR2.7), here. The transport
/// timeout is gwz state, not environment, which `configure_transport_runtime`
/// sets; it is read once, here, as the runtime starts.
pub(super) fn endpoint_config(
    environment: &EnvironmentSnapshot,
) -> ModelResult<(SshEndpointConfig, HttpsEndpointConfig)> {
    let tls = tls_config(environment)?;
    let home =
        platform::ssh_home(environment).ok_or_else(|| invalid("endpoint HOME is unavailable"))?;
    let timeout = crate::git::transport_timeout_ms();
    let mut pool = pool::Config::default();
    apply_native_timeout(&mut pool, timeout);
    let ssh = SshEndpointConfig {
        home,
        agent: platform::agent(environment),
        pool,
        io_timeout_ms: timeout,
    };
    // The helper is spawned with env_clear() and exactly these entries.
    let environment = environment
        .entries()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let auth = https_auth::Config {
        executable: PathBuf::from("git"),
        environment,
    };
    Ok((
        ssh,
        HttpsEndpointConfig {
            tls,
            auth: Some(auth),
        },
    ))
}

/// The value of the first of `names` that is set and not empty, under the
/// platform's name rules.
fn value(environment: &EnvironmentSnapshot, names: &[&str]) -> Option<OsString> {
    names.iter().find_map(|name| {
        let value = environment.get(name)?.as_os_str();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn tls_config(environment: &EnvironmentSnapshot) -> ModelResult<https_connection::Config> {
    let mut config = https_connection::Config::default();
    if let Some(path) = value(environment, &["GIT_SSL_CAINFO", "SSL_CERT_FILE"]) {
        use std::io::Read;
        let file =
            std::fs::File::open(path).map_err(|_| invalid("cannot read endpoint CA file"))?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("cannot read endpoint CA file"))?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid("endpoint CA file exceeds 1 MiB"));
        }
        config.ca_roots = ca_bundle::certificates(&bytes).map_err(|refusal| match refusal {
            ca_bundle::Refusal::Malformed => {
                invalid("endpoint CA file has a malformed certificate block")
            }
            ca_bundle::Refusal::NoCertificate => invalid("endpoint CA file has no certificate"),
        })?;
    }
    platform::proxy(environment, &mut config)?;
    config
        .validate()
        .map_err(|_| invalid("unsupported endpoint TLS/proxy configuration"))?;
    Ok(config)
}

/// The proxy and the bypass list that the environment's variables name.
fn environment_proxy(
    environment: &EnvironmentSnapshot,
    config: &mut https_connection::Config,
) -> ModelResult<()> {
    if let Some(raw) = value(
        environment,
        &["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"],
    ) {
        let raw = raw
            .to_str()
            .ok_or_else(|| invalid("invalid endpoint proxy"))?;
        let proxy = url::Url::parse(raw).map_err(|_| invalid("invalid endpoint proxy"))?;
        if !matches!(proxy.scheme(), "http" | "https")
            || !proxy.username().is_empty()
            || proxy.password().is_some()
            || proxy.query().is_some()
            || proxy.fragment().is_some()
            || proxy.path() != "/"
        {
            return Err(invalid(
                "alpha supports HTTP(S) proxies without URL credentials or paths",
            ));
        }
        config.proxy = Some(https_connection::Proxy {
            host: proxy
                .host_str()
                .ok_or_else(|| invalid("proxy host required"))?
                .trim_start_matches('[')
                .trim_end_matches(']')
                .into(),
            port: proxy
                .port_or_known_default()
                .ok_or_else(|| invalid("proxy port required"))?,
            tls: proxy.scheme() == "https",
            authorization: None,
        });
    }
    if let Some(raw) = value(environment, &["no_proxy", "NO_PROXY"]) {
        config.no_proxy = raw
            .to_str()
            .ok_or_else(|| invalid("invalid endpoint no_proxy"))?
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    Ok(())
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// Unix: the snapshot's variables alone.
        mod platform {
            use super::{EnvironmentSnapshot, ModelResult, PathBuf, environment_proxy, https_connection, value};

            /// The SSH home, whose `.ssh/known_hosts` the endpoint reads:
            /// `HOME`, when it is absolute.
            pub(super) fn ssh_home(environment: &EnvironmentSnapshot) -> Option<PathBuf> {
                value(environment, &["HOME"])
                    .map(PathBuf::from)
                    .filter(|home| home.is_absolute())
            }

            /// The agent: the socket `SSH_AUTH_SOCK` names, when it is set.
            pub(super) fn agent(environment: &EnvironmentSnapshot) -> Option<PathBuf> {
                value(environment, &["SSH_AUTH_SOCK"]).map(PathBuf::from)
            }

            /// The HTTPS proxy and its bypass list: the environment's.
            pub(super) fn proxy(
                environment: &EnvironmentSnapshot,
                config: &mut https_connection::Config,
            ) -> ModelResult<()> {
                environment_proxy(environment, config)
            }
        }
    } else {
        compile_error!(
            "transport_host::endpoint_environment has no Windows arm yet: TR1.8 designs it, \
             and S4.4, TR4.8 and TR4.9 write it as a sibling `mod platform` here"
        );
    }
}
