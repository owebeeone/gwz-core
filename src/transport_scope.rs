//! The transport scope: which requests run inside a transport runtime. It is
//! one predicate, which both drivers call: gwz-cli's dispatch and gwz-py's
//! native entry (gwz-py `dev-docs/GwzPyPerOperationTransportDesign.md` §2.1;
//! `dev-docs/GwzTransportReleasePlanAmendment-2.md` §3.17, S6.2).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | `Operation` names exactly the operations whose gwz-core handler scopes a transport with `with_transport`, by their request types; a source test pins the two equal | design §2.1 |
//! | Each operation is named by its protocol method too, so a driver that dispatches by method name, as gwz-py's extension does, asks the same predicate | design §2.1, §2.3 |
//! | A tag runs in scope only when it reaches a remote: when it pushes or fetches tags, or lists or deletes them on a remote | TR2.11's `transport_meta` |
//!
//! Since TR2.11 a backend without a host context takes libgit2's native
//! route, so an operation missing here would leave the transport silently,
//! and an extra one would build a runtime for an operation that opens no
//! connection.

use crate::{TagOp, TagRequest};

/// An operation whose gwz-core handler scopes a transport with
/// `with_transport`, named as its request type is: `Fetch` carries a
/// `FetchRequest`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Operation {
    CloneRepoMember,
    CloneWorkspace,
    Fetch,
    InitFromSources,
    Materialize,
    PullHead,
    PullSnapshot,
    Push,
    /// In scope only when the tag reaches a remote (`in_scope`).
    Tag,
}

impl Operation {
    /// Every operation whose handler scopes a transport.
    pub const ALL: [Self; 9] = [
        Self::CloneRepoMember,
        Self::CloneWorkspace,
        Self::Fetch,
        Self::InitFromSources,
        Self::Materialize,
        Self::PullHead,
        Self::PullSnapshot,
        Self::Push,
        Self::Tag,
    ];

    /// The protocol method that carries the operation's request.
    pub const fn method(self) -> &'static str {
        match self {
            Self::CloneRepoMember => "clone_repo_member",
            Self::CloneWorkspace => "clone_workspace",
            Self::Fetch => "fetch",
            Self::InitFromSources => "init_from_sources",
            Self::Materialize => "materialize",
            Self::PullHead => "pull_head",
            Self::PullSnapshot => "pull_snapshot",
            Self::Push => "push",
            Self::Tag => "tag",
        }
    }

    /// The operation that protocol method `method` carries, when it is one
    /// whose handler scopes a transport.
    pub fn from_method(method: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|operation| operation.method() == method)
    }
}

/// Whether a request of `operation` runs inside a transport runtime. Every
/// operation's request does except a tag's that reaches no remote; `tag` is
/// the tag's request, and without it `Operation::Tag` is out of scope.
pub fn in_scope(operation: Operation, tag: Option<&TagRequest>) -> bool {
    match operation {
        Operation::Tag => tag.is_some_and(reaches_remote),
        _ => true,
    }
}

/// Whether a tag request reaches a remote: it pushes or fetches tags, or
/// lists or deletes them on a remote.
fn reaches_remote(request: &TagRequest) -> bool {
    matches!(request.op, TagOp::Push | TagOp::Fetch)
        || (matches!(request.op, TagOp::List | TagOp::Delete) && request.remote.is_some())
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[path = "transport_scope_tests.rs"]
        mod tests;
    }
}
