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
//! by `git::endpoint::agent_source` (plan step 3.2b). Both are chosen for every
//! runtime, HTTPS-only ones included, because the one configuration serves SSH and
//! HTTPS: a home that cannot be resolved (on Windows, for example, an explicitly empty
//! `HOME` with no usable `USERPROFILE`) refuses the command even when it uses only
//! HTTPS remotes, and the agent probe (Pageant's window on Windows) runs once. On
//! both platforms the credential helper is `git` with the snapshot as its environment
//! (plan step 4.3).
//!
//! One capture gives an operation its connection-owner inputs too (step 1.5 of
//! dev-docs/GwzTransportReleaseConvergencePlan.md; contract section 7): the
//! `EndpointProfile`, whose canonical bytes are the reuse design's section 3 fields
//! (home, agent, CA content, proxy and bypass, timeouts, build) and whose inputs are
//! the endpoint settings an endpoint factory makes the adapter from; and the
//! `AuthContext`, which holds the helper's executable and environment
//! (`https_auth::OperationAuth`). The helper environment, and so every token and
//! configuration file it names, is in the context alone: never in the profile's
//! bytes or inputs, so never in an endpoint instance.

use super::{EndpointSettings, HttpsEndpointConfig, apply_native_timeout, invalid, unsupported};
use crate::git::endpoint::{ca_bundle, https_connection, ssh_home};
use gwz_connection::{AuthContext, EndpointProfile};
cfg_if::cfg_if! { if #[cfg(not(any(windows, target_vendor = "apple")))] {
    use crate::git::endpoint::{https_tls, verify_paths};
} }
use super::SshSettings;
use crate::git::endpoint::agent_source::AgentSource;
use crate::git::endpoint::https_auth;
use crate::git::endpoint::https_worker::native::NativeCaller;
use crate::model::ModelResult;
use crate::session_host::EnvironmentSnapshot;
use gwz_transport::pool;
use std::ffi::OsString;
use std::path::PathBuf;

cfg_if::cfg_if! {
    if #[cfg(any(windows, test))] {
        pub(crate) use super::machine_proxy::MachineProxy;

        /// The machine setting as canonical bytes, from its parsed, bounded inputs (plan step 1.5, remediation C-P2-3):
        /// the setting (its kind, and the named proxy's port and host or the refusal's text), the bypass rules as the
        /// parser read them, and this machine's own addresses as octets. No `Debug` layout.
        pub(super) fn machine_canonical(machine: &MachineProxy) -> [Vec<u8>; 3] {
            match machine {
                MachineProxy::Direct => [vec![0], Vec::new(), Vec::new()],
                MachineProxy::Named(named) => {
                    let mut setting = vec![1];
                    setting.extend_from_slice(&named.proxy.port.to_le_bytes());
                    setting.extend_from_slice(named.proxy.host.as_bytes());
                    let (rules, own) = named.bypass.canonical();
                    [setting, rules, own]
                }
                MachineProxy::Refused(text) => {
                    let mut setting = vec![2];
                    setting.extend_from_slice(text.as_bytes());
                    [setting, Vec::new(), Vec::new()]
                }
            }
        }
    } else {
        /// Unix takes its proxy from the environment, so it never captures a machine proxy.
        #[derive(Clone, Debug)]
        pub(crate) enum MachineProxy {}

        impl MachineProxy {
            pub(crate) fn apply(&self, _: &mut https_connection::Config) {
                match *self {}
            }
        }

        /// Unix captures no machine setting, so there is none to encode.
        pub(super) fn machine_canonical(machine: &MachineProxy) -> [Vec<u8>; 3] {
            match *machine {}
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
    // The runtime's endpoint takes only the helper from the capture; the caller of this entry reaches the endpoint
    // session itself (`with_https_native`), and the operation's own capture gives its context the caller.
    let capture = capture_operation(environment, cwd, machine, None)?;
    // The runtime's endpoint is its operation's own, so its helper is the one the operation's context holds.
    let auth = https_auth::OperationAuth::of(&capture.auth).map(|auth| auth.helper().clone());
    let inputs = capture.inputs;
    Ok((
        inputs.local.clone(),
        HttpsEndpointConfig {
            tls: inputs.tls.clone(),
            auth,
        },
    ))
}

/// What an endpoint factory makes an operation's adapter from: the endpoint settings and the TLS and proxy
/// configuration, as the profile's validated inputs. It holds no helper, environment or credential.
pub(crate) struct EndpointInputs {
    pub(crate) local: EndpointSettings,
    pub(crate) tls: https_connection::Config,
}

/// One operation's capture: its profile and its authentication context, and the inputs the profile carries.
pub(crate) struct OperationCapture {
    pub(crate) inputs: std::sync::Arc<EndpointInputs>,
    #[allow(
        dead_code,
        reason = "requests are admitted with it from plan phase 3; the factory reads its inputs and the rows admit with it now"
    )]
    pub(crate) profile: EndpointProfile,
    pub(crate) auth: AuthContext,
}

