#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "later log-engine steps consume the S2.1 cursor API"
    )
)]

mod aggregate;
mod coalesce;
mod filter;
mod handler;
mod merge;
mod request;

pub(super) use handler::handle_log;
pub use handler::{
    CommitLogOutputRegistry, CommitLogReadRequest, CommitLogReadResponse, CommitLogReadState,
};
#[cfg(test)]
use request::CommitLogHistories;
use request::open_request_histories;

use std::path::Path;
use std::process::{Command, Stdio};

use crate::artifact::ArtifactSourceKind;
use crate::git::spawn_gate::SpawnGate;
use crate::model::ModelResult;

use filter::CommitLogFilters;

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
        /// The spawn gate of the host this history's request runs under: the path cursor's `git rev-list` spawns
        /// under it (operator decision D25). Only a history that can walk has one.
        gate: SpawnGate,
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
    filters: CommitLogFilters,
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
            RepositoryState::Ready {
                repository,
                walk,
                gate,
            } => RepositoryMessages::from_repository(
                &self.target,
                repository,
                walk,
                gate,
                &self.pathspecs,
                &self.filters,
            ),
            RepositoryState::Degraded(record) => RepositoryMessages::single(
                &self.target,
                CommitLogEvent::Degradation(record.clone()),
                &self.filters,
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
        first_parent: bool,
        skip: usize,
        gate: SpawnGate,
    },
    Single(Option<Box<CommitLogEvent>>),
    Done,
}

/// A newest-first, per-repository cursor in native `git log` default order.
pub struct RepositoryMessages<'repo> {
    target: &'repo CommitLogTarget,
    filters: &'repo CommitLogFilters,
    state: MessagesState<'repo>,
}

