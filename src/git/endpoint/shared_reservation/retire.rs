//! Retiring the idle connections a lowered ceiling leaves over the reservations (State review P2-3).
//!
//! The ceilings are the live requests' largest limits, or the base when none is live, so they fall when a lone request
//! below the base is admitted or when the largest request ends. Reservations are held by idle connections too, until they
//! are disposed of, so a fall can leave a host holding more than its ceiling allows, and a new connection to it would wait
//! on the idle ones' 60 second timeout. The surplus idle connections are discarded through the pools' own control
//! (`PoolControl::discard_idle`), as `install_capacity` retired them before it installed lower ceilings; their disposal
//! releases the reservations and wakes the connections that wait for one.

use super::*;

/// The sites (scheme, host, port) that hold reservations, by host, with how many each holds: where a host's idle
/// connections are found when the ceilings fall below what it holds.
#[derive(Default)]
pub(super) struct Sites(BTreeMap<String, Vec<(Site, usize)>>);

impl Sites {
    pub(super) fn add(&mut self, site: &Site) {
        let sites = self.0.entry(site.host.clone()).or_default();
        match sites.iter_mut().find(|(known, _)| known == site) {
            Some((_, held)) => {
                *held += 1;
            }
            None => {
                sites.push((site.clone(), 1));
            }
        }
    }
    pub(super) fn remove(&mut self, site: &Site) {
        let Some(sites) = self.0.get_mut(&site.host) else {
            return;
        };
        if let Some((_, held)) = sites.iter_mut().find(|(known, _)| known == site) {
            *held = held.saturating_sub(1);
        }
        sites.retain(|(_, held)| *held > 0);
        if sites.is_empty() {
            self.0.remove(&site.host);
        }
    }
}

impl Authority {
    /// A pool whose connections hold this authority's reservations: its idle connections are retired when the ceilings
    /// fall below what a host or the total holds.
    pub(crate) fn register_pool(&self, control: PoolControl) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pools
            .push(control);
    }

    /// Discards the idle connections that hold reservations beyond the ceilings: while a host holds more than the
    /// per-host ceiling, the idle connections of its sites; then, while all hosts hold more than the total, those of
    /// the busiest hosts. Leased and opening connections are never touched, and nothing waits for the disposal here:
    /// it releases the reservations, which wakes a connection that waits for one inside its own allocation.
    ///
    /// The pools discard by site (`discard_idle`), so a site that holds idle connections loses all of them, not only
    /// the surplus, and not oldest first: `install_capacity` retired every connection of both pools. The count each
    /// discard reports stops the walk, so a host keeps the idle connections of the sites that were not needed.
    pub(super) fn retire_surplus(&self) {
        let (hosts, over_total, pools) = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.pools.is_empty() {
                return;
            }
            let (total_ceiling, host_ceiling) = state.ceilings();
            let over_total = state.total.saturating_sub(total_ceiling);
            let mut hosts: Vec<_> = state
                .sites
                .0
                .iter()
                .map(|(host, sites)| {
                    let mut sites = sites.clone();
                    sites.sort_by_key(|(_, held)| std::cmp::Reverse(*held));
                    let held = state.hosts.get(host).copied().unwrap_or(0);
                    (held, held.saturating_sub(host_ceiling), sites)
                })
                .collect();
            hosts.sort_by_key(|(held, _, _)| std::cmp::Reverse(*held));
            if over_total == 0 && hosts.iter().all(|(_, over, _)| *over == 0) {
                return;
            }
            (hosts, over_total, state.pools.clone())
        };
        let mut retired = 0;
        for (_, over, sites) in &hosts {
            if *over > 0 {
                retired += retire_sites(&pools, sites, *over);
            }
        }
        let mut remaining = over_total.saturating_sub(retired);
        for (_, _, sites) in &hosts {
            if remaining == 0 {
                break;
            }
            remaining -= retire_sites(&pools, sites, remaining).min(remaining);
        }
    }
}

/// Discards the idle connections of `sites`, the largest first, until at least `need` are closed or the sites are
/// done; how many were closed.
fn retire_sites(pools: &[PoolControl], sites: &[(Site, usize)], need: usize) -> usize {
    let mut closed = 0;
    for (site, _) in sites {
        if closed >= need {
            break;
        }
        for pool in pools {
            closed += pool.discard_idle(site);
        }
    }
    closed
}