/// The endpoint settings, profile and authentication context of one operation, from `environment`, captured once on its
/// original entry. The TLS and proxy settings are checked first, so an unusable CA file or proxy refuses the operation
/// before any endpoint exists. `native` is the logon-session caller that entry captured, if it captured one: it goes into
/// the operation's context and nowhere else, so no endpoint instance holds a caller and none authenticates as another
/// operation's (R3; draft r5 section 3.2).
pub(super) fn capture_operation(
    environment: &EnvironmentSnapshot,
    cwd: Option<&std::path::Path>,
    machine: Option<MachineProxy>,
    native: Option<NativeCaller>,
) -> ModelResult<OperationCapture> {
    // Windows reads its proxy from the machine and never the environment, so an entry that did not capture the machine
    // setting has nothing to configure the endpoint's HTTPS connections from.
    if cfg!(windows) && machine.is_none() {
        return Err(unsupported(
            "the Windows machine proxy setting was not captured on the original entry",
        ));
    }
    let (tls, trust) = tls_config(environment, machine.as_ref())?;
    profile::refuse_proxy_authorization(&tls)?;
    // The one home contract (plan step 3.1): `known_hosts` and `~/` identities resolve under the home chosen here,
    // once, from the snapshot and the current directory its driver captured.
    let home = ssh_home::resolve(ssh_home::PLATFORM, environment, cwd, &ssh_home::Disk)
        .map_err(|refusal| invalid(&refusal.to_string()))?;
    let ssh = SshSettings {
        home: home.path,
        home_source: home.source,
        agent: agent(environment),
    };
    let timeout = crate::git::transport_timeout_ms();
    let mut pool = pool::Config::default();
    apply_native_timeout(&mut pool, timeout);
    let canonical = profile::canonical(&profile::Fields {
        ssh: &ssh,
        ca: trust.ca.as_ref(),
        platform: trust.platform,
        roots: [
            value(environment, &["SSL_CERT_FILE"]),
            value(environment, &["SSL_CERT_DIR"]),
        ],
        tls: &tls,
        machine: machine.as_ref(),
        timeout,
        pool: &pool,
    });
    let local = EndpointSettings {
        ssh: Some(ssh),
        pool,
        io_timeout_ms: timeout,
    };
    // The helper is spawned with exactly this snapshot as its environment, on either platform; Windows finds
    // `git.exe` on the snapshot's own `PATH` (`https_auth::executable`).
    let auth = https_auth::OperationAuth::context(
        https_auth::Config {
            executable: PathBuf::from("git"),
            environment: environment
                .entries()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        },
        native,
    );
    let inputs = std::sync::Arc::new(EndpointInputs { local, tls });
    let digest = profile::digest(&canonical);
    let length = canonical.len();
    // The encoding's field maxima sum to less than the bound (`profile::the_field_maxima_fit_the_profile_bound`), so
    // this refusal names a broken encoder, not a configuration.
    let profile = EndpointProfile::with_inputs(canonical, digest, inputs.clone()).map_err(|_| {
        invalid(&format!(
            "endpoint profile of {length} bytes exceeds the connection host's bound of {} bytes",
            gwz_connection::MAX_PROFILE_BYTES
        ))
    })?;
    Ok(OperationCapture {
        inputs,
        profile,
        auth,
    })
}

