//! The in-process carrier between a driver session's port and an endpoint
//! session's port. Each direction moves every message its source has ready,
//! in order, and parks only when neither mux has anything for it.
use super::*;

// The carrier seam lives in `transport_host/carrier/`. `transport_host/mod.rs` is the Phase 1 trunk's file, so the
// module is declared here until the trunk moves the declaration (the patch is in the step 2.1 report).
// Only the in-process carrier has a production caller in this step: the stream and local carriers, their wire and the
// re-exports the rows use are first called by `gwz server` (plan step 2.3), so the module is silent about what is
// unused until then.
#[allow(
    dead_code,
    unused_imports,
    reason = "first production caller of the stream and local carriers: plan step 2.3"
)]
#[path = "../carrier/mod.rs"]
pub(in crate::transport_host) mod carrier;

/// The in-process link between a driver session's port and an endpoint session's port: the default carrier, joined to
/// both ports by one pumping thread. `LocalLink::new(core, peer)` keeps the signature `build_native` calls; where the
/// composition chooses another carrier it builds a `carrier::CarrierLink` over the carrier's end instead (the trunk's
/// patch is in the step 2.1 report).
pub(in crate::transport_host) struct LocalLink(carrier::CarrierLink);

impl LocalLink {
    pub(in crate::transport_host) fn new(
        core: TransportPort,
        peer: TransportPort,
    ) -> ModelResult<Self> {
        carrier::CarrierLink::in_process(core, peer).map(Self)
    }
    pub(in crate::transport_host) fn stop(&self) {
        self.0.stop();
    }
}
