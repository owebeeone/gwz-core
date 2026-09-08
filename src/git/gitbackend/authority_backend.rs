//! Compiler-sealed backend admitted to v1 merge authority and execution.

#![forbid(clippy::disallowed_methods)]

mod sealed {
    pub(crate) trait Sealed {
        fn operation_services(&self) -> crate::operation_context::OperationServices;
    }
}

/// Production backend permitted to supply v1 merge authority facts and
/// execute their bound physical actions.
///
/// `GitBackend` remains open for ordinary operations and downstream test
/// doubles. This narrower interface is sealed because v1 authority relies on
/// the reviewed `Git2Backend` observation and mutation semantics as one unit.
#[allow(private_bounds)]
pub trait MergeAuthorityBackend: super::contract::GitBackend + sealed::Sealed {}

impl sealed::Sealed for super::backend::Git2Backend {
    fn operation_services(&self) -> crate::operation_context::OperationServices {
        crate::operation_context::OperationServices::from_services(
            self.filesystem.clone(),
            std::sync::Arc::new(self.clone()),
        )
    }
}
impl MergeAuthorityBackend for super::backend::Git2Backend {}

#[cfg(test)]
impl sealed::Sealed for super::factory::GitTestRepository {
    fn operation_services(&self) -> crate::operation_context::OperationServices {
        match self {
            Self::Real(repository) => repository.operation_services(),
            Self::Fake(repository) => crate::operation_context::OperationServices::from_services(
                repository.filesystem.clone(),
                std::sync::Arc::new((**repository).clone()),
            ),
        }
    }
}
#[cfg(test)]
impl MergeAuthorityBackend for super::factory::GitTestRepository {}
