//! Ports for the fixture servers that bind their port themselves (`sshd`, `git-daemon`).
//!
//! Such a server cannot take a listener that is already bound, so its port is picked by binding a socket and
//! releasing it, and in the gap before the server's own bind another socket can be given that port. When the port
//! came from the system's own allocation (`bind` to port 0) it was the system's to give again at once, to any
//! other test's listener, such as a proxy, or to the source port of any connection a test makes: the server then
//! failed to bind, or a probe of the port reached a server that was not the fixture's own. A port below every range
//! an operating system hands out to `bind(0)` and to outgoing connections is nobody's to take but another fixture's,
//! which draws at random from the same range and so meets this one a few times in thousands.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::TcpListener;

/// The first port of the range, and the first port after it. The ranges the systems hand out begin at 32768 (Linux)
/// and 49152 (macOS and Windows).
pub(crate) const FIRST: u16 = 12_000;
pub(crate) const END: u16 = 32_000;

/// How many ports are drawn before it gives up: all but a handful of the range's 20,000 ports are free.
const DRAWS: usize = 1_000;

/// A port in `FIRST..END` that nothing listens on at the moment: drawn at random, then bound and released to see
/// that it is free.
pub(crate) fn fixture_port() -> u16 {
    for _ in 0..DRAWS {
        let draw = RandomState::new().build_hasher().finish();
        let port = FIRST + (draw % u64::from(END - FIRST)) as u16;
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port in {FIRST}..{END} for a fixture server after {DRAWS} draws");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_fixture_port_is_below_every_range_the_system_hands_out_and_is_free() {
        for _ in 0..50 {
            let port = fixture_port();
            assert!((FIRST..END).contains(&port), "{port}");
            drop(TcpListener::bind(("127.0.0.1", port)).unwrap());
        }
    }

    #[test]
    fn fixture_ports_are_drawn_at_random() {
        let ports: HashSet<u16> = (0..20).map(|_| fixture_port()).collect();
        assert!(ports.len() > 10, "{ports:?}");
    }
}
