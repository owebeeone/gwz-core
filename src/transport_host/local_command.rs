//! Candidate-only synchronous command embedding for the local alpha.
use super::{
    CleanupReport, HttpsEndpointConfig, SshEndpointConfig, TransportRequest, TransportRuntime,
    invalid, unavailable,
};
use crate::git::endpoint::{https_auth, https_connection};
use crate::{RequestMeta, git::Git2Backend, model::ModelResult};
use std::{ffi::OsString, path::PathBuf};

/// Own one command's shared SSH/HTTPS endpoint, including bounded cleanup on error.
/// Credentials and environment are captured on this local endpoint only.
pub fn with_local_transport<T>(
    meta: RequestMeta,
    operation: String,
    action: impl FnOnce(&Git2Backend) -> T,
) -> ModelResult<(T, CleanupReport)> {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| unavailable("local transport executor unavailable"))?;
    let environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let tls = tls_config(&environment)?;
    let runtime = TransportRuntime::with_https(
        SshEndpointConfig::from_environment()?,
        HttpsEndpointConfig {
            tls,
            auth: Some(https_auth::Config {
                executable: PathBuf::from("gh"),
                environment,
            }),
        },
    )?;
    let request = executor.block_on(runtime.request(meta, operation))?;
    let mut command = Command {
        executor,
        runtime,
        request: Some(request),
    };
    let result = action(command.request.as_ref().unwrap().backend());
    let cleanup = command.finish();
    Ok((result, cleanup))
}
struct Command {
    executor: tokio::runtime::Runtime,
    runtime: TransportRuntime,
    request: Option<TransportRequest>,
}
impl Command {
    fn finish(&mut self) -> CleanupReport {
        let Some(request) = self.request.take() else {
            return CleanupReport::default();
        };
        self.executor.block_on(async {
            let operation = request.finish().await;
            let runtime = self.runtime.shutdown().await;
            // Shutdown is the final snapshot of the same owned work, not a
            // second set of jobs to add to the operation's earlier snapshot.
            CleanupReport {
                pending_local_work: runtime.pending_local_work,
                peer_cleanup_confirmed: operation.peer_cleanup_confirmed
                    && runtime.peer_cleanup_confirmed,
            }
        })
    }
}
impl Drop for Command {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
fn value(environment: &[(OsString, OsString)], names: &[&str]) -> Option<OsString> {
    names.iter().find_map(|name| {
        environment
            .iter()
            .find(|(k, v)| k == name && !v.is_empty())
            .map(|(_, v)| v.clone())
    })
}
fn tls_config(environment: &[(OsString, OsString)]) -> ModelResult<https_connection::Config> {
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
        config.ca_pem = Some(bytes);
    }
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
    config
        .validate()
        .map_err(|_| invalid("unsupported endpoint TLS/proxy configuration"))?;
    Ok(config)
}
