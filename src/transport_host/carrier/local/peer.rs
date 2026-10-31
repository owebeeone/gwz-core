//! Who is on the other end of a local carrier, and the one rule that decides whether to talk to them.
//!
//! The platform modules read an operating-system identity ([`PeerIdentity`]) from a connected socket or pipe; the rule
//! here ([`PeerPolicy::verify`]) is a pure function of two identities, so its refusals are tested on every platform
//! with identities no single-user host can produce. There is one rule and one way to a policy: [`PeerPolicy::current_user`]
//! reads this process's own identity. No setting, argument or build variant changes what it accepts (a source check,
//! `scripts/checks/check_carrier_peer_rule.py`, fails the build on a hook that would, and on a field of the policy
//! written outside this file). The one other constructor exists in test builds only, in this file, for the rows that need
//! a policy for a user the host does not have.
use super::{
    LocalRefusal,
    sandbox::{self, Cause, SandboxFindings},
};
use std::fmt;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl PeerPolicy {
            /// A policy for an identity the host may not have: the rows' way to a user or an integrity level that no
            /// single-user host can produce. Test builds only.
            pub(crate) fn from_identity(own: PeerIdentity) -> Self {
                Self { own }
            }

            /// [`current_user`](Self::current_user)'s rule applied to an identity given: the rows' way to an own
            /// process that is sandboxed. Test builds only.
            pub(crate) fn try_from_identity(own: PeerIdentity) -> Result<Self, LocalRefusal> {
                Self::from_own(own)
            }
        }
    }
}

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
    /// What the sandbox probes found (`sandbox.rs`); nothing found on Windows, whose token flags are above.
    pub(crate) sandbox: SandboxFindings,
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
    /// The peer is sandboxed in a way the platform's probe sees, or is in a namespace other than this process's.
    Sandboxed(Cause),
    /// The platform did not report what the rule needs; a check that cannot be made refuses.
    Unreadable(&'static str),
}

impl fmt::Display for PeerRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeerRefusal::OtherPrincipal => f.write_str("the peer is another user"),
            PeerRefusal::Integrity => f.write_str("the peer's integrity level is not acceptable"),
            PeerRefusal::AppContainer => f.write_str("the peer runs in an AppContainer"),
            PeerRefusal::Sandboxed(cause) => write!(f, "the peer is sandboxed by {cause}"),
            PeerRefusal::Unreadable(what) => write!(f, "the peer check could not read {what}"),
        }
    }
}

impl PeerRefusal {
    /// The refusal as said about the caller's own process, which has no peer: the same cause, with this process as the
    /// subject (parked server design §4, "Sandboxed callers").
    pub(crate) fn about_caller(&self) -> String {
        match self {
            PeerRefusal::OtherPrincipal => "this process is another user".to_string(),
            PeerRefusal::Integrity => "this process runs below Medium integrity".to_string(),
            PeerRefusal::AppContainer => "this process runs in an AppContainer".to_string(),
            PeerRefusal::Sandboxed(cause) => format!("this process is sandboxed by {cause}"),
            PeerRefusal::Unreadable(what) => {
                format!("this process could not be checked: it could not read {what}")
            }
        }
    }
}

/// The rule: who may be on the other end. Built once from this process's own identity.
#[derive(Clone, Debug)]
pub(crate) struct PeerPolicy {
    own: PeerIdentity,
}

impl PeerPolicy {
    /// The policy of this process: the same user, from outside any AppContainer, at the integrity this process has. The
    /// caller is held to the rule it applies to its peer: a process below Medium integrity, or one the sandbox probes
    /// find sandboxed, is itself sandboxed (parked server design §4, "Sandboxed callers"), and gets no policy. A probe
    /// that cannot complete refuses.
    pub(crate) fn current_user() -> Result<Self, LocalRefusal> {
        let mut own = super::own_identity()?;
        own.sandbox = sandbox::of_self().map_err(LocalRefusal::Own)?;
        Self::from_own(own)
    }

    /// The caller's side of the rule, a function of the identity read: a process that is itself sandboxed gets no policy.
    fn from_own(own: PeerIdentity) -> Result<Self, LocalRefusal> {
        if own.app_container {
            return Err(LocalRefusal::Own(PeerRefusal::AppContainer));
        }
        if own.integrity.is_some_and(|level| level < Integrity::Medium) {
            return Err(LocalRefusal::Own(PeerRefusal::Integrity));
        }
        if let Some(cause) = own.sandbox.cause() {
            return Err(LocalRefusal::Own(PeerRefusal::Sandboxed(cause)));
        }
        Ok(Self { own })
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
        // Absolute: whatever sandbox this side is in, and whatever the other side's. Then the namespaces must be shared.
        if let Some(cause) = observed.sandbox.cause().or(self.own.sandbox.cause()) {
            return Err(PeerRefusal::Sandboxed(cause));
        }
        if !observed.sandbox.same_namespaces(&self.own.sandbox) {
            return Err(PeerRefusal::Sandboxed(Cause::OtherNamespace));
        }
        match (self.own.integrity, observed.integrity) {
            (None, None) => Ok(()),
            (Some(own), Some(theirs)) => {
                // A process below Medium integrity is sandboxed, whatever the other side's level: no two of them
                // reach each other through a carrier.
                if own < Integrity::Medium || theirs < Integrity::Medium {
                    return Err(PeerRefusal::Integrity);
                }
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
