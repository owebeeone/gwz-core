//! Local endpoint assembly. Paths are supplied by its owner; constructing the
//! shared worker does no trust, key, environment or agent I/O on the caller.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::{
            agent_auth, agent_socket, ssh_key_auth,
            ssh_key_snapshot::Registry,
            ssh_network,
            ssh_setup::{Authenticated, Setup, SetupConnector},
            ssh_worker::Endpoint,
        };
        use super::shared_reservation::{Authority, ReservedConnector};
        use gwz_transport::{
            pool::{Config, Identity, Key},
            protocol::{AuthMethod, Facts},
        };
        use std::{io, path::PathBuf, time::Duration};

        pub(crate) fn connect(
            config: Config,
            known_hosts: PathBuf,
            agent_socket: Option<PathBuf>,
            io_timeout_ms: u64,
        ) -> io::Result<Endpoint> {
            connect_with_authority(
                config.clone(),
                known_hosts,
                agent_socket,
                io_timeout_ms,
                Authority::new(config.total, config.per_host),
            )
        }

        pub(crate) fn connect_with_authority(
            config: Config,
            known_hosts: PathBuf,
            agent_socket: Option<PathBuf>,
            io_timeout_ms: u64,
            authority: Authority,
        ) -> io::Result<Endpoint> {
            let cleanup = Duration::from_millis(config.cleanup_timeout_ms);
            Endpoint::with_registry(
                config,
                Registry::new(),
                move |origin, registry| {
                    ReservedConnector::new(SetupConnector::reported(
                        origin,
                        cleanup,
                        move |key: &Key, identity: &Identity, progress| -> io::Result<Setup> {
                            // Lookup pins the already admitted snapshot. It performs no
                            // file access; the path is never reopened during setup.
                            let selected = match identity {
                                Identity::Explicit(_) => Some(registry.lookup(key, identity)?),
                                Identity::Ambient => None,
                                Identity::Https => return Err(io::ErrorKind::InvalidInput.into()),
                            };
                            let key = key.clone();
                            let known_hosts = known_hosts.clone();
                            let agent_socket = agent_socket.clone();
                            Ok(Box::new(move |control| {
                                let offered = |method| {
                                    let mut facts = progress.lock().unwrap_or_else(|e| e.into_inner());
                                    facts.method = method;
                                    facts.credential_offered = true;
                                    facts.authenticated = None;
                                };
                                let rejected = || {
                                    progress.lock().unwrap_or_else(|e| e.into_inner()).authenticated = Some(false);
                                };
                                let (connection, trusted) =
                                    ssh_network::establish(&key, &known_hosts, &control)?;
                                if let Some(selected) = selected {
                                    return ssh_key_auth::authenticate_reporting(
                                        connection, &trusted, selected, control,
                                        || offered(AuthMethod::SshKey), rejected,
                                    )
                                    .and_then(Authenticated::selected);
                                }
                                let socket = agent_socket.ok_or(io::ErrorKind::NotFound)?;
                                let user = key.username.as_deref().ok_or(io::ErrorKind::InvalidInput)?;
                                let connection = agent_auth::authenticate_reporting(
                                    connection,
                                    user,
                                    &trusted,
                                    control.clone(),
                                    || agent_socket::connect(&socket, control),
                                    || offered(AuthMethod::SshAgent), rejected,
                                )?;
                                Authenticated::new(
                                    connection,
                                    Identity::Ambient,
                                    Facts {
                                        method: AuthMethod::SshAgent,
                                        authenticated: Some(true),
                                        credential_offered: true,
                                        ..Facts::default()
                                    },
                                )
                            }))
                        },
                    ), authority.clone())
                },
                io_timeout_ms,
            )
        }
    }
}
