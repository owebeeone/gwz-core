//! An open whose reply waits for its channel, and an open its lease did not
//! serve (dev-docs/GwzTransportIdleLossDesign.md §6.2).
use super::*;

/// The reply of an open whose channel is not open yet. Git sends nothing
/// before it has the reply, so until then the exchange has written no Git byte.
pub(super) struct Held {
    pub(super) request: OpenRequest,
    pub(super) policy: gwz_transport::pool::Request,
    pub(super) reused: bool,
    pub(super) reply: (EndpointAttachment, Opened),
}
impl Held {
    pub(super) fn reply(self) {
        self.request.complete(Ok(self.reply));
    }
    /// Its open was cancelled before the reply. Its allocation is over, and
    /// its stream's I/O deadline bounds the wait for the channel.
    pub(super) fn cancelled(&self) -> bool {
        self.request.cancelled.load(Ordering::Acquire)
    }
    pub(super) fn cancel(self) {
        self.request
            .complete(Err(io::ErrorKind::ConnectionAborted.into()));
    }
    /// The exchange ended before its channel opened: `failed` when the
    /// channel failed, otherwise its stream ended (its I/O deadline).
    pub(super) fn unserved(self, failed: bool) -> Unserved {
        if failed && self.reused {
            Unserved::Dead(self.request, self.policy)
        } else {
            let kind = if failed {
                io::ErrorKind::ConnectionReset
            } else {
                io::ErrorKind::TimedOut
            };
            Unserved::Failed(self.request, kind.into())
        }
    }
}

/// An open its lease did not serve.
pub(super) enum Unserved {
    /// A reused connection died before any Git byte: the open is retried once,
    /// on a fresh connection. A fresh request never gets a reused one, so the
    /// retry cannot be Dead again.
    Dead(OpenRequest, gwz_transport::pool::Request),
    /// A connection another binding last used, which this open's binding could not prove (reuse design §4): the open
    /// asks again, within its own deadlines, with that connection excluded.
    Revoked(OpenRequest, gwz_transport::pool::Request),
    /// This open's binding's proof of a connection failed (the key was refused or the host key is no longer trusted),
    /// so the connection and its idle siblings are retired: the open asks again for a new connection, with no
    /// exclusion, and takes no idle one, whether or not the siblings' retirement has been seen yet (reuse design §4's
    /// table; step 1.6 State N-P3-1).
    Denied(OpenRequest, gwz_transport::pool::Request),
    Failed(OpenRequest, io::Error),
}
impl Unserved {
    pub(super) fn settle(self, pool: &Pool, pending: &mut Vec<Pending>) {
        let (request, policy) = match self {
            // A dead connection is replaced by a new one.
            Unserved::Dead(request, mut policy) => {
                policy.fresh = true;
                policy.exclude.clear();
                (request, policy)
            }
            // A connection the binding could not prove is excluded (step 1.6 remediation, S-P2-2), so the binding's
            // own idle connection, or a new one, is offered next; past the pool's exclusion bound the open asks for a
            // new connection.
            Unserved::Revoked(request, mut policy) => {
                if policy.exclude.len() > gwz_transport::pool::MAX_EXCLUDED {
                    policy.fresh = true;
                    policy.exclude.clear();
                }
                (request, policy)
            }
            Unserved::Denied(request, policy) => (request, denied(policy)),
            Unserved::Failed(request, error) => {
                request.complete(Err(error));
                return;
            }
        };
        match pool.checkout_until(policy.clone(), request.deadline) {
            Ok(checkout) => pending.push(Pending {
                checkout,
                request,
                policy,
            }),
            Err(error) => request.complete(Err(io::Error::other(error))),
        }
    }
}

/// `policy` asking again after its binding's proof was denied: for a new connection, with no exclusion.
pub(super) fn denied(mut policy: gwz_transport::pool::Request) -> gwz_transport::pool::Request {
    policy.fresh = true;
    policy.exclude.clear();
    policy
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[test]
        fn a_denied_proofs_retry_asks_for_a_new_connection() {
            use gwz_transport::pool::{Identity, Key, Owner, Request};
            let policy = Request::new(
                Key::ssh("git", "example.invalid", 22),
                Identity::Ambient,
                Owner::new("session", "operation"),
            );
            assert!(!policy.fresh);
            let retry = denied(policy);
            assert!(retry.fresh && retry.exclude.is_empty());
        }
    }
}
