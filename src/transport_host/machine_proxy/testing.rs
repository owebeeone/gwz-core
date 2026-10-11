//! The injected sources the machine-proxy tests share: no test reads the host's WinHTTP setting or its interfaces.

use super::{Access, AddressSource, Configuration, MachineProxy, ProxySource};
use std::{cell::Cell, net::IpAddr};

/// A source that counts its reads, so that "read once" is a test and not a comment.
pub(super) struct Fixed {
    pub(super) configuration: Configuration,
    pub(super) reads: Cell<usize>,
    pub(super) enumerations: Cell<usize>,
    pub(super) addresses: Vec<IpAddr>,
}

impl Fixed {
    pub(super) fn new(access: Access, proxy: Option<&str>, bypass: Option<&str>) -> Self {
        Self {
            configuration: Configuration {
                access,
                proxy: proxy.map(str::to_owned),
                bypass: bypass.map(str::to_owned),
            },
            reads: Cell::new(0),
            enumerations: Cell::new(0),
            addresses: vec!["10.1.2.3".parse().unwrap(), "192.168.0.9".parse().unwrap()],
        }
    }
    pub(super) fn named(proxy: &str, bypass: Option<&str>) -> Self {
        Self::new(Access::Named, Some(proxy), bypass)
    }
    pub(super) fn capture(&self) -> MachineProxy {
        MachineProxy::capture(self, self)
    }
}

impl ProxySource for Fixed {
    fn read(&self) -> Configuration {
        self.reads.set(self.reads.get() + 1);
        self.configuration.clone()
    }
}

impl AddressSource for Fixed {
    fn addresses(&self) -> Vec<IpAddr> {
        self.enumerations.set(self.enumerations.get() + 1);
        self.addresses.clone()
    }
}
