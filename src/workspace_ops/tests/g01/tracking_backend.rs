use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};

pub(crate) const TEST_COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[derive(Clone)]
pub(crate) struct TrackingBackend {
    fetch: Arc<OverlapTracker>,
    push: Arc<OverlapTracker>,
}

impl TrackingBackend {
    pub(crate) fn new(expected_overlap: usize) -> Self {
        Self {
            fetch: Arc::new(OverlapTracker::new(expected_overlap)),
            push: Arc::new(OverlapTracker::new(expected_overlap)),
        }
    }

    pub(crate) fn fetch_peak(&self) -> usize {
        self.fetch.peak()
    }

    pub(crate) fn push_peak(&self) -> usize {
        self.push.peak()
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

    fn read_ref(&self, _path: &Path, _ref_spec: &str) -> ModelResult<Option<String>> {
        Ok(Some(TEST_COMMIT.to_owned()))
    }

    fn is_ancestor(&self, _path: &Path, _ancestor: &str, _descendant: &str) -> ModelResult<bool> {
        Ok(true)
    }
}
