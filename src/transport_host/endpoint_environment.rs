//! The endpoint environment: what a transport runtime's endpoint
//! configuration is derived from. It is an environment snapshot, and nothing
//! here reads the process environment (1.1.0 S6.1, as amendment 2's §3.17
//! extends it; gwz-py `dev-docs/GwzPyPerOperationTransportDesign.md` §2.2).
//!
//! Both entries derive their runtime's configuration here: the cancellable
//! entry from the snapshot its caller passes, and `with_local_transport` from
//! the process environment it captures once, at its command's start.
//!
//! Unix uses its captured environment proxy settings. Windows reads the WinHTTP
//! machine proxy once on the original caller entry (`machine_proxy`, plan step
//! 4.6) and never the environment's: a direct setting is admitted, an unusable
//! one is recorded with a text that names it and refuses an https open (the
//! configuration still builds, decision D6), and a named proxy
//! (with its bypass rules) is the HTTPS endpoint's proxy for every https
//! connection (plan step 4.7). Its SSH home is chosen by the home contract on
//! every platform (`git::endpoint::ssh_home`, plan step 3.1), and its agent is chosen
//! by `git::endpoint::agent_source` (plan step 3.2b). It never derives a HOME or agent for HTTPS. On both platforms
//! the credential helper is `git` with the snapshot as its environment (plan
//! step 4.3).

use super::{EndpointSettings, HttpsEndpointConfig, apply_native_timeout, invalid, unsupported};
use crate::git::endpoint::{ca_bundle, https_connection, ssh_home};
cfg_if::cfg_if! { if #[cfg(not(any(windows, target_vendor = "apple")))] {
    use crate::git::endpoint::{https_tls, verify_paths};
} }
use super::SshSettings;
use crate::git::endpoint::agent_source::AgentSource;
use crate::git::endpoint::https_auth;
use crate::model::ModelResult;
use crate::session_host::EnvironmentSnapshot;
use gwz_transport::pool;
use std::ffi::OsString;
use std::path::PathBuf;

cfg_if::cfg_if! {
    if #[cfg(any(windows, test))] {
        pub(crate) use super::machine_proxy::MachineProxy;
    } else {
        /// Unix takes its proxy from the environment, so it never captures a machine proxy.
        #[derive(Clone, Debug)]
        pub(crate) enum MachineProxy {}

        impl MachineProxy {
            pub(crate) fn apply(&self, _: &mut https_connection::Config) {
                match *self {}
            }
        }
    }
}

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
    endpoint_config_native(environment, None, capture_machine_proxy())
}

/// `endpoint_config`, with the current directory a driver captured beside the snapshot.
pub(super) fn endpoint_config_in(
    environment: &EnvironmentSnapshot,
    cwd: &std::path::Path,
) -> ModelResult<(EndpointSettings, HttpsEndpointConfig)> {
    endpoint_config_native(environment, Some(cwd), capture_machine_proxy())
}

} }

/// `cwd` is the current directory that the driver captured beside `environment`, against which a relative home is
/// resolved where the home contract allows one (design section 3); a driver that captured none passes `None`, and a
/// relative home is then skipped.
pub(super) fn endpoint_config_native(
    environment: &EnvironmentSnapshot,
    cwd: Option<&std::path::Path>,
    machine: Option<MachineProxy>,
) -> ModelResult<(EndpointSettings, HttpsEndpointConfig)> {
    if cfg!(all(
        windows,
        gwz_transport_candidate,
        gwz_windows_https_qualification
    )) && machine.is_none()
    {
        return Err(unsupported(
            "Windows HTTPS qualification requires the WinHTTP machine proxy setting, captured on the original entry",
        ));
    }
    let tls = tls_config(environment, machine.as_ref())?;
    // The one home contract (plan step 3.1): `known_hosts` and `~/` identities resolve under the home chosen here,
    // once, from the snapshot and the current directory its driver captured.
    let home = ssh_home::resolve(ssh_home::PLATFORM, environment, cwd, &ssh_home::Disk)
        .map_err(|refusal| invalid(&refusal.to_string()))?;
    let ssh = Some(SshSettings {
        home: home.path,
        home_source: home.source,
        agent: agent(environment),
    });
    let timeout = crate::git::transport_timeout_ms();
    let mut pool = pool::Config::default();
    apply_native_timeout(&mut pool, timeout);
    let local = EndpointSettings {
        ssh,
        pool,
        io_timeout_ms: timeout,
    };
    // The helper is spawned with exactly this snapshot as its environment, on either platform; Windows finds
    // `git.exe` on the snapshot's own `PATH` (`https_auth::executable`).
    let auth = Some(https_auth::Config {
        executable: PathBuf::from("git"),
        environment: environment
            .entries()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
    });
    Ok((local, HttpsEndpointConfig { tls, auth }))
}

