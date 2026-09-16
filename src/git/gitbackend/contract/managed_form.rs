//! Managed-object and managed-form vocabulary for the Git root publication.
//! These types name what the root repository's managed marker, lock and index
//! look like in each preservation form, and how one form moves to another.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitRootManagedObject {
    MarkerWorktree,
    LockWorktree,
    Index,
    MarkerParentDirectory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRootManagedIndexEntry {
    pub path: Vec<u8>,
    pub object_id: String,
    pub mode: u32,
    pub stage: u8,
    pub assume_valid: bool,
    pub skip_worktree: bool,
    pub intent_to_add: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitRootManagedIndexFact {
    Absent { path: Vec<u8> },
    Present(GitRootManagedIndexEntry),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRootManagedIndexForm {
    pub marker: GitRootManagedIndexFact,
    pub lock: GitRootManagedIndexFact,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRootManagedForm {
    pub marker: Option<GitCandidateFile>,
    pub lock: GitCandidateFile,
    pub index: GitRootManagedIndexForm,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitRootManagedFormName {
    AttachedClean,
    RestoreClean,
    Handoff,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRootManagedTransition {
    pub object: GitRootManagedObject,
    pub source: GitRootManagedFormName,
    pub goal: GitRootManagedFormName,
}
