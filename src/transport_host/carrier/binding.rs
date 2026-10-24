//! The binding capability a carrier end hands the composition (contract §6: "the carrier adapter attaches the known
//! binding capability to incoming messages before dispatch").
use super::{Arrival, local::PeerIdentity};
use std::fmt;

/// A carrier connection's identity: 128 random bits drawn when the connection is made and shared by its two ends.
/// It is never reused and not derived from anything a peer can choose, so a message cannot claim another
/// connection's.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct CarrierId([u8; 16]);

impl CarrierId {
    pub(crate) fn random() -> std::io::Result<Self> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|_| std::io::Error::other("no random bytes for a carrier id"))?;
        Ok(Self(bytes))
    }
}

impl fmt::Debug for CarrierId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Which party of the split an end is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Side {
    /// The core, which issues stream requests and runs Git; the accepting side of the local carrier.
    Core,
    /// The client, which owns connections and credentials; the connecting side.
    Client,
}

/// Who the other end is, as the carrier established it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Peer {
    /// The other end of an in-process pair: the same process, so nothing to check.
    InProcess,
    /// A process on the other end of a byte stream whose credentials the peer check read and accepted.
    Verified(PeerIdentity),
}

/// The capability to speak on one carrier connection, as one side of it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CarrierBinding {
    id: CarrierId,
    side: Side,
    peer: Peer,
}

impl CarrierBinding {
    pub(crate) fn new(id: CarrierId, side: Side, peer: Peer) -> Self {
        Self { id, side, peer }
    }

    pub(crate) fn id(&self) -> CarrierId {
        self.id
    }

    pub(crate) fn side(&self) -> Side {
        self.side
    }

    pub(crate) fn peer(&self) -> &Peer {
        &self.peer
    }

    /// Whether `arrival` came in on the connection this binding bound, at this side of it. A binding is valid only
    /// there: an arrival from another connection, or from the other side of this one, is not admitted.
    pub(crate) fn admits(&self, arrival: &Arrival) -> bool {
        arrival.binding.id == self.id && arrival.binding.side == self.side
    }
}
