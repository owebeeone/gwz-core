//! Fixture-only values. These are not part of the production API or protocol.
use super::*;

#[derive(Clone, Debug)]
pub struct TestRepoSpec {
    pub branch: String,
    pub bare: bool,
    pub sha256: bool,
}
impl Default for TestRepoSpec {
    fn default() -> Self {
        Self {
            branch: "main".into(),
            bare: false,
            sha256: false,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TestRefTarget {
    Direct(String),
    Symbolic(String),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TestHead {
    Attached(String),
    Detached(String),
}
#[derive(Clone, Debug)]
pub enum TestCommitTree {
    Index,
    Files(Vec<GitCandidateFile>),
}
#[derive(Clone, Debug)]
pub struct TestCommitSpec {
    pub tree: TestCommitTree,
    pub parents: Vec<String>,
    pub message: String,
    pub author: GitPreparedSignature,
    pub committer: GitPreparedSignature,
}
impl TestCommitSpec {
    pub fn from_index(message: impl Into<String>, parents: Vec<String>) -> Self {
        let identity = GitPreparedSignature {
            name: "GWZ Test".into(),
            email: "gwz@example.invalid".into(),
            time_seconds: 1_000_000_000,
            timezone_offset_minutes: 0,
        };
        Self {
            tree: TestCommitTree::Index,
            parents,
            message: message.into(),
            author: identity.clone(),
            committer: identity,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestCommit {
    pub tree: String,
    pub parents: Vec<String>,
    pub message: String,
    pub author: GitPreparedSignature,
    pub committer: GitPreparedSignature,
}
pub type TestIndexEntry = GitRootManagedIndexEntry;

#[derive(Clone, Debug)]
pub struct TestCommitFileEdit {
    pub path: String,
    pub bytes: Option<Vec<u8>>,
}
