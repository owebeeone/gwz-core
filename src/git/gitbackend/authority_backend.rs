//! Compiler-sealed backend admitted to v1 merge authority and execution.

#![forbid(clippy::disallowed_methods)]

mod sealed {
    pub trait Sealed {}
}

/// Production backend permitted to supply v1 merge authority facts and
/// execute their bound physical actions.
///
/// `GitBackend` remains open for ordinary operations and downstream test
/// doubles. This narrower interface is sealed because v1 authority relies on
/// the reviewed `Git2Backend` observation and mutation semantics as one unit.
#[allow(private_bounds)]
pub trait MergeAuthorityBackend: super::contract::GitBackend + sealed::Sealed {}

impl sealed::Sealed for super::backend::Git2Backend {}
impl MergeAuthorityBackend for super::backend::Git2Backend {}
