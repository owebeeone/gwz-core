#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "later log-engine steps consume the S2.1 cursor API"
    )
)]

mod coalesce;
mod handler;
mod merge;
mod request;

pub(super) use handler::handle_log;
#[cfg(test)]
use request::CommitLogHistories;
use request::open_request_histories;

use std::path::Path;
use std::process::{Command, Stdio};
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use crate::artifact::ArtifactSourceKind;
use crate::model::ModelResult;

/// Internal identity for the repository carrying one raw entry or degradation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitLogTarget {
    pub member_id: String,
    pub member_path: String,
    pub source_kind: ArtifactSourceKind,
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
    RevisionUnresolved,
    SnapshotEntryMissing,
    LockEntryMissing,
    HistoryUnreadable,
}

/// A per-repository failure record. It is an event, never a whole-request error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitLogDegradation {
    pub target: CommitLogTarget,
    pub kind: CommitLogDegradationKind,
    pub operand: Option<String>,
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
        walk: WalkPlan,
    },
    Degraded(CommitLogDegradation),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct WalkPlan {
    pushes: Vec<git2::Oid>,
    hides: Vec<git2::Oid>,
}

/// One selected repository. [`Self::messages`] creates a streaming HEAD cursor.
pub struct RepositoryHistory {
    target: CommitLogTarget,
    pathspecs: Vec<String>,
    state: RepositoryState,
}

impl RepositoryHistory {
    pub fn target(&self) -> &CommitLogTarget {
        &self.target
    }

    #[cfg(test)]
    fn pathspecs(&self) -> &[String] {
        &self.pathspecs
    }

