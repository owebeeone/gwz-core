use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};

pub(crate) const TEST_COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// One anonymous local transfer observed by the double (LCM1.0c).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AnonymousTransfer {
    Fetch {
        path: PathBuf,
        url: String,
        refspecs: Vec<String>,
    },
    Push {
        path: PathBuf,
        url: String,
        refspec: String,
    },
}

#[derive(Clone)]
pub(crate) struct TrackingBackend {
    fetch: Arc<OverlapTracker>,
    push: Arc<OverlapTracker>,
    anonymous: Arc<Mutex<Vec<AnonymousTransfer>>>,
    anonymous_failure: Arc<Mutex<Option<String>>>,
}

impl TrackingBackend {
    pub(crate) fn new(expected_overlap: usize) -> Self {
        Self {
            fetch: Arc::new(OverlapTracker::new(expected_overlap)),
            push: Arc::new(OverlapTracker::new(expected_overlap)),
            anonymous: Arc::new(Mutex::new(Vec::new())),
            anonymous_failure: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn fetch_peak(&self) -> usize {
        self.fetch.peak()
    }

    pub(crate) fn push_peak(&self) -> usize {
        self.push.peak()
    }

    /// Every anonymous fetch/push the double received, in order.
    pub(crate) fn anonymous_transfers(&self) -> Vec<AnonymousTransfer> {
        self.anonymous.lock().unwrap().clone()
    }

    /// Make the next anonymous transfer fail with `git_command_failed`.
    pub(crate) fn fail_next_anonymous(&self, detail: &str) {
        *self.anonymous_failure.lock().unwrap() = Some(detail.to_owned());
    }

    fn record_anonymous(&self, transfer: AnonymousTransfer) -> ModelResult<()> {
        self.anonymous.lock().unwrap().push(transfer);
        match self.anonymous_failure.lock().unwrap().take() {
            Some(detail) => Err(ModelError::new(ErrorCode::GitCommandFailed, detail)),
            None => Ok(()),
        }
    }
}

struct OverlapTracker {
    expected_overlap: usize,
    active: AtomicUsize,
    peak: AtomicUsize,
    entered: Mutex<usize>,
    all_entered: Condvar,
}

impl OverlapTracker {
    fn new(expected_overlap: usize) -> Self {
        Self {
            expected_overlap,
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            entered: Mutex::new(0),
            all_entered: Condvar::new(),
        }
    }

    fn run(&self) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.record_peak(active);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut entered = self.entered.lock().unwrap();
        *entered += 1;
        self.all_entered.notify_all();
        while *entered < self.expected_overlap {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            let (next, timeout) = self.all_entered.wait_timeout(entered, remaining).unwrap();
            entered = next;
            if timeout.timed_out() {
                break;
            }
        }
        drop(entered);
        self.active.fetch_sub(1, Ordering::SeqCst);
    }

    fn record_peak(&self, active: usize) {
        let mut observed = self.peak.load(Ordering::SeqCst);
        while active > observed {
            match self
                .peak
                .compare_exchange(observed, active, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(next) => observed = next,
            }
        }
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

impl GitBackend for TrackingBackend {
    fn test_init_repo(&self, _repo: &Path, _spec: &crate::git::TestRepoSpec) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "fixture operation is not supported by this transfer spy",
        ))
    }
    fn test_create_commit(
        &self,
        _repo: &Path,
        _spec: &crate::git::TestCommitSpec,
    ) -> ModelResult<String> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "fixture operation is not supported by this transfer spy",
        ))
    }
    fn test_read_commit(&self, _repo: &Path, _oid: &str) -> ModelResult<crate::git::TestCommit> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "fixture operation is not supported by this transfer spy",
        ))
    }
    fn test_set_ref(
        &self,
        _repo: &Path,
        _name: &str,
        _target: Option<&crate::git::TestRefTarget>,
    ) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "fixture operation is not supported by this transfer spy",
        ))
    }
    fn test_set_head(&self, _repo: &Path, _state: &crate::git::TestHead) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "fixture operation is not supported by this transfer spy",
        ))
    }
    fn test_replace_index(
        &self,
        _repo: &Path,
        _entries: &[crate::git::TestIndexEntry],
    ) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "fixture operation is not supported by this transfer spy",
        ))
    }
    fn test_read_index(&self, _repo: &Path) -> ModelResult<Vec<crate::git::TestIndexEntry>> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "fixture operation is not supported by this transfer spy",
        ))
    }
    fn test_set_config(&self, _repo: &Path, _key: &str, _values: &[String]) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "fixture operation is not supported by this transfer spy",
        ))
    }
    fn test_read_config(&self, _repo: &Path, _key: &str) -> ModelResult<Vec<String>> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "fixture operation is not supported by this transfer spy",
        ))
    }

    fn is_repository(&self, _path: &Path) -> ModelResult<bool> {
        Ok(true)
    }

    fn stage_paths(
        &self,
        _path: &Path,
        _pathspecs: &[&str],
    ) -> ModelResult<crate::git::GitStageResult> {
        Ok(crate::git::GitStageResult { staged: 0 })
    }

    fn commit(
        &self,
        _path: &Path,
        _message: &str,
        _all: bool,
    ) -> ModelResult<crate::git::GitCommitResult> {
        Ok(crate::git::GitCommitResult {
            commit: TEST_COMMIT.to_owned(),
        })
    }

    fn tag_create(
        &self,
        _path: &Path,
        name: &str,
        _message: Option<&str>,
        _signed: bool,
    ) -> ModelResult<crate::git::GitTagResult> {
        Ok(crate::git::GitTagResult {
            name: name.to_owned(),
            commit: TEST_COMMIT.to_owned(),
        })
    }

    fn tag_list(&self, _path: &Path) -> ModelResult<Vec<String>> {
        Ok(Vec::new())
    }

    fn tag_delete(&self, _path: &Path, _name: &str) -> ModelResult<()> {
        Ok(())
    }

    fn tag_fetch(&self, _path: &Path, remote: &str) -> ModelResult<crate::git::GitFetchResult> {
        Ok(crate::git::GitFetchResult {
            remote: remote.to_owned(),
        })
    }

    fn create_repo(&self, path: &Path) -> ModelResult<crate::git::GitCreateResult> {
        Ok(crate::git::GitCreateResult {
            path: path.to_path_buf(),
        })
    }

    fn clone_repo(&self, url: &str, path: &Path) -> ModelResult<crate::git::GitCloneResult> {
        let _ = url;
        Ok(crate::git::GitCloneResult {
            path: path.to_path_buf(),
            head: self.head(path)?,
        })
    }

    fn fetch(&self, _path: &Path, remote: &str) -> ModelResult<crate::git::GitFetchResult> {
        self.fetch.run();
        Ok(crate::git::GitFetchResult {
            remote: remote.to_owned(),
        })
    }

    fn ls_remote(&self, _path: &Path, _remote: &str) -> ModelResult<Vec<crate::git::GitRemoteRef>> {
        Ok(vec![crate::git::GitRemoteRef {
            name: "refs/heads/main".to_owned(),
            target: TEST_COMMIT.to_owned(),
        }])
    }

    fn fast_forward(
        &self,
        _path: &Path,
        _branch: &str,
        _upstream_ref: &str,
    ) -> ModelResult<crate::git::GitUpdateResult> {
        Ok(crate::git::GitUpdateResult {
            updated: false,
            commit: Some(TEST_COMMIT.to_owned()),
        })
    }

    fn merge_upstream(
        &self,
        _path: &Path,
        _branch: &str,
        _upstream_ref: &str,
    ) -> ModelResult<crate::git::GitIntegrateResult> {
        Ok(crate::git::GitIntegrateResult::clean(
            TEST_COMMIT.to_owned(),
        ))
    }

    fn prepare_merge_upstream_checked(
        &self,
        _path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        _attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<crate::git::GitPreparedMerge> {
        if branch == "main" && expected_before == TEST_COMMIT && source_commit == TEST_COMMIT {
            Ok(crate::git::GitPreparedMerge::Unchanged)
        } else {
            Err(ModelError::new(
                ErrorCode::MergeDrift,
                "tracking backend received unexpected prepared merge inputs",
            ))
        }
    }

    fn validate_prepared_merge_upstream_state(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        prepared: &crate::git::GitPreparedMerge,
    ) -> ModelResult<()> {
        let current = self.prepare_merge_upstream_checked(
            path,
            branch,
            expected_before,
            source_commit,
            None,
        )?;
        if &current == prepared {
            Ok(())
        } else {
            Err(ModelError::new(
                ErrorCode::MergeDrift,
                "tracking backend prepared merge changed",
            ))
        }
    }

    fn execute_prepared_merge_upstream_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        _message: &str,
        prepared: &crate::git::GitPreparedMerge,
    ) -> ModelResult<crate::git::GitIntegrateResult> {
        self.validate_prepared_merge_upstream_state(
            path,
            branch,
            expected_before,
            source_commit,
            prepared,
        )?;
        Ok(crate::git::GitIntegrateResult::clean(
            TEST_COMMIT.to_owned(),
        ))
    }

    fn rebase_onto(
        &self,
        _path: &Path,
        _branch: &str,
        _upstream_ref: &str,
    ) -> ModelResult<crate::git::GitIntegrateResult> {
        Ok(crate::git::GitIntegrateResult::clean(
            TEST_COMMIT.to_owned(),
        ))
    }

    fn reset_hard(
        &self,
        _path: &Path,
        _branch: &str,
        _upstream_ref: &str,
    ) -> ModelResult<crate::git::GitUpdateResult> {
        Ok(crate::git::GitUpdateResult {
            updated: true,
            commit: Some(TEST_COMMIT.to_owned()),
        })
    }

    fn checkout_commit(
        &self,
        _path: &Path,
        commit: &str,
    ) -> ModelResult<crate::git::GitUpdateResult> {
        Ok(crate::git::GitUpdateResult {
            updated: true,
            commit: Some(commit.to_owned()),
        })
    }

    fn checkout_branch(
        &self,
        _path: &Path,
        _branch: &str,
        commit: &str,
    ) -> ModelResult<crate::git::GitUpdateResult> {
        Ok(crate::git::GitUpdateResult {
            updated: true,
            commit: Some(commit.to_owned()),
        })
    }

    fn status(&self, _path: &Path) -> ModelResult<crate::git::GitStatus> {
        Ok(crate::git::GitStatus::clean())
    }

    fn head(&self, _path: &Path) -> ModelResult<crate::git::GitHeadState> {
        Ok(crate::git::GitHeadState {
            branch: Some("main".to_owned()),
            commit: Some(TEST_COMMIT.to_owned()),
            is_detached: false,
        })
    }

    fn remotes(&self, _path: &Path) -> ModelResult<Vec<crate::git::GitRemote>> {
        Ok(Vec::new())
    }

    fn add_remote(
        &self,
        _path: &Path,
        name: &str,
        url: &str,
    ) -> ModelResult<crate::git::GitRemoteResult> {
        Ok(crate::git::GitRemoteResult {
            remote: crate::git::GitRemote {
                name: name.to_owned(),
                url: Some(url.to_owned()),
                push_url: None,
            },
        })
    }

    fn push(
        &self,
        _path: &Path,
        remote: &str,
        refspec: &str,
    ) -> ModelResult<crate::git::GitPushResult> {
        self.push.run();
        Ok(crate::git::GitPushResult {
            remote: remote.to_owned(),
            refspec: refspec.to_owned(),
        })
    }

    fn prepare_push(
        &self,
        path: &Path,
        remote: &str,
        refspec: &str,
    ) -> ModelResult<crate::git::GitPreparedPush> {
        Ok(crate::git::GitPreparedPush {
            remote: remote.to_owned(),
            url: format!(
                "ssh://{}.invalid/repo.git",
                path.file_name().unwrap().to_string_lossy()
            ),
            refspecs: vec![refspec.to_owned()],
        })
    }

    fn ls_remote_url(
        &self,
        path: &Path,
        _url: &str,
        remote: &str,
        _identity_repo: Option<&Path>,
    ) -> ModelResult<Vec<crate::git::GitRemoteRef>> {
        self.ls_remote(path, remote)
    }

    fn push_prepared(
        &self,
        path: &Path,
        plan: &crate::git::GitPreparedPush,
    ) -> ModelResult<crate::git::GitPushResult> {
        self.push(path, &plan.remote, &plan.refspecs[0])
    }

    fn fetch_anonymous(
        &self,
        path: &Path,
        url: &str,
        refspecs: &[&str],
    ) -> ModelResult<crate::git::GitFetchResult> {
        self.record_anonymous(AnonymousTransfer::Fetch {
            path: path.to_path_buf(),
            url: url.to_owned(),
            refspecs: refspecs.iter().map(|spec| (*spec).to_owned()).collect(),
        })?;
        Ok(crate::git::GitFetchResult {
            remote: url.to_owned(),
        })
    }

    fn push_anonymous(
        &self,
        path: &Path,
        url: &str,
        refspec: &str,
    ) -> ModelResult<crate::git::GitPushResult> {
        self.record_anonymous(AnonymousTransfer::Push {
            path: path.to_path_buf(),
            url: url.to_owned(),
            refspec: refspec.to_owned(),
        })?;
        Ok(crate::git::GitPushResult {
            remote: url.to_owned(),
            refspec: refspec.to_owned(),
        })
    }

    fn read_ref(&self, _path: &Path, _ref_spec: &str) -> ModelResult<Option<String>> {
        Ok(Some(TEST_COMMIT.to_owned()))
    }

    fn is_ancestor(&self, _path: &Path, _ancestor: &str, _descendant: &str) -> ModelResult<bool> {
        Ok(true)
    }
}
