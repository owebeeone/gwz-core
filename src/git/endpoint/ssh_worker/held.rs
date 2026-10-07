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
    Failed(OpenRequest, io::Error),
}
impl Unserved {
    pub(super) fn settle(self, pool: &Pool, pending: &mut Vec<Pending>) {
        match self {
            Unserved::Dead(request, mut policy) => {
                policy.fresh = true;
                match pool.checkout_until(policy.clone(), request.deadline) {
                    Ok(checkout) => pending.push(Pending {
                        checkout,
                        request,
                        policy,
                    }),
                    Err(error) => request.complete(Err(io::Error::other(error))),
                }
            }
            Unserved::Failed(request, error) => request.complete(Err(error)),
        }
    }
}
