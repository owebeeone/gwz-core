//! `commit`, `tag_create` and `tag_delete`: core's entry points into gwz-git's
//! native commit and tag (step 5.2 moved the implementation and its Git CLI
//! fallbacks there). Core supplies what gwz-git does not own: the captured
//! process environment, the route policy, the reader of the "why Git ran"
//! notes and the host's spawn gate for the CLI route's child.

use gwz_git::native_commit::{self, Services};

use super::*;
use crate::git::model_error;

fn services(backend: &Git2Backend) -> Services<'_> {
    Services::new(&backend.environment, &backend.spawn_gate)
        .with_policy(backend.route_policy)
        .with_notes(backend.route_notes.as_ref())
}

pub(super) fn commit(
    backend: &Git2Backend,
    path: &Path,
    message: &str,
    all: bool,
) -> ModelResult<GitCommitResult> {
    native_commit::commit(&services(backend), path, message, all)
        .map(|outcome| GitCommitResult {
            commit: outcome.id.to_string(),
        })
        .map_err(model_error)
}

pub(super) fn tag_create(
    backend: &Git2Backend,
    path: &Path,
    name: &str,
    message: Option<&str>,
    signed: bool,
) -> ModelResult<GitTagResult> {
    native_commit::tag_create(&services(backend), path, name, message, signed)
        .map(|outcome| GitTagResult {
            name: outcome.name,
            commit: outcome.commit.to_string(),
        })
        .map_err(model_error)
}

pub(super) fn tag_delete(backend: &Git2Backend, path: &Path, name: &str) -> ModelResult<()> {
    native_commit::tag_delete(&services(backend), path, name).map_err(model_error)
}
