//! Who is on the other end of a local carrier, and the one rule that decides whether to talk to them.
//!
//! The platform modules read an operating-system identity ([`PeerIdentity`]) from a connected socket or pipe; the rule
//! here ([`PeerPolicy::verify`]) is a pure function of two identities, so its refusals are tested on every platform
//! with identities no single-user host can produce. There is one rule and one way to a policy: [`PeerPolicy::current_user`]
//! reads this process's own identity. No setting, argument or build variant changes what it accepts (a source check,
//! `scripts/checks/check_carrier_peer_rule.py`, fails the build on a hook that would).
use super::LocalRefusal;
use std::fmt;

/// The account a process runs as.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Principal {
    /// A Unix user id.
    Uid(u32),
    /// A Windows security identifier in its string form (`S-1-5-21-...`).
    Sid(String),
}

/// A Windows mandatory integrity level, ordered.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum Integrity {
    Untrusted,
    Low,
    Medium,
    High,
    System,
}

impl Integrity {
    /// The level a mandatory-label RID names; a RID between two levels counts as the level below it.
    pub(crate) fn from_rid(rid: u32) -> Self {
        match rid {
            0..=0x0FFF => Integrity::Untrusted,
            0x1000..=0x1FFF => Integrity::Low,
            0x2000..=0x2FFF => Integrity::Medium,
            0x3000..=0x3FFF => Integrity::High,
            _ => Integrity::System,
        }
    }
}

/// What the operating system says about a process on the other end of a connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerIdentity {
    pub(crate) principal: Principal,
    /// The peer's process id, where the platform reports it. For reporting; the rule does not rest on it, since a
    /// process id can be reused.
    pub(crate) pid: Option<u32>,
    /// Windows only: the peer's integrity level.
    pub(crate) integrity: Option<Integrity>,
    /// Windows only: the peer's token is an AppContainer token.
    pub(crate) app_container: bool,
}

/// Which way the check runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Relation {
    /// The listener checks the client it accepted: its integrity may not be below the listener's own.
    ListenerChecksClient,
    /// The client checks the listener it reached: its integrity must be the client's own.
    ClientChecksListener,
}

/// Why a peer was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PeerRefusal {
    /// The peer is another user.
    OtherPrincipal,
    /// The peer runs below (listener's view) or other than (client's view) this process's integrity.
    Integrity,
    /// The peer's token is an AppContainer token.
    AppContainer,
    /// The platform did not report what the rule needs; a check that cannot be made refuses.
    Unreadable(&'static str),
}

impl fmt::Display for PeerRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeerRefusal::OtherPrincipal => f.write_str("the peer is another user"),
            PeerRefusal::Integrity => f.write_str("the peer's integrity level is not acceptable"),
            PeerRefusal::AppContainer => f.write_str("the peer runs in an AppContainer"),
            PeerRefusal::Unreadable(what) => write!(f, "the peer check could not read {what}"),
        }
    }
}

/// The rule: who may be on the other end. Built once from this process's own identity.
#[derive(Clone, Debug)]
pub(crate) struct PeerPolicy {
    // Visible to the carrier's own tests, which build a policy for a user the host does not have; production code
    // reaches a policy only through `current_user`.
    pub(in super::super) own: PeerIdentity,
}

impl PeerPolicy {
    /// The policy of this process: the same user, from outside any AppContainer, at the integrity this process has.
    pub(crate) fn current_user() -> Result<Self, LocalRefusal> {
        super::own_identity().map(|own| Self { own })
    }

    pub(crate) fn own(&self) -> &PeerIdentity {
        &self.own
    }

    /// Accepts `observed` or says why not. A pure function of the two identities.
    pub(crate) fn verify(
        &self,
        observed: &PeerIdentity,
        relation: Relation,
    ) -> Result<(), PeerRefusal> {
        if observed.principal != self.own.principal {
            return Err(PeerRefusal::OtherPrincipal);
        }
        if observed.app_container {
            return Err(PeerRefusal::AppContainer);
        }
        match (self.own.integrity, observed.integrity) {
            (None, None) => Ok(()),
            (Some(own), Some(theirs)) => {
                let acceptable = match relation {
                    Relation::ListenerChecksClient => theirs >= own,
                    Relation::ClientChecksListener => theirs == own,
                };
                acceptable.then_some(()).ok_or(PeerRefusal::Integrity)
            }
            // The platform reported an integrity for one side and not the other.
            _ => Err(PeerRefusal::Unreadable("an integrity level")),
        }
    }
}