cfg_if::cfg_if! { if #[cfg(test)] {
impl Authority {
    /// How many pools are registered.
    pub(crate) fn registered_pools(&self) -> usize {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).pools.len()
    }
}
mod tests {
    use super::super::{Authority, ReservedConnector};
    use crate::git::endpoint::ssh_pool::{Connector, PoolHost, Resource};
    use gwz_transport::{
        pool::{Capacity, Checkout, Config, Identity, Key, Lease, Owner, Pool, Request},
        protocol::{Disposition, Failure},
    };
    use std::{
        future::Future,
        io,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    struct Factory;
    struct Fake(Identity);
    impl Connector for Factory {
        type Resource = Fake;
        fn start(&mut self, _: &Key, identity: &Identity, _: Option<u64>) -> Result<Fake, Failure> {
            Ok(Fake(identity.clone()))
        }
    }
    impl Resource for Fake {
        fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
            Poll::Ready(Ok(Some(self.0.clone())))
        }
        fn poll_dispose(&mut self, _: &mut Context<'_>, _: bool) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn reusable(&self) -> bool {
            true
        }
    }

    type Host = PoolHost<ReservedConnector<Factory>>;
    /// Where the pool's idle connections are: the SSH pool and the HTTPS pool of one transport host, which share an
    /// authority (as `Session::endpoint_with_https_native` builds them).
    struct Rig {
        authority: Authority,
        ssh: (Pool, Host),
        https: (Pool, Host),
        now: u64,
    }
    const START: u64 = 10_000;
    /// The allocation timeout of the pools below, which a request waits out at most.
    const ALLOCATION_MS: u64 = 30_000;

    fn limits(per_host: usize) -> Capacity {
        Capacity {
            per_user_host: per_host,
            per_host,
            total: 256,
            max_requests: 1024,
        }
    }
    impl Rig {
        /// The host's base ceilings are (`total`, `per_host`); the pools' own capacity (64 a host) is larger, so that
        /// only the authority bounds a request at 64.
        fn new(total: usize, per_host: usize) -> Self {
            let authority = Authority::new(total, per_host);
            let side = || {
                let (pool, host) = PoolHost::new(
                    Config {
                        per_user_host: 64,
                        per_host: 64,
                        total: 256,
                        allocation_timeout_ms: ALLOCATION_MS,
                        ..Config::default()
                    },
                    ReservedConnector::new(Factory, authority.clone()),
                    START,
                )
                .unwrap();
                authority.register_pool(pool.control());
                (pool, host)
            };
            Self {
                ssh: side(),
                https: side(),
                authority,
                now: START,
            }
        }
        fn tick(&mut self) {
            let mut cx = Context::from_waker(Waker::noop());
            self.ssh.1.step(&mut cx, self.now).unwrap();
            self.https.1.step(&mut cx, self.now).unwrap();
        }
        fn ssh_request(host: &str, owner: &str, per_host: usize, fresh: bool) -> Request {
            let mut request = Request::new(
                Key::ssh("git", host, 22),
                Identity::Ambient,
                Owner::new("session", owner),
            );
            request.limits = Some(limits(per_host));
            request.fresh = fresh;
            request
        }
        fn https_request(host: &str, owner: &str, per_host: usize) -> Request {
            let mut request = Request::new(
                Key::https(host, 443),
                Identity::Https,
                Owner::new("session", owner),
            );
            request.limits = Some(limits(per_host));
            request
        }
        /// The lease of `checkout` once the pools' turns have served it, or `None` while it waits.
        fn served(&mut self, checkout: &mut Checkout, turns: usize) -> Option<Lease> {
            for _ in 0..turns {
                self.tick();
                if let Poll::Ready(result) =
                    pin!(&mut *checkout).poll(&mut Context::from_waker(Waker::noop()))
                {
                    return Some(result.expect("the allocation ran out"));
                }
            }
            None
        }
        /// `count` idle SSH connections to `host`, left by `owner` at `per_host`.
        fn idle_ssh(&mut self, host: &str, owner: &str, per_host: usize, count: usize) {
            let before = self.ssh.0.counts().idle;
            let mut checkouts: Vec<_> = (0..count)
                .map(|_| self.ssh.0.checkout(Self::ssh_request(host, owner, per_host, false)).unwrap())
                .collect();
            let mut leases = Vec::new();
            for checkout in &mut checkouts {
                leases.push(self.served(checkout, 8).expect("within the request's limit"));
            }
            for lease in leases {
                self.ssh.1.release(lease, Disposition::Reusable).unwrap();
            }
            self.tick();
            assert_eq!(self.ssh.0.counts().idle, before + count);
        }
        /// Turns the pools until the reservations have settled.
        fn settle(&mut self) {
            for _ in 0..16 {
                self.tick();
            }
        }
    }

