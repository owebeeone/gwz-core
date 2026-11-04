//! Test-only dispatch of `GitPullUpdate` for the backend the test world selects.

use super::super::factory::GitTestRepository;
use super::*;

impl GitPullUpdateSealed for GitTestRepository {}

impl GitPullUpdate for GitTestRepository {
    fn update_collisions(&self, path: &Path, target: &str) -> ModelResult<Vec<GitUpdateCollision>> {
        match self {
            Self::Real(repo) => repo.update_collisions(path, target),
            Self::Fake(repo) => repo.update_collisions(path, target),
        }
    }

    fn replay_collisions(
        &self,
        path: &Path,
        onto: &str,
        local: &str,
    ) -> ModelResult<Vec<GitUpdateCollision>> {
        match self {
            Self::Real(repo) => repo.replay_collisions(path, onto, local),
            Self::Fake(repo) => repo.replay_collisions(path, onto, local),
        }
    }

    fn prepare_pull_update_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
    ) -> ModelResult<GitPreparedMerge> {
        match self {
            Self::Real(repo) => {
                repo.prepare_pull_update_checked(path, branch, expected_before, source_commit)
            }
            Self::Fake(repo) => {
                repo.prepare_pull_update_checked(path, branch, expected_before, source_commit)
            }
        }
    }

    fn validate_prepared_pull_update(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<()> {
        match self {
            Self::Real(repo) => repo.validate_prepared_pull_update(
                path,
                branch,
                expected_before,
                source_commit,
                prepared,
            ),
            Self::Fake(repo) => repo.validate_prepared_pull_update(
                path,
                branch,
                expected_before,
                source_commit,
                prepared,
            ),
        }
    }

    fn execute_prepared_pull_update(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        message: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult> {
        match self {
            Self::Real(repo) => repo.execute_prepared_pull_update(
                path,
                branch,
                expected_before,
                source_commit,
                message,
                prepared,
            ),
            Self::Fake(repo) => repo.execute_prepared_pull_update(
                path,
                branch,
                expected_before,
                source_commit,
                message,
                prepared,
            ),
        }
    }
}
