//! The WinHTTP machine proxy (design `GwzTransportWindowsParityDesign.md` §6; plan steps 4.6 and 4.7).
//!
//! On Windows the machine setting is the one proxy source for an https origin: `HTTPS_PROXY`, `ALL_PROXY`,
//! `NO_PROXY` and git's `http.proxy` are never consulted, whether the machine proxy is named or direct. It is read
//! once, beside the environment snapshot, into an owned [`MachineProxy`]: direct, a named proxy with its bypass
//! rules, or a refusal that names the setting (which refuses an https open and nothing else, decision D6). Nothing here decides by resolving a name, and nothing goes direct
//! because a form could not be read: a request goes direct only where a stated bypass rule matches it.
//!
//! The grammar and the selection are pure and read the machine through two injected sources ([`ProxySource`] and
//! [`AddressSource`]), so they run on any host without touching its settings. The real sources are `winhttp`'s.

mod bypass;
mod proxy_string;
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod winhttp;
        pub(crate) use winhttp::capture_machine;
    }
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod bypass_tests;
        mod capture_tests;
        mod proxy_string_tests;
        mod refusal_tests;
        mod testing;
        mod tunnel_tests;
    }
}
cfg_if::cfg_if! { if #[cfg(all(windows, test))] { mod host_rows; mod host_tunnel; } }

use crate::git::endpoint::https_connection;
use std::{net::IpAddr, sync::Arc};

pub(crate) use bypass::Bypass;
pub(crate) use proxy_string::ProxyEntry;
use proxy_string::{Selection, select_https};

/// How much of an administrator's text a refusal quotes.
const QUOTE_LIMIT: usize = 512;

/// What `WinHttpGetDefaultProxyConfiguration` said, as owned values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Configuration {
    pub(crate) access: Access,
    pub(crate) proxy: Option<String>,
    pub(crate) bypass: Option<String>,
}

/// The WinHTTP access type of the machine setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Access {
    /// `WINHTTP_ACCESS_TYPE_NO_PROXY`: direct access.
    #[default]
    Direct,
    /// `WINHTTP_ACCESS_TYPE_NAMED_PROXY`: a static proxy list.
    Named,
    /// Automatic configuration (PAC or WPAD), or an access type this transport does not know.
    Automatic,
    /// The call failed, or its text could not be read.
    Unreadable,
}

/// Where the machine setting is read from. The Windows source calls WinHTTP; tests inject their own.
pub(crate) trait ProxySource {
    fn read(&self) -> Configuration;
}

/// The addresses of this machine's interfaces, for the own-address rule (design §6, S9).
pub(crate) trait AddressSource {
    fn addresses(&self) -> Vec<IpAddr>;
}

/// The machine proxy as the transport uses it, captured once.
#[derive(Clone, Debug)]
pub(crate) enum MachineProxy {
    /// WinHTTP access type `NO_PROXY`: connect directly.
    Direct,
    /// Every https connection goes through this proxy unless a bypass rule matches.
    Named(Arc<Named>),
    /// The setting cannot be used; the text names it and the next action. Every https open is refused with it.
    Refused(Arc<str>),
}

/// A named proxy and the rules that send a destination around it.
#[derive(Debug)]
pub(crate) struct Named {
    pub(crate) proxy: ProxyEntry,
    pub(crate) bypass: Arc<Bypass>,
}

impl MachineProxy {
    /// Makes `config` use this machine setting alone: the environment's proxy and bypass list are replaced, and the
    /// setting is recorded beside them. A refused setting does not refuse the configuration (decision D6): it leaves
    /// no proxy at all and is recorded with its text, which refuses every https open before any connection, and
    /// SSH and local operations are untouched.
    pub(crate) fn apply(&self, config: &mut https_connection::Config) {
        match self {
            Self::Direct => {
                config.proxy = None;
                config.no_proxy.clear();
                config.machine_bypass = None;
                config.machine = https_connection::MachineSetting::Direct;
            }
            Self::Named(named) => {
                // No credential accompanies the CONNECT: the proxy has none, and the tunnel carries no origin one.
                config.proxy = Some(https_connection::Proxy {
                    host: named.proxy.host.clone(),
                    port: named.proxy.port,
                    tls: false,
                    authorization: None,
                });
                config.no_proxy.clear();
                config.machine_bypass = Some(named.bypass.clone());
                config.machine = https_connection::MachineSetting::Named;
            }
            Self::Refused(text) => {
                config.proxy = None;
                config.no_proxy.clear();
                config.machine_bypass = None;
                config.machine = https_connection::MachineSetting::Refused(text.clone());
            }
        }
    }

