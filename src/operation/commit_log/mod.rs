#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "later log-engine steps consume the S2.1 cursor API"
    )
)]

mod coalesce;
mod handler;

pub(super) use handler::handle_log;

use std::path::Path;

use crate::artifact::{self, ArtifactSourceKind};
use crate::model::ModelResult;
use crate::workspace_ops::{
    CommandDefaultTargets, RootSelectionPolicy, SelectedTarget, resolve_targets,
};

/// Internal identity for the repository carrying one raw entry or degradation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitLogTarget {
    pub member_id: String,
    pub member_path: String,
}

/// A Git signature timestamp, preserving the recorded timezone offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitLogTime {
    pub seconds: i64,
    pub offset_minutes: i32,
}

/// Raw Git identity bytes are retained for later byte-exact coalescing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitLogIdentity {
    pub name: Vec<u8>,
    pub email: Vec<u8>,
    pub time: CommitLogTime,
}

/// One repository's uncoalesced commit entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitLogEntry {
    pub target: CommitLogTarget,
    pub commit_id: String,
    pub parent_ids: Vec<String>,
    pub author: CommitLogIdentity,
    pub committer: CommitLogIdentity,
    pub message: Vec<u8>,
    pub message_encoding: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitLogDegradationKind {
    UnsupportedSourceKind,
    RepositoryUnreadable,
    UnbornHead,
    HistoryUnreadable,
}

/// A per-repository failure record. It is an event, never a whole-request error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitLogDegradation {
    pub target: CommitLogTarget,
    pub kind: CommitLogDegradationKind,
    pub detail: String,
}

/// The internal message boundary consumed by later merge and protocol steps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitLogEvent {
    Entry(CommitLogEntry),
    Degradation(CommitLogDegradation),
}

enum RepositoryState {
    Ready {
        repository: git2::Repository,
        head_id: git2::Oid,
    },
    Degraded(CommitLogDegradation),
}

/// One selected repository. [`Self::messages`] creates a streaming HEAD cursor.
pub struct RepositoryHistory {
    target: CommitLogTarget,
    state: RepositoryState,
}

impl RepositoryHistory {
    pub fn target(&self) -> &CommitLogTarget {
        &self.target
    }

    pub fn messages(&self) -> RepositoryMessages<'_> {
        match &self.state {
            RepositoryState::Ready {
                repository,
                head_id,
            } => RepositoryMessages::from_repository(&self.target, repository, *head_id),
            RepositoryState::Degraded(record) => RepositoryMessages::single(
                &self.target,
                CommitLogEvent::Degradation(record.clone()),
            ),
        }
    }
}

enum MessagesState<'repo> {
    Walk {
        repository: &'repo git2::Repository,
        walk: git2::Revwalk<'repo>,
    },
    Single(Option<Box<CommitLogEvent>>),
    Done,
}

/// A newest-first, per-repository cursor in native `git log` default order.
pub struct RepositoryMessages<'repo> {
    target: &'repo CommitLogTarget,
    state: MessagesState<'repo>,
}

impl<'repo> RepositoryMessages<'repo> {
    fn from_repository(
        target: &'repo CommitLogTarget,
        repository: &'repo git2::Repository,
        head_id: git2::Oid,
    ) -> Self {
        let mut walk = match repository.revwalk() {
            Ok(walk) => walk,
            Err(error) => {
                return Self::single(
                    target,
                    degradation(
                        target,
                        CommitLogDegradationKind::HistoryUnreadable,
                        format!("could not create history cursor: {}", error.message()),
                    ),
                );
            }
        };
        if let Err(error) = walk.push(head_id) {
            return Self::single(
                target,
                degradation(
                    target,
                    CommitLogDegradationKind::HistoryUnreadable,
                    format!("could not start history cursor: {}", error.message()),
                ),
            );
        }

        // Deliberately do not call `set_sorting`: libgit2's default revwalk is
        // the repository-local default order required by `git log` parity.
        Self {
            target,
            state: MessagesState::Walk { repository, walk },
        }
    }

    fn single(target: &'repo CommitLogTarget, event: CommitLogEvent) -> Self {
        Self {
            target,
            state: MessagesState::Single(Some(Box::new(event))),
        }
    }

    fn fail(&mut self, detail: String) -> Option<CommitLogEvent> {
        self.state = MessagesState::Done;
        Some(degradation(
            self.target,
            CommitLogDegradationKind::HistoryUnreadable,
            detail,
        ))
    }
}

impl Iterator for RepositoryMessages<'_> {
    type Item = CommitLogEvent;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.state {
            MessagesState::Single(event) => event.take().map(|event| *event),
            MessagesState::Done => None,
            MessagesState::Walk { repository, walk } => match walk.next() {
                Some(Ok(oid)) => match repository.find_commit(oid) {
                    Ok(commit) => Some(CommitLogEvent::Entry(entry(self.target, &commit))),
                    Err(error) => {
                        self.fail(format!("could not read commit {oid}: {}", error.message()))
                    }
                },
                Some(Err(error)) => self.fail(format!("history walk failed: {}", error.message())),
                None => {
                    self.state = MessagesState::Done;
                    None
                }
            },
        }
    }
}