    pub fn messages(&self) -> RepositoryMessages<'_> {
        match &self.state {
            RepositoryState::Ready { repository, walk } => {
                RepositoryMessages::from_repository(&self.target, repository, walk, &self.pathspecs)
            }
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
    PathWalk {
        repository: &'repo git2::Repository,
        plan: WalkPlan,
        pathspecs: Vec<String>,
        skip: usize,
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
        plan: &WalkPlan,
        pathspecs: &'repo [String],
    ) -> Self {
        if !pathspecs.is_empty() {
            return Self::from_path_walk(target, repository, plan, pathspecs);
        }
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
        for oid in &plan.pushes {
            if let Err(error) = walk.push(*oid) {
                return Self::single(
                    target,
                    degradation(
                        target,
                        CommitLogDegradationKind::HistoryUnreadable,
                        format!(
                            "could not push {oid} onto history cursor: {}",
                            error.message()
                        ),
                    ),
                );
            }
        }
        for oid in &plan.hides {
            if let Err(error) = walk.hide(*oid) {
                return Self::single(
                    target,
                    degradation(
                        target,
                        CommitLogDegradationKind::HistoryUnreadable,
                        format!(
                            "could not hide {oid} from history cursor: {}",
                            error.message()
                        ),
                    ),
                );
            }
        }

        // Deliberately do not call `set_sorting`: libgit2's default revwalk is
        // the repository-local default order required by `git log` parity.
        Self {
            target,
            state: MessagesState::Walk { repository, walk },
        }
    }

    fn from_path_walk(
        target: &'repo CommitLogTarget,
        repository: &'repo git2::Repository,
        plan: &WalkPlan,
        pathspecs: &[String],
    ) -> Self {
        Self {
            target,
            state: MessagesState::PathWalk {
                repository,
                plan: plan.clone(),
                pathspecs: pathspecs.to_vec(),
                skip: 0,
            },
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
            MessagesState::PathWalk {
                repository,
                plan,
                pathspecs,
                skip,
            } => {
                #[cfg(test)]
                let _reader_probe = path_reader_probe_enter(self.target);
                let output = Command::new("git")
                    .arg("--git-dir")
                    .arg(repository.path())
                    .arg("rev-list")
                    .arg("--max-count=1")
                    .arg(format!("--skip={skip}"))
                    .args(plan.pushes.iter().map(ToString::to_string))
                    .args(plan.hides.iter().map(|oid| format!("^{oid}")))
                    .arg("--")
                    .args(pathspecs.iter())
                    .env("GIT_OPTIONAL_LOCKS", "0")
                    .env("GIT_NO_LAZY_FETCH", "1")
                    .stderr(Stdio::null())
                    .output();
                match output {
                    Err(error) => {
                        self.fail(format!("could not run local path history cursor: {error}"))
                    }
                    Ok(output) if !output.status.success() => self.fail(format!(
                        "local path history cursor exited with {}",
                        output.status
                    )),
                    Ok(output) if output.stdout.is_empty() => {
                        self.state = MessagesState::Done;
                        None
                    }
                    Ok(output) => {
                        let line = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
                        match std::str::from_utf8(line)
                            .ok()
                            .and_then(|line| git2::Oid::from_str(line.trim()).ok())
                        {
                            Some(oid) => match repository.find_commit(oid) {
                                Ok(commit) => {
                                    *skip += 1;
                                    Some(CommitLogEvent::Entry(entry(self.target, &commit)))
                                }
                                Err(error) => self.fail(format!(
                                    "could not read path history commit {oid}: {}",
                                    error.message()
                                )),
                            },
                            None => self.fail(
                                "local path history cursor returned an invalid object id"
                                    .to_owned(),
                            ),
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[derive(Default)]
struct PathReaderProbeState {
    watched_prefix: Option<String>,
    active: usize,
    max_active: usize,
}

#[cfg(test)]
fn path_reader_probe() -> &'static Mutex<PathReaderProbeState> {
    static PROBE: OnceLock<Mutex<PathReaderProbeState>> = OnceLock::new();
    PROBE.get_or_init(|| Mutex::new(PathReaderProbeState::default()))
}

#[cfg(test)]
fn path_reader_probe_enter(target: &CommitLogTarget) -> PathReaderProbeGuard {
    let mut state = path_reader_probe().lock().unwrap();
    let watched = state
        .watched_prefix
        .as_ref()
        .is_some_and(|prefix| target.member_id.starts_with(prefix));
    if watched {
        state.active += 1;
        state.max_active = state.max_active.max(state.active);
    }
    PathReaderProbeGuard { watched }
}

#[cfg(test)]
struct PathReaderProbeGuard {
    watched: bool,
}

#[cfg(test)]
impl Drop for PathReaderProbeGuard {
    fn drop(&mut self) {
        if self.watched {
            path_reader_probe().lock().unwrap().active -= 1;
        }
    }
}

#[cfg(test)]
fn watch_path_readers(prefix: &str) {
    let mut state = path_reader_probe().lock().unwrap();
    assert_eq!(state.active, 0);
    state.watched_prefix = Some(prefix.to_owned());
    state.max_active = 0;
}

#[cfg(test)]
fn finish_watching_path_readers() -> usize {
    let mut state = path_reader_probe().lock().unwrap();
    assert_eq!(state.active, 0);
    state.watched_prefix = None;
    state.max_active
}

impl std::iter::FusedIterator for RepositoryMessages<'_> {}

/// Open the no-operand, default selection: `@root` plus every active member.
///
/// Reading the manifest is the only request-level fallible operation. Each
/// repository thereafter either exposes its own HEAD cursor or a degradation
/// event. This path performs no integrity gate, transport operation, or lock.
pub fn open_default_head_histories(workspace_root: &Path) -> ModelResult<Vec<RepositoryHistory>> {
    let request = crate::LogRequest::default();
    Ok(open_request_histories(workspace_root, &request)?.into_histories())
}

fn degradation(
    target: &CommitLogTarget,
    kind: CommitLogDegradationKind,
    detail: impl Into<String>,
) -> CommitLogEvent {
    CommitLogEvent::Degradation(CommitLogDegradation {
        target: target.clone(),
        kind,
        operand: None,
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
mod merge_tests;
#[cfg(test)]
mod tests;
