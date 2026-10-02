//! Shared stateful Git substitute. No native repositories or Git processes.
//! Operations not implemented here return UnsupportedOperation, never pretend success.
use super::*;
use crate::filesystem::{FileSystem, FsKind, make_filesystem};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
mod fixture;
mod root;
mod state;
mod worktree_io;

pub(crate) use state::*;
use worktree_io::*;

#[macro_use]
mod methods_fixtures;
#[macro_use]
mod methods_merge;
#[macro_use]
mod methods_root;
#[macro_use]
mod methods_repository;
#[macro_use]
mod methods_refs;
#[macro_use]
mod methods_worktree;
#[macro_use]
mod methods_backup;
#[macro_use]
mod methods_stash;

#[allow(unused_variables)]
impl GitRepository for FakeGitRepository {
    fake_repository_fixtures!();
    fake_repository_merge!();
    fake_repository_root!();
    fake_repository_repository!();
    fake_repository_refs!();
    fake_repository_worktree!();
    fake_repository_backup!();
    fake_repository_stash!();
}