impl std::iter::FusedIterator for RepositoryMessages<'_> {}

/// Open the no-operand, default selection: `@root` plus every active member.
///
/// Reading the manifest is the only request-level fallible operation. Each
/// repository thereafter either exposes its own HEAD cursor or a degradation
/// event. This path performs no integrity gate, transport operation, or lock.
pub fn open_default_head_histories(workspace_root: &Path) -> ModelResult<Vec<RepositoryHistory>> {
    let manifest = artifact::read_manifest(workspace_root)?;
    let selected = resolve_targets(
        &manifest,
        None,
        CommandDefaultTargets::All,
        RootSelectionPolicy::Allow,
    )?;

    Ok(selected
        .into_iter()
        .map(|selected| match selected {
            SelectedTarget::Root => open_history(
                CommitLogTarget {
                    member_id: "@root".to_owned(),
                    member_path: ".".to_owned(),
                },
                workspace_root,
                ArtifactSourceKind::Git,
            ),
            SelectedTarget::Member(member) => open_history(
                CommitLogTarget {
                    member_id: member.id.clone(),
                    member_path: member.path.clone(),
                },
                &workspace_root.join(&member.path),
                member.source_kind,
            ),
        })
        .collect())
}

fn open_history(
    target: CommitLogTarget,
    path: &Path,
    source_kind: ArtifactSourceKind,
) -> RepositoryHistory {
    let state = if source_kind != ArtifactSourceKind::Git {
        RepositoryState::Degraded(CommitLogDegradation {
            target: target.clone(),
            kind: CommitLogDegradationKind::UnsupportedSourceKind,
            detail: format!("commit history does not support {source_kind:?} members"),
        })
    } else {
        match git2::Repository::open(path) {
            Ok(repository) => match repository
                .head()
                .and_then(|head| head.peel_to_commit())
                .map(|commit| commit.id())
            {
                Ok(head_id) => RepositoryState::Ready {
                    repository,
                    head_id,
                },
                Err(error) if error.code() == git2::ErrorCode::UnbornBranch => {
                    RepositoryState::Degraded(CommitLogDegradation {
                        target: target.clone(),
                        kind: CommitLogDegradationKind::UnbornHead,
                        detail: "repository HEAD is unborn".to_owned(),
                    })
                }
                Err(error) => RepositoryState::Degraded(CommitLogDegradation {
                    target: target.clone(),
                    kind: CommitLogDegradationKind::HistoryUnreadable,
                    detail: format!("could not resolve HEAD locally: {}", error.message()),
                }),
            },
            Err(error) => RepositoryState::Degraded(CommitLogDegradation {
                target: target.clone(),
                kind: CommitLogDegradationKind::RepositoryUnreadable,
                detail: format!("could not open repository: {}", error.message()),
            }),
        }
    };
    RepositoryHistory { target, state }
}

fn degradation(
    target: &CommitLogTarget,
    kind: CommitLogDegradationKind,
    detail: impl Into<String>,
) -> CommitLogEvent {
    CommitLogEvent::Degradation(CommitLogDegradation {
        target: target.clone(),
        kind,
        detail: detail.into(),
    })
}

fn entry(target: &CommitLogTarget, commit: &git2::Commit<'_>) -> CommitLogEntry {
    CommitLogEntry {
        target: target.clone(),
        commit_id: commit.id().to_string(),
        parent_ids: commit.parent_ids().map(|oid| oid.to_string()).collect(),
        author: identity(commit.author()),
        committer: identity(commit.committer()),
        message: commit.message_raw_bytes().to_vec(),
        message_encoding: encoding_header(commit.raw_header_bytes()),
    }
}

fn encoding_header(raw_header: &[u8]) -> Option<Vec<u8>> {
    // `Commit::raw_header_bytes` is infallible once libgit2 has produced the
    // commit, so absence here means the byte-exact header is genuinely absent.
    raw_header
        .split(|byte| *byte == b'\n')
        .find_map(|line| line.strip_prefix(b"encoding ").map(<[u8]>::to_vec))
}

fn identity(signature: git2::Signature<'_>) -> CommitLogIdentity {
    let when = signature.when();
    CommitLogIdentity {
        name: signature.name_bytes().to_vec(),
        email: signature.email_bytes().to_vec(),
        time: CommitLogTime {
            seconds: when.seconds(),
            offset_minutes: when.offset_minutes(),
        },
    }
}

#[cfg(test)]
mod coalesce_tests;
#[cfg(test)]
mod tests;