/// The CA file the capture read: the variable's value and the SHA-256 of the bytes it held.
pub(super) struct CaFile {
    path: OsString,
    digest: [u8; 32],
}

/// The trust content a capture read: the CA file's, and on OpenSSL builds the platform roots' (the SHA-256 of their
/// certificates, read once here, which the instance then trusts exactly).
pub(super) struct Trust {
    ca: Option<CaFile>,
    platform: Option<[u8; 32]>,
}

/// The canonical bytes of a profile (the reuse design's section 3 fields, in its order): each field a tag and a
/// length-prefixed value, an absent value distinct from an empty one. Every field has a maximum: a value within it is
/// kept as its bytes, a longer one as its SHA-256 (a marker byte tells which), so a capture the grammars accept always
/// fits `gwz_connection::MAX_PROFILE_BYTES` (remediation C-P2-3). Paths are compared as bytes (WTF-8 on Windows),
/// lexically. The home's source, the agent form and the pool's clocks are their `Debug` text, which names every field
/// that selects them in this build and is bounded (the agent's by field 5's maximum); the Pageant pin's is its window
/// and process, an address the reuse design treats as nothing more (possession is proven at lease, step 1.6). The
/// machine setting is encoded from its parsed inputs (`machine_canonical`), never from its `Debug`.
mod profile {
    use super::{CaFile, MachineProxy, OsString, SshSettings, https_connection, invalid, pool};
    use crate::model::ModelResult;

    /// What the format and the build are: a profile is never compared across either.
    const FORMAT: &[u8] = b"gwz-endpoint-profile/2";
    /// The longest a path or an address may be and still be kept as its bytes.
    const PATH: usize = 4096;
    /// Each field's tag and the most bytes its value is kept as; a longer value is kept as its SHA-256.
    pub(super) const FIELDS: [(u8, usize); 17] = [
        (1, 64),    // format
        (2, 64),    // build
        (3, PATH),  // SSH home
        (4, 64),    // home source
        (5, PATH),  // agent
        (6, PATH),  // CA file path
        (7, 32),    // CA file content digest
        (8, PATH),  // SSL_CERT_FILE
        (9, PATH),  // SSL_CERT_DIR
        (10, 512),  // proxy host, port and TLS flag
        (11, 8192), // bypass list (no_proxy), normalized
        (12, 4200), // machine setting: kind, and the named proxy (string grammar 4096) or the refusal's text
        (13, 8),    // timeout
        (14, 1024), // pool clocks
        (15, 8192), // machine bypass rules (list grammar 8192)
        (16, PATH), // this machine's own addresses
        (17, 32),   // OpenSSL platform roots content digest
    ];
    /// A field's framing: its tag, its length and the marker byte.
    const FRAMING: usize = 1 + 4 + 1;

    fn maximum(tag: u8) -> usize {
        FIELDS
            .iter()
            .find(|(field, _)| *field == tag)
            .map_or(0, |(_, maximum)| *maximum)
    }