impl<'repo> RepositoryMessages<'repo> {
    fn from_repository(
        target: &'repo CommitLogTarget,
        repository: &'repo git2::Repository,
        plan: &WalkPlan,
        gate: &SpawnGate,
        pathspecs: &'repo [String],
        filters: &'repo CommitLogFilters,
    ) -> Self {
        if !pathspecs.is_empty() {
            return Self::from_path_walk(target, repository, plan, gate, pathspecs, filters);
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
                    filters,
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
                    filters,
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
                    filters,
                );
            }
        }

        if filters.first_parent()
            && let Err(error) = walk.simplify_first_parent()
        {
            return Self::single(
                target,
                degradation(
                    target,
                    CommitLogDegradationKind::HistoryUnreadable,
                    format!(
                        "could not simplify history to first parents: {}",
                        error.message()
                    ),
                ),
                filters,
            );
        }

        // Deliberately do not call `set_sorting`: libgit2's default revwalk is
        // the repository-local default order required by `git log` parity.
        Self {
            target,
            filters,
            state: MessagesState::Walk { repository, walk },
        }
    }

    fn from_path_walk(
        target: &'repo CommitLogTarget,
        repository: &'repo git2::Repository,
        plan: &WalkPlan,
        gate: &SpawnGate,
        pathspecs: &[String],
        filters: &'repo CommitLogFilters,
    ) -> Self {
        Self {
            target,
            filters,
            state: MessagesState::PathWalk {
                repository,
                plan: plan.clone(),
                pathspecs: pathspecs.to_vec(),
                first_parent: filters.first_parent(),
                skip: 0,
                gate: gate.clone(),
            },
        }
    }

    fn single(
        target: &'repo CommitLogTarget,
        event: CommitLogEvent,
        filters: &'repo CommitLogFilters,
    ) -> Self {
        Self {
            target,
            filters,
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

    fn next_unfiltered(&mut self) -> Option<CommitLogEvent> {
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
                first_parent,
                skip,
                gate,
            } => {
                let mut command = Command::new("git");
                // The child's directory is explicit: it never inherits the host's.
                command
                    .current_dir(repository.path())
                    .arg("--git-dir")
                    .arg(repository.path())
                    .arg("rev-list")
                    .arg("--max-count=1")
                    .arg(format!("--skip={skip}"));
                if *first_parent {
                    command.arg("--first-parent");
                }
                command
                    .args(plan.pushes.iter().map(ToString::to_string))
                    .args(plan.hides.iter().map(|oid| format!("^{oid}")))
                    .arg("--")
                    .args(pathspecs.iter())
                    .env("GIT_OPTIONAL_LOCKS", "0")
                    .env("GIT_NO_LAZY_FETCH", "1")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                // The spawn is under the host's gate and its wait is not (`git/spawn_gate.rs`); the probe
                // watches the child's lifetime, so a reader held back by the gate is not yet counted.
                let output = gate.spawn(&mut command).and_then(|child| {
                    let _reader_probe = path_reader_probe_enter(self.target);
                    child.wait_with_output()
                });
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

impl Iterator for RepositoryMessages<'_> {
    type Item = CommitLogEvent;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let event = self.next_unfiltered()?;
            if !matches!(&event, CommitLogEvent::Entry(entry) if !self.filters.allows(entry)) {
                return Some(event);
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        use std::collections::BTreeMap;
        use std::sync::{Mutex, OnceLock};

        /// The path readers (a spawned `git rev-list` child, from its spawn to its wait) of one watched
        /// member-id prefix.
        #[derive(Default)]
        struct PathReaderCounts {
            active: usize,
            max_active: usize,
        }

        fn path_reader_probe() -> &'static Mutex<BTreeMap<String, PathReaderCounts>> {
            static PROBE: OnceLock<Mutex<BTreeMap<String, PathReaderCounts>>> = OnceLock::new();
            PROBE.get_or_init(|| Mutex::new(BTreeMap::new()))
        }

        fn path_reader_probe_enter(target: &CommitLogTarget) -> PathReaderProbeGuard {
            let mut watched = path_reader_probe().lock().unwrap();
            let prefix = watched
                .keys()
                .find(|prefix| target.member_id.starts_with(prefix.as_str()))
                .cloned();
            if let Some(prefix) = &prefix {
                let counts = watched.get_mut(prefix).unwrap();
                counts.active += 1;
                counts.max_active = counts.max_active.max(counts.active);
            }
            PathReaderProbeGuard { prefix }
        }

        struct PathReaderProbeGuard {
            prefix: Option<String>,
        }

        impl Drop for PathReaderProbeGuard {
            fn drop(&mut self) {
                if let Some(prefix) = &self.prefix {
                    path_reader_probe()
                        .lock()
                        .unwrap()
                        .get_mut(prefix)
                        .unwrap()
                        .active -= 1;
                }
            }
        }

        /// Starts counting the path readers of members whose id starts with `prefix`. Watches of different
        /// prefixes do not disturb each other, so tests that run side by side may each have one.
        fn watch_path_readers(prefix: &str) {
            let mut watched = path_reader_probe().lock().unwrap();
            assert_eq!(watched.get(prefix).map_or(0, |counts| counts.active), 0);
            watched.insert(prefix.to_owned(), PathReaderCounts::default());
        }

        /// The most path readers of `prefix`'s members alive at once since the watch began, with the watch still on.
        fn path_readers_seen(prefix: &str) -> usize {
            path_reader_probe().lock().unwrap()[prefix].max_active
        }

        fn finish_watching_path_readers(prefix: &str) -> usize {
            let counts = path_reader_probe().lock().unwrap().remove(prefix).unwrap();
            assert_eq!(counts.active, 0);
            counts.max_active
        }
    } else {
        /// Nothing watches path readers outside tests.
        struct PathReaderProbeGuard;

        fn path_reader_probe_enter(_target: &CommitLogTarget) -> PathReaderProbeGuard {
            PathReaderProbeGuard
        }
    }
}

impl std::iter::FusedIterator for RepositoryMessages<'_> {}

/// Open the no-operand, default selection: `@root` plus every active member.
///
/// Reading the manifest is the only request-level fallible operation. Each
/// repository thereafter either exposes its own HEAD cursor or a degradation
/// event. This path performs no integrity gate, transport operation, or lock.
pub fn open_default_head_histories(
    workspace_root: &Path,
    gate: &SpawnGate,
) -> ModelResult<Vec<RepositoryHistory>> {
    let request = crate::LogRequest::default();
    Ok(open_request_histories(workspace_root, &request, gate)?.into_histories())
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