/// The agent the snapshot's `SSH_AUTH_SOCK` selects: a socket path on Unix, and on Windows the one agent of TR1.8 §4
/// (plan step 3.2b). A value Windows refuses is carried as a refused source, so that the runtime is built and only a
/// setup that needs an agent is refused.
fn agent(environment: &EnvironmentSnapshot) -> AgentSource {
    AgentSource::capture(value(environment, &["SSH_AUTH_SOCK"]).as_deref())
}

/// The value of the first of `names` that is set and not empty, under the
/// platform's name rules.
fn value(environment: &EnvironmentSnapshot, names: &[&str]) -> Option<OsString> {
    names.iter().find_map(|name| {
        let value = environment.get(name)?.as_os_str();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

cfg_if::cfg_if! {
    if #[cfg(not(any(windows, target_vendor = "apple")))] {
        /// The CA file the snapshot names for the endpoint. Where the backend is
        /// OpenSSL, `SSL_CERT_FILE` and `SSL_CERT_DIR` are not that but OpenSSL's
        /// default verify paths, the platform's roots (`platform_roots`).
        const CA_FILE_VARIABLES: &[&str] = &["GIT_SSL_CAINFO"];

        /// The platform's roots where the backend is OpenSSL: what OpenSSL's
        /// default verify paths hold, as the snapshot's `SSL_CERT_FILE` and
        /// `SSL_CERT_DIR` and the standard locations give them, and nothing
        /// else (`verify_paths`). The backend's own roots are left out, so that
        /// setting either variable narrows trust as it does in gwz 1.0.17.
        fn platform_roots(environment: &EnvironmentSnapshot) -> https_tls::RootsLoader {
            let file = value(environment, &["SSL_CERT_FILE"]);
            let dir = value(environment, &["SSL_CERT_DIR"]);
            std::sync::Arc::new(move || {
                verify_paths::Paths::resolve(file.as_deref(), dir.as_deref(), standard_paths)
                    .roots()
            })
        }

        /// The locations a start of libgit2's host finds for the variables that
        /// name none: `openssl-probe` 0.1's.
        fn standard_paths() -> verify_paths::Paths {
            let probed = openssl_probe::probe();
            verify_paths::Paths {
                file: probed.cert_file,
                dir: probed.cert_dir,
            }
        }
    } else {
        /// The CA file the snapshot names for the endpoint.
        const CA_FILE_VARIABLES: &[&str] = &["GIT_SSL_CAINFO", "SSL_CERT_FILE"];
    }
}

/// The HTTPS endpoint's TLS and proxy configuration. The proxy comes from the machine setting whenever one was
/// captured (a setting that cannot be used is recorded in the configuration and refuses an https open, not the
/// configuration, decision D6); the environment's proxy is read only where no machine setting is (design §6).
fn tls_config(
    environment: &EnvironmentSnapshot,
    machine: Option<&MachineProxy>,
) -> ModelResult<https_connection::Config> {
    let mut config = https_connection::Config::default();
    if let Some(machine) = machine {
        machine.apply(&mut config);
    }
    if let Some(path) = value(environment, CA_FILE_VARIABLES) {
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
    cfg_if::cfg_if! { if #[cfg(not(any(windows, target_vendor = "apple")))] {
        config.platform_roots = Some(platform_roots(environment));
    } }
    if machine.is_none() {
        platform::proxy(environment, &mut config)?;
    }
    config
        .validate()
        .map_err(|_| invalid("unsupported endpoint TLS/proxy configuration"))?;
    Ok(config)
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// Unix: the snapshot's variables alone.
        mod platform {
            use super::{EnvironmentSnapshot, ModelResult, https_connection, invalid, value};

            /// The HTTPS proxy and its bypass list: the environment's.
            pub(super) fn proxy(
                environment: &EnvironmentSnapshot,
                config: &mut https_connection::Config,
            ) -> ModelResult<()> {
                environment_proxy(environment, config)
            }

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
        }
    } else if #[cfg(all(windows, gwz_transport_candidate, gwz_windows_https_qualification))] {
        mod platform {
            use super::{EnvironmentSnapshot, ModelResult, https_connection};
            /// Windows reads no proxy from the environment: the machine setting is the proxy source (design §6), and
            /// the qualification refuses to build a configuration without one.
            pub(super) fn proxy(
                _: &EnvironmentSnapshot,
                _: &mut https_connection::Config,
            ) -> ModelResult<()> {
                Ok(())
            }
        }
    } else { compile_error!("unsupported transport endpoint platform"); }
}

/// Read once at original entry beside the environment; never on HTTP worker poll. Only the Windows qualification
/// reads the machine setting; Unix has none, and other builds have no HTTPS transport on Windows to configure.
pub(crate) fn capture_machine_proxy() -> Option<MachineProxy> {
    cfg_if::cfg_if! { if #[cfg(all(windows, gwz_transport_candidate, gwz_windows_https_qualification))] { Some(super::machine_proxy::capture_machine()) } else { None } }
}