    pub(super) struct Fields<'a> {
        pub(super) ssh: &'a SshSettings,
        pub(super) ca: Option<&'a CaFile>,
        /// The SHA-256 of the OpenSSL platform roots the capture read, where that is the backend.
        pub(super) platform: Option<[u8; 32]>,
        /// `SSL_CERT_FILE` and `SSL_CERT_DIR`, which choose OpenSSL's platform roots where that is the backend.
        pub(super) roots: [Option<OsString>; 2],
        pub(super) tls: &'a https_connection::Config,
        pub(super) machine: Option<&'a MachineProxy>,
        pub(super) timeout: u64,
        pub(super) pool: &'a pool::Config,
    }

    struct Writer(Vec<u8>);
    impl Writer {
        fn field(&mut self, tag: u8, value: Option<&[u8]>) {
            self.0.push(tag);
            let Some(bytes) = value else {
                self.0.extend_from_slice(&u32::MAX.to_le_bytes());
                return;
            };
            let digested;
            let (marker, kept) = if bytes.len() > maximum(tag) {
                digested = digest(bytes);
                (1u8, digested.as_slice())
            } else {
                (0u8, bytes)
            };
            let len = u32::try_from(kept.len() + 1).unwrap_or(u32::MAX - 1);
            self.0.extend_from_slice(&len.to_le_bytes());
            self.0.push(marker);
            self.0.extend_from_slice(kept);
        }
    }

    /// A shared endpoint never holds a proxy credential: equal canonical bytes would hand one client's to another
    /// (reuse design section 3; remediation S-P3-1). Today's parsers never set one; this refuses if one ever does.
    pub(super) fn refuse_proxy_authorization(tls: &https_connection::Config) -> ModelResult<()> {
        if tls
            .proxy
            .as_ref()
            .is_some_and(|proxy| proxy.authorization.is_some())
        {
            return Err(invalid(
                "an endpoint proxy with authorization cannot be shared between operations",
            ));
        }
        Ok(())
    }

    pub(super) fn canonical(fields: &Fields<'_>) -> Vec<u8> {
        let mut out = Writer(Vec::new());
        out.field(1, Some(FORMAT));
        out.field(2, Some(env!("CARGO_PKG_VERSION").as_bytes()));
        out.field(3, Some(fields.ssh.home.as_os_str().as_encoded_bytes()));
        out.field(4, Some(format!("{:?}", fields.ssh.home_source).as_bytes()));
        out.field(5, Some(format!("{:?}", fields.ssh.agent).as_bytes()));
        out.field(6, fields.ca.map(|ca| ca.path.as_encoded_bytes()));
        out.field(7, fields.ca.map(|ca| ca.digest.as_slice()));
        for (tag, root) in [8, 9].into_iter().zip(&fields.roots) {
            out.field(tag, root.as_ref().map(|value| value.as_encoded_bytes()));
        }
        let proxy = fields.tls.proxy.as_ref().map(|proxy| {
            let mut bytes = proxy.port.to_le_bytes().to_vec();
            bytes.push(u8::from(proxy.tls));
            bytes.extend_from_slice(proxy.host.as_bytes());
            bytes
        });
        out.field(10, proxy.as_deref());
        let mut bypass: Vec<String> = fields
            .tls
            .no_proxy
            .iter()
            .map(|host| host.to_ascii_lowercase())
            .collect();
        bypass.sort();
        bypass.dedup();
        out.field(11, Some(bypass.join("\0").as_bytes()));
        let machine = fields.machine.map(super::machine_canonical);
        out.field(
            12,
            machine.as_ref().map(|[setting, _, _]| setting.as_slice()),
        );
        out.field(13, Some(&fields.timeout.to_le_bytes()));
        out.field(14, Some(format!("{:?}", fields.pool).as_bytes()));
        out.field(15, machine.as_ref().map(|[_, rules, _]| rules.as_slice()));
        out.field(16, machine.as_ref().map(|[_, _, own]| own.as_slice()));
        out.field(17, fields.platform.as_ref().map(|digest| digest.as_slice()));
        out.0
    }

    pub(super) fn digest(canonical: &[u8]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        Sha256::digest(canonical).into()
    }

    cfg_if::cfg_if! {
        if #[cfg(test)] {
            use crate::git::endpoint::{agent_source::AgentSource, ssh_home::HomeSource};

            /// The canonical bytes of a profile whose only varying inputs are the proxy and the bypass list.
            fn with_proxy(proxy: Option<&str>, bypass: &[&str]) -> Vec<u8> {
                let ssh = SshSettings {
                    home: "home".into(),
                    home_source: HomeSource::Home,
                    agent: AgentSource::None,
                };
                let tls = https_connection::Config {
                    proxy: proxy.map(|host| https_connection::Proxy {
                        host: host.into(),
                        port: 3128,
                        tls: false,
                        authorization: None,
                    }),
                    no_proxy: bypass.iter().map(|host| host.to_string()).collect(),
                    ..https_connection::Config::default()
                };
                canonical(&Fields {
                    ssh: &ssh,
                    ca: None,
                    platform: None,
                    roots: [None, None],
                    tls: &tls,
                    machine: None,
                    timeout: 9_000,
                    pool: &pool::Config::default(),
                })
            }

            /// The proxy (the environment's on Unix, the machine's named proxy on Windows) and its bypass list are
            /// profile fields, on every platform; the bypass list compares as a set of lower-cased hosts.
            #[test]
            fn the_proxy_and_bypass_list_are_profile_fields() {
                let base = with_proxy(Some("proxy-a.example"), &["Internal.example", ".corp.example"]);
                assert_ne!(base, with_proxy(Some("proxy-b.example"), &["internal.example", ".corp.example"]));
                assert_ne!(base, with_proxy(None, &["internal.example", ".corp.example"]));
                assert_ne!(base, with_proxy(Some("proxy-a.example"), &["internal.example"]));
                assert_eq!(base, with_proxy(Some("proxy-a.example"), &[".corp.example", "internal.example"]));
            }

            /// The sum of the field maxima, each with its framing (and a digest's 32 bytes where that is longer), is
            /// under the connection host's bound: no capture the grammars accept is refused for its size (C-P2-3).
            #[test]
            fn the_field_maxima_fit_the_profile_bound() {
                let total: usize = FIELDS
                    .iter()
                    .map(|(_, maximum)| FRAMING + (*maximum).max(32))
                    .sum();
                assert!(total <= gwz_connection::MAX_PROFILE_BYTES, "{total} bytes of fields");
                let tags: std::collections::BTreeSet<u8> = FIELDS.iter().map(|(tag, _)| *tag).collect();
                assert_eq!(tags.len(), FIELDS.len(), "one maximum per tag");
            }

            /// A 16 KiB `NO_PROXY` is kept as its digest, so it fits, and one changed host is another profile.
            #[test]
            fn a_long_bypass_list_fits_and_one_host_changes_the_profile() {
                let hosts: Vec<String> = (0..1024).map(|n| format!("h{n:05}.corp.example")).collect();
                let long: Vec<&str> = hosts.iter().map(String::as_str).collect();
                assert!(long.iter().map(|host| host.len() + 1).sum::<usize>() >= 16 * 1024);
                let first = with_proxy(Some("proxy-a.example"), &long);
                // Kept as its 32-byte digest: the whole profile stays far smaller than the list.
                assert!(first.len() < 2048, "{} bytes", first.len());
                assert!(first.len() <= gwz_connection::MAX_PROFILE_BYTES);
                let mut changed = long.clone();
                changed[512] = "other.corp.example";
                assert_ne!(first, with_proxy(Some("proxy-a.example"), &changed));
            }

            /// A proxy that carries authorization is refused before it can become a shared endpoint's (S-P3-1).
            #[test]
            fn a_proxy_with_authorization_is_refused() {
                let mut tls = https_connection::Config {
                    proxy: Some(https_connection::Proxy {
                        host: "proxy.example".into(),
                        port: 3128,
                        tls: false,
                        authorization: None,
                    }),
                    ..https_connection::Config::default()
                };
                assert!(refuse_proxy_authorization(&tls).is_ok());
                if let Some(proxy) = tls.proxy.as_mut() {
                    proxy.authorization = Some("Basic dXNlcjpwYXNz".into());
                }
                assert!(refuse_proxy_authorization(&tls).is_err());
            }
        }
    }
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
        ///
        /// They are read once, here, at capture (plan step 1.5, remediation
        /// S-P2-3): `config`'s loader returns exactly those certificates, so the
        /// instance trusts what the profile names, and the SHA-256 of their DER
        /// is the profile's field 17.
        fn platform_trust(
            environment: &EnvironmentSnapshot,
            config: &mut https_connection::Config,
        ) -> Option<[u8; 32]> {
            use sha2::{Digest, Sha256};
            let file = value(environment, &["SSL_CERT_FILE"]);
            let dir = value(environment, &["SSL_CERT_DIR"]);
            let roots = verify_paths::Paths::resolve(file.as_deref(), dir.as_deref(), standard_paths)
                .roots();
            let mut digest = Sha256::new();
            for root in &roots {
                let der = root.to_der().unwrap_or_default();
                digest.update((der.len() as u64).to_le_bytes());
                digest.update(&der);
            }
            let roots = std::sync::Arc::new(roots);
            let loader: https_tls::RootsLoader = std::sync::Arc::new(move || roots.as_ref().clone());
            config.platform_roots = Some(loader);
            Some(digest.finalize().into())
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

        /// The platform's own store answers every handshake here (Security.framework, Schannel): no snapshot of it is
        /// taken, so no stale one can be shared, and the CA file is the trust content the profile names.
        fn platform_trust(
            _: &EnvironmentSnapshot,
            _: &mut https_connection::Config,
        ) -> Option<[u8; 32]> {
            None
        }
    }
}