    /// Reads `source` once and, only for a named proxy whose list is readable, enumerates `addresses` once.
    pub(crate) fn capture(source: &dyn ProxySource, addresses: &dyn AddressSource) -> Self {
        let configuration = source.read();
        match configuration.access {
            Access::Direct => match (&configuration.proxy, &configuration.bypass) {
                (None, None) => Self::Direct,
                (proxy, bypass) => refuse(format!(
                    "The Windows machine proxy setting says direct access and also names a proxy or a bypass list (proxy \"{}\", bypass list \"{}\"). GWZ does not guess which applies, and never connects around a proxy it cannot read. Ask your administrator to restate it with netsh winhttp reset proxy, or netsh winhttp set proxy proxy-server=\"host:port\".",
                    quote(proxy.as_deref().unwrap_or_default()),
                    quote(bypass.as_deref().unwrap_or_default())
                )),
            },
            Access::Automatic => refuse(
                "The Windows machine proxy setting uses automatic configuration (a PAC or WPAD script) or an access type GWZ does not know. GWZ does not support that, and never connects around a proxy it cannot read. Ask your administrator to set a static proxy with netsh winhttp set proxy proxy-server=\"host:port\"."
                    .into(),
            ),
            Access::Unreadable => refuse(
                "The Windows machine proxy setting could not be read (WinHttpGetDefaultProxyConfiguration failed, or returned text GWZ cannot read), so GWZ does not guess a proxy. Check it with netsh winhttp show proxy."
                    .into(),
            ),
            Access::Named => named(&configuration, addresses),
        }
    }
}

/// A named proxy: the entry for https, the bypass rules, or the refusal that says why neither can be had.
fn named(configuration: &Configuration, addresses: &dyn AddressSource) -> MachineProxy {
    let Some(setting) = configuration.proxy.as_deref() else {
        return refuse("The Windows machine proxy setting names no proxy, though it says it names one. GWZ does not guess one, and never connects around a proxy it cannot read. Check it with netsh winhttp show proxy.".into());
    };
    let proxy = match select_https(setting) {
        Ok(Selection::Https(entry)) => entry,
        Ok(Selection::NoHttpsEntry) => return refuse(no_https_entry(setting)),
        Err(why) => return refuse(unreadable("proxy list", setting, &why)),
    };
    let list = configuration.bypass.as_deref().unwrap_or_default();
    match Bypass::parse(list, addresses.addresses()) {
        Ok(bypass) => MachineProxy::Named(Arc::new(Named {
            proxy,
            bypass: Arc::new(bypass),
        })),
        Err(why) => refuse(unreadable("bypass list", list, &why)),
    }
}

fn refuse(text: String) -> MachineProxy {
    MachineProxy::Refused(text.into())
}

/// Text T1 (design §14, item 13): the administrator's fix first, `--transport native` only as the conditional
/// second. 1.0.17 goes direct here; the transport does not (decision D1, register item 9).
fn no_https_entry(setting: &str) -> String {
    format!(
        "The Windows machine proxy setting has no entry for https: \"{}\". GWZ does not send https traffic around a proxy that was set up for other protocols only. Ask your administrator to add an https entry, for example netsh winhttp set proxy proxy-server=\"http=proxy:8080;https=proxy:8080\". Only if a direct connection is acceptable on your network, --transport native connects directly as gwz 1.0 did.",
        quote(setting)
    )
}

/// A value outside the grammar: the setting named, the form said, no direct fallback.
fn unreadable(what: &str, value: &str, why: &str) -> String {
    format!(
        "The Windows machine proxy setting is not one GWZ can read: its {what} \"{}\" {why}. GWZ never connects around a proxy it cannot read. Ask your administrator to restate it with netsh winhttp set proxy proxy-server=\"host:port\" bypass-list=\"...\".",
        quote(value)
    )
}

/// `text` as one bounded line: control characters are escaped and the length is limited.
fn quote(text: &str) -> String {
    let mut quoted = String::new();
    for (index, c) in text.chars().enumerate() {
        if index == QUOTE_LIMIT {
            quoted.push_str("...");
            break;
        }
        if c.is_control() {
            quoted.extend(c.escape_unicode());
        } else {
            quoted.push(c);
        }
    }
    quoted
}
