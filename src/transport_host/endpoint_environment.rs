//! The endpoint environment: what a transport runtime's endpoint
//! configuration is derived from. It is an environment snapshot, and nothing
//! here reads the process environment (1.1.0 S6.1, as amendment 2's §3.17
//! extends it; gwz-py `dev-docs/GwzPyPerOperationTransportDesign.md` §2.2).
//!
//! Both entries derive their runtime's configuration here: the cancellable
//! entry from the snapshot its caller passes, and `with_local_transport` from
//! the process environment it captures once, at its command's start.
//!
//! Unix uses its captured environment proxy settings. Windows WH1 admits only
//! a verified WinHTTP DIRECT snapshot captured on the original caller entry.
//! It has no SSH settings and never derives a HOME or agent for HTTPS.

use super::{EndpointSettings, HttpsEndpointConfig, apply_native_timeout, invalid};
use crate::git::endpoint::{ca_bundle, https_connection};
use crate::model::ModelResult;
use crate::session_host::EnvironmentSnapshot;
use gwz_transport::pool;
use std::ffi::OsString;
cfg_if::cfg_if! { if #[cfg(unix)] {
    use super::SshSettings;
    use crate::git::endpoint::https_auth;
    use std::path::PathBuf;
} }

cfg_if::cfg_if! { if #[cfg(test)] {
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
) -> ModelResult<(EndpointSettings, HttpsEndpointConfig)> {
    endpoint_config_native(environment, capture_qualification_proxy())
}

} }
pub(super) fn endpoint_config_native(
    environment: &EnvironmentSnapshot,
    direct: Option<bool>,
) -> ModelResult<(EndpointSettings, HttpsEndpointConfig)> {
    if cfg!(all(
        windows,
        gwz_transport_candidate,
        gwz_windows_https_qualification
    )) && direct != Some(true)
    {
        return Err(super::unsupported(
            "Windows HTTPS qualification requires verified WinHTTP DIRECT",
        ));
    }
    let tls = tls_config(environment)?;
    let ssh = {
        cfg_if::cfg_if! { if #[cfg(unix)] {
            Some(SshSettings {
                home: platform::ssh_home(environment).ok_or_else(|| invalid("endpoint HOME is unavailable"))?,
                agent: platform::agent(environment),
            })
        } else { None } }
    };
    let timeout = crate::git::transport_timeout_ms();
    let mut pool = pool::Config::default();
    apply_native_timeout(&mut pool, timeout);
    let local = EndpointSettings {
        ssh,
        pool,
        io_timeout_ms: timeout,
    };
    let auth = {
        cfg_if::cfg_if! { if #[cfg(unix)] {
            // The Unix helper is spawned with env_clear and exactly this snapshot.
            Some(https_auth::Config {
                executable: PathBuf::from("git"),
                environment: environment.entries().map(|(name, value)| (name.to_owned(), value.to_owned())).collect(),
            })
        } else { None } }
    };
    Ok((local, HttpsEndpointConfig { tls, auth }))
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
    cfg_if::cfg_if! { if #[cfg(unix)] { platform::proxy(environment, &mut config)?; } }
    config
        .validate()
        .map_err(|_| invalid("unsupported endpoint TLS/proxy configuration"))?;
    Ok(config)
}

cfg_if::cfg_if! { if #[cfg(unix)] {
/// The proxy and bypass list named in the captured Unix environment.
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

} }

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
    } else if #[cfg(all(windows, gwz_transport_candidate, gwz_windows_https_qualification))] {
        mod platform {
            use windows_sys::Win32::Foundation::GlobalFree;
            use windows_sys::Win32::Networking::WinHttp::{WinHttpGetDefaultProxyConfiguration, WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_PROXY_INFO};
            pub(super) fn direct() -> bool {
                // SAFETY: initialized exclusive output. Every returned allocation,
                // including partial failure outputs, is disposed below.
                let mut info: WINHTTP_PROXY_INFO = unsafe { std::mem::zeroed() };
                let ok = unsafe { WinHttpGetDefaultProxyConfiguration(&mut info) };
                let direct = verified_direct(ok != 0, info.dwAccessType, !info.lpszProxy.is_null(), !info.lpszProxyBypass.is_null());
                for pointer in [info.lpszProxy, info.lpszProxyBypass] {
                    if !pointer.is_null() {
                        // SAFETY: WinHTTP returns storage allocated for GlobalFree.
                        unsafe { GlobalFree(pointer.cast()); }
                    }
                }
                direct
            }
            pub(super) fn verified_direct(ok: bool, access: u32, named: bool, bypass: bool) -> bool {
                ok && access == WINHTTP_ACCESS_TYPE_NO_PROXY && !named && !bypass
            }
            cfg_if::cfg_if! { if #[cfg(test)] {
                #[test]
                fn only_verified_direct_without_named_outputs_is_admitted() {
                    assert!(verified_direct(true, WINHTTP_ACCESS_TYPE_NO_PROXY, false, false));
                    assert!(!verified_direct(false, WINHTTP_ACCESS_TYPE_NO_PROXY, false, false));
                    assert!(!verified_direct(true, 3, false, false));
                    assert!(!verified_direct(true, WINHTTP_ACCESS_TYPE_NO_PROXY, true, false));
                    assert!(!verified_direct(true, WINHTTP_ACCESS_TYPE_NO_PROXY, false, true));
                }
            } }
        }
    } else { compile_error!("unsupported transport endpoint platform"); }
}

/// Read once at original entry beside the environment; never on HTTP worker poll.
pub(crate) fn capture_qualification_proxy() -> Option<bool> {
    cfg_if::cfg_if! { if #[cfg(all(windows, gwz_transport_candidate, gwz_windows_https_qualification))] { Some(platform::direct()) } else { None } }
}