    #[test]
    fn a_request_below_the_base_retires_the_idle_connections_a_replacement_would_wait_behind() {
        // State review P2-3 (a): R1 at the default leaves 6 idle SSH connections to host H; R2 admitted at 4 needs an
        // HTTPS connection to H. The authority's ceiling is 4 and 6 are held, so it deferred the connect until the idle
        // timeout (longer than the allocation): now the surplus idle connections are retired and R2 is served.
        let mut rig = Rig::new(256, 32);
        rig.authority.admit_operation("r1", 256, 32);
        rig.idle_ssh("h", "r1", 32, 6);
        rig.authority.end_operation("r1");
        assert_eq!(rig.authority.counts("h"), (6, 6));
        rig.authority.admit_operation("r2", 256, 4);
        let mut checkout = rig.https.0.checkout(Rig::https_request("h", "r2", 4)).unwrap();
        let lease = rig
            .served(&mut checkout, 64)
            .expect("served within its allocation: the surplus idle connections were retired");
        rig.settle();
        assert_eq!(rig.ssh.0.counts().idle, 0, "the idle SSH connections went");
        assert_eq!(rig.authority.counts("h"), (1, 1), "their reservations were released; the lease holds one");
        drop(lease);
    }

    #[test]
    fn a_request_at_the_default_retires_nothing() {
        // State review P2-3 (b): the same sequence with R2 at the default: the ceiling does not fall below the 6 held,
        // so the idle connections stay and R2 is served beside them.
        let mut rig = Rig::new(256, 32);
        rig.authority.admit_operation("r1", 256, 32);
        rig.idle_ssh("h", "r1", 32, 6);
        rig.authority.end_operation("r1");
        rig.authority.admit_operation("r2", 256, 32);
        let mut checkout = rig.https.0.checkout(Rig::https_request("h", "r2", 32)).unwrap();
        let lease = rig.served(&mut checkout, 64).expect("served at once");
        rig.settle();
        assert_eq!(rig.ssh.0.counts().idle, 6, "nothing was retired");
        assert_eq!(rig.authority.counts("h"), (7, 7));
        drop(lease);
    }

    #[test]
    fn a_request_at_64_ending_retires_the_idle_connections_beyond_the_base() {
        // State review P2-3 (c): R at 64 ends with 60 idle held; the ceiling falls to the base (32 a host). The surplus
        // is retired, and a following request that needs a new connection is served.
        let mut rig = Rig::new(256, 32);
        rig.authority.admit_operation("big", 256, 64);
        rig.idle_ssh("h", "big", 64, 60);
        assert_eq!(rig.authority.counts("h"), (60, 60));
        rig.authority.end_operation("big");
        rig.authority.admit_operation("next", 256, 32);
        let mut checkout = rig
            .ssh
            .0
            .checkout(Rig::ssh_request("h", "next", 32, true))
            .unwrap();
        let lease = rig
            .served(&mut checkout, 64)
            .expect("a new connection is served within the allocation");
        rig.settle();
        let (total, host) = rig.authority.counts("h");
        assert!(host <= 32 && total <= 32, "held {host} on the host after the ceiling fell to 32");
        drop(lease);
    }

    #[test]
    fn a_lowering_never_retires_a_leased_connection() {
        // State review P2-3 (d): 40 leased and 20 idle are held when R at 64 ends; the idle ones go, the leased ones are
        // untouched and keep their reservations.
        let mut rig = Rig::new(256, 32);
        rig.authority.admit_operation("big", 256, 64);
        let mut checkouts: Vec<_> = (0..40)
            .map(|_| rig.ssh.0.checkout(Rig::ssh_request("h", "big", 64, false)).unwrap())
            .collect();
        let mut leases = Vec::new();
        for checkout in &mut checkouts {
            leases.push(rig.served(checkout, 8).expect("within the limit"));
        }
        rig.idle_ssh("h", "big", 64, 20);
        assert_eq!(rig.ssh.0.counts().leased, 40);
        rig.authority.end_operation("big");
        rig.settle();
        let counts = rig.ssh.0.counts();
        assert_eq!(counts.leased, 40, "leased connections are never retired");
        assert_eq!(counts.idle, 0, "the idle ones went");
        assert_eq!(rig.authority.counts("h"), (40, 40));
        for lease in &leases {
            assert!(rig.ssh.1.resource(lease).is_ok(), "a leased resource is still there");
        }
    }

    #[test]
    fn the_total_ceiling_falling_retires_idle_connections_of_the_busiest_hosts() {
        // The total, not a host, is over: 3 idle on each of two hosts (6 held) when a lone request at 4 in total is
        // admitted. Enough idle connections go to bring the total down to its ceiling.
        let mut rig = Rig::new(256, 32);
        rig.authority.admit_operation("r1", 256, 32);
        rig.idle_ssh("a", "r1", 32, 3);
        rig.idle_ssh("b", "r1", 32, 3);
        rig.authority.end_operation("r1");
        assert_eq!(rig.authority.counts("a"), (6, 3));
        rig.authority.admit_operation("r2", 4, 32);
        rig.settle();
        let (total, _) = rig.authority.counts("a");
        assert!(total <= 4, "{total} held in total under a ceiling of 4");
        assert!(rig.ssh.0.counts().idle >= 1, "not everything is retired when a host's worth is enough");
    }
}
} }
