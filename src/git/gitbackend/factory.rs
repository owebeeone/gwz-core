//! Repository construction. The production build contains no selector or fake.
use super::*;

/// Construct a repository backend. Production always constructs native Git.
#[cfg(not(test))]
pub fn make_repository() -> impl GitRepository {
    Git2Repository::new()
}

/// Test processes select one implementation for all factory calls and threads.
/// Set GWZ_TEST_GIT=real or fake before launching the test executable.
#[cfg(test)]
pub(crate) fn make_repository() -> GitTestRepository {
    test_repository::make_repository()
}

#[cfg(test)]
mod test_repository;

#[cfg(test)]
pub(crate) use test_repository::GitTestRepository;