/// The HTTPS endpoint's TLS and proxy configuration. The proxy comes from the machine setting whenever one was
/// captured (a setting that cannot be used is recorded in the configuration and refuses an https open, not the
/// configuration, decision D6); the environment's proxy is read only where no machine setting is (design §6).
fn tls_config(
    environment: &EnvironmentSnapshot,
    machine: Option<&MachineProxy>,
) -> ModelResult<(https_connection::Config, Trust)> {
    let mut config = https_connection::Config::default();
    if let Some(machine) = machine {
        machine.apply(&mut config);
    }
    let mut ca = None;
    if let Some(path) = value(environment, CA_FILE_VARIABLES) {
        use std::io::Read;
        let file =
            std::fs::File::open(&path).map_err(|_| invalid("cannot read endpoint CA file"))?;
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
        // The profile compares what the file held when it was read, not its name alone.
        ca = Some(CaFile {
            path,
            digest: profile::digest(&bytes),
        });
    }
    let platform = platform_trust(environment, &mut config);
    if machine.is_none() {
        platform::proxy(environment, &mut config)?;
    }
    config
        .validate()
        .map_err(|_| invalid("unsupported endpoint TLS/proxy configuration"))?;
    Ok((config, Trust { ca, platform }))
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
    } else if #[cfg(windows)] {
        mod platform {
            use super::{EnvironmentSnapshot, ModelResult, https_connection};
            /// Windows reads no proxy from the environment: the machine setting is the proxy source (design §6), and
            /// the endpoint refuses to build a configuration without one.
            pub(super) fn proxy(
                _: &EnvironmentSnapshot,
                _: &mut https_connection::Config,
            ) -> ModelResult<()> {
                Ok(())
            }
        }
    } else { compile_error!("unsupported transport endpoint platform"); }
}

/// Read once at original entry beside the environment; never on HTTP worker poll. Only Windows reads the
/// machine setting; Unix has none.
pub(crate) fn capture_machine_proxy() -> Option<MachineProxy> {
    cfg_if::cfg_if! { if #[cfg(windows)] { Some(super::machine_proxy::capture_machine()) } else { None } }
}
