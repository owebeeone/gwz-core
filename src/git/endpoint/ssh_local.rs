//! Local endpoint assembly. Paths are supplied by its owner; constructing the
//! shared worker does no trust, key, environment or agent I/O on the caller.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::{
            agent_auth, agent_socket, ssh_key_auth,
            ssh_handoff::Handoff,
            ssh_key_snapshot::Registry,
            ssh_network,
            ssh_password::{self, Password},
            ssh_pool::Opening,
            ssh_setup::{Authenticated, Setup, SetupConnector},
            ssh_worker::Endpoint,
        };
        use super::shared_reservation::{Authority, ReservedConnector};
        use gwz_transport::{
            pool::{Config, Identity, Key},
            protocol::{AuthMethod, Facts},
        };
        use std::{io, path::PathBuf, time::Duration};

        cfg_if::cfg_if! { if #[cfg(test)] {
        /// The endpoint. A driver in its process deposits each open's URL
        /// extras in `handoff`; any other driver's opens have none (TR2.18).
        pub(crate) fn connect_with_authority(
            config: Config,
            known_hosts: PathBuf,
            agent_socket: Option<PathBuf>,
            io_timeout_ms: u64,
            authority: Authority,
            handoff: Handoff,
        ) -> io::Result<Endpoint> {
            connect_with_helpers(config, known_hosts, agent_socket, io_timeout_ms, authority, handoff, None)
        }
        } }
        pub(crate) fn connect_with_helpers(
            config: Config, known_hosts: PathBuf, agent_socket: Option<PathBuf>, io_timeout_ms: u64,
            authority: Authority, handoff: Handoff,
            helpers: Option<std::sync::Arc<super::ssh_password_helpers::Helpers>>,
        ) -> io::Result<Endpoint> {
            let cleanup = Duration::from_millis(config.cleanup_timeout_ms);
            Endpoint::with_handoff(
                config,
                Registry::new(),
                move |origin, registry| {
                    ReservedConnector::new(SetupConnector::reported(
                        origin,
                        cleanup,
                        move |key: &Key, identity: &Identity, opening: Opening| -> io::Result<Setup> {
                            let helper_opening = opening.clone();
                            let Opening { progress, url, identity: selection, selected: pinned, .. } = opening;
                            let selection = selection.as_ref().unwrap_or(identity);
                            let helpers_allowed = url.as_ref().is_none_or(|url| url.helpers_allowed());
                            // A URL password's open has an identity of its own and
                            // brings its own selected key, if any (TR2.18). Any
                            // other selected key is found by its identity: lookup
                            // pins the already admitted snapshot without file
                            // access, and the path is never reopened during setup.
                            let password = url.clone().filter(|url| url.password().is_some());
                            let selected = match (selection, &password) {
                                (Identity::Explicit(_), Some(_)) => pinned,
                                (Identity::Explicit(_), None) => Some(registry.lookup(key, selection)?),
                                (Identity::Ambient, None) => None,
                                _ => return Err(io::ErrorKind::InvalidInput.into()),
                            };
                            let requested = identity.clone();
                            let isolated = identity != selection;
                            let key = key.clone();
                            // The host as the open's URL wrote it, for a hashed
                            // known_hosts name (TR2.18); else the key's own.
                            let written = url.map_or_else(|| key.host.clone(), |url| url.host().to_owned());
                            let known_hosts = known_hosts.clone();
                            let agent_socket = agent_socket.clone();
                            let helpers = helpers.clone();
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
                                let (mut connection, trusted) = ssh_network::establish_written(
                                    &key,
                                    &written,
                                    &known_hosts,
                                    &control,
                                )?;
                                let user = key.username.as_deref().ok_or(io::ErrorKind::InvalidInput)?;
                                // libgit2 offers a URL's password before any key,
                                // when the server offers password authentication.
                                if let Some(secret) = password.as_ref().and_then(|url| url.password()) {
                                    match ssh_password::authenticate(
                                        connection, user, secret, &trusted, &control,
                                        || offered(AuthMethod::None), rejected,
                                    )? {
                                        Password::Accepted(accepted) => {
                                            let mut facts = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
                                            facts.authenticated = Some(true);
                                            return Authenticated::new(accepted, requested, facts);
                                        }
                                        Password::Declined(declined) => connection = declined,
                                    }
                                }
                                if let Some(selected) = selected {
                                    let authenticated = ssh_key_auth::authenticate_reporting(
                                        connection, &trusted, selected, control,
                                        || offered(AuthMethod::SshKey), rejected,
                                    )
                                    .and_then(Authenticated::selected)?;
                                    return Ok(if password.is_some() || isolated {
                                        authenticated.under(requested)
                                    } else {
                                        authenticated
                                    });
                                }
                                if helpers_allowed && let Some(helpers) = helpers
                                    && ssh_password::password_only(&mut connection, user, &control)? {
                                        progress.lock().unwrap_or_else(|e| e.into_inner()).method = AuthMethod::Gh;
                                        let mut secret = helpers.lookup(&key, &helper_opening, &control)?;
                                        let accepted = ssh_password::authenticate_helper(connection, &mut secret, &trusted, &control,
                                            || offered(AuthMethod::Gh), rejected)?;
                                        let mut facts = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
                                        facts.authenticated = Some(true);
                                        return Authenticated::new(accepted, requested, facts);
                                }
                                let socket = agent_socket.ok_or(io::ErrorKind::NotFound)?;
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
                                    requested,
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
                handoff,
            )
        }
    }
}
