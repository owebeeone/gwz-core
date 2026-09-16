//! `gwz-workspace-install`: destination construction and installation
//! ordering (lane N).
//!
//! [`install`] composes a local clone destination (gwz-dev
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §8, design §4) in
//! the four ordered steps the design specifies, and the order is the
//! product:
//!
//! 1. **Admit.** Aggregate the name, path, source-layout and
//!    nested-repository checks through [`InstallPorts::snapshot_source`] and
//!    [`InstallPorts::observe_destination`], refusing an overlapping name or
//!    path, a nonempty destination, a destination that is already a
//!    workspace, an unsupported source layout (design §4.0), a verbatim copy
//!    of a source with an open gwz merge (§4.1) and a `-b <branch>` that
//!    already exists at freeze time (§4.2). Nothing is fetched and nothing
//!    is written.
//! 2. **Reserve.** Write the `creating` row through the live `FamilySession`
//!    and allocate the destination directory. The snapshot captured in step
//!    1 is the one in-memory freeze vector for the rest of the invocation.
//! 3. **Build.** Copy with exclusions applied during traversal (verbatim) or
//!    construct clean/bare repositories through the construction port,
//!    install the destination's own Git configuration and the fresh pointer
//!    and allocation marker through the store session, check the destination
//!    against design §4.1's completion column and §4.0's independence rules,
//!    recheck the source observations, then recapture the destination lock
//!    and desired branches.
//! 4. **Publish.** Write the final manifest **last**, then mark the row
//!    `ready`.
//!
//! It never writes family files itself, and an error or a cancellation
//! between steps leaves an incomplete row and a retained directory for
//! inspection: the only thing installation does after a failure is record a
//! diagnostic on the `creating` row. There is no cleanup, no rollback, no
//! resume and no promotion, even when the destination looks complete.

#![forbid(unsafe_code)]

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

mod admit;
mod destination;
mod install;
mod ports;
mod refusal;
mod report;
mod request;

#[cfg(test)]
mod tests;

pub use destination::*;
pub use install::*;
pub use ports::*;
pub use refusal::*;
pub use report::*;
pub use request::*;

pub(crate) use admit::*;
