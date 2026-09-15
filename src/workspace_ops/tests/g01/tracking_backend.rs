use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
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

/// One publication call observed by the double, in arrival order: an
/// advertisement read (`ls_remote_url`) or a captured push (`push_prepared`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemoteCall {
    Read {
        path: PathBuf,
        url: String,
        remote: String,
        identity_repo: Option<PathBuf>,
    },
    Push {
        path: PathBuf,
        remote: String,
        url: String,
        refspecs: Vec<String>,
    },
}

/// A remote of one configured repository, with the fetch refspecs its
/// configuration carries.
#[derive(Clone, Debug)]
pub(crate) struct ConfiguredRemote {
    pub(crate) name: String,
    pub(crate) url: Option<String>,
    pub(crate) push_url: Option<String>,
    pub(crate) fetch_refspecs: Vec<String>,
}

impl ConfiguredRemote {
    /// A remote as `git clone` writes it: one URL and a forced branch mapping.
    pub(crate) fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_owned(),
            url: Some(url.to_owned()),
            push_url: None,
            fetch_refspecs: vec![format!("+refs/heads/*:refs/remotes/{name}/*")],
        }
    }
}

/// `calls` grouped by the host each one's URL reaches, in their order within
/// each host. Calls to different hosts may overlap, so a test that spans hosts
/// compares these.
pub(crate) fn calls_by_host(calls: Vec<RemoteCall>) -> BTreeMap<Option<String>, Vec<RemoteCall>> {
    let mut hosts: BTreeMap<Option<String>, Vec<RemoteCall>> = BTreeMap::new();
    for call in calls {
        let (RemoteCall::Read { url, .. } | RemoteCall::Push { url, .. }) = &call;
        let host = crate::git::git_host(url);
        hosts.entry(host).or_default().push(call);
    }
    hosts
}

#[derive(Clone)]
pub(crate) struct TrackingBackend {
    fetch: Arc<OverlapTracker>,
    push: Arc<OverlapTracker>,
    read: Arc<OverlapTracker>,
    /// Reads that start once a push has been recorded: a push's root proof.
    post_push_read: Arc<OverlapTracker>,
    anonymous: Arc<Mutex<Vec<AnonymousTransfer>>>,
    anonymous_failure: Arc<Mutex<Option<String>>>,
    model: Arc<Mutex<Model>>,
}

impl TrackingBackend {
    pub(crate) fn new(expected_overlap: usize) -> Self {
        Self {
            fetch: Arc::new(OverlapTracker::new(expected_overlap)),
            push: Arc::new(OverlapTracker::new(expected_overlap)),
            read: Arc::new(OverlapTracker::new(1)),
            post_push_read: Arc::new(OverlapTracker::new(1)),
            anonymous: Arc::new(Mutex::new(Vec::new())),
            anonymous_failure: Arc::new(Mutex::new(None)),
            model: Arc::new(Mutex::new(Model::default())),
        }
    }

    /// Hold advertisement reads for overlap as `new` holds fetches and pushes.
    /// Reads expect no overlap by default, so sequential reads never wait.
    /// Reads after a recorded push have their own counter.
    pub(crate) fn with_read_overlap(mut self, expected_overlap: usize) -> Self {
        self.read = Arc::new(OverlapTracker::new(expected_overlap));
        self
    }

    /// As `with_read_overlap`, for reads that start once a push has been
    /// recorded, which in a push are its root proof's reads.
    pub(crate) fn with_post_push_read_overlap(mut self, expected_overlap: usize) -> Self {
        self.post_push_read = Arc::new(OverlapTracker::new(expected_overlap));
        self
    }

    pub(crate) fn fetch_peak(&self) -> usize {
        self.fetch.peak()
    }

    pub(crate) fn push_peak(&self) -> usize {
        self.push.peak()
    }

    pub(crate) fn read_peak(&self) -> usize {
        self.read.peak()
    }

    pub(crate) fn post_push_read_peak(&self) -> usize {
        self.post_push_read.peak()
    }

    /// Fail every advertisement read of `url` with `git_command_failed`. The
    /// read is still recorded and counted.
    pub(crate) fn fail_reads(&self, url: &str, detail: &str) {
        self.model()
            .failing_reads
            .insert(url.to_owned(), detail.to_owned());
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

    /// Configure `path` as a repository attached to `branch` at `commit`.
    pub(crate) fn set_head(&self, path: &Path, branch: &str, commit: &str) {
        self.model().repository(path).head = crate::git::GitHeadState {
            branch: Some(branch.to_owned()),
            commit: Some(commit.to_owned()),
            is_detached: false,
        };
    }

    pub(crate) fn set_materialized(&self, path: &Path, materialized: bool) {
        self.model().repository(path).materialized = materialized;
    }

    pub(crate) fn add_remote_config(&self, path: &Path, remote: ConfiguredRemote) {
        self.model().repository(path).remotes.push(remote);
    }

    /// Create the lightweight tag `name` at `commit`, which `tag_list` and
    /// `read_ref` of `refs/tags/<name>` then answer.
    pub(crate) fn set_tag(&self, path: &Path, name: &str, commit: &str) {
        self.model()
            .repository(path)
            .tags
            .insert(name.to_owned(), commit.to_owned());
    }

    /// Capture every refspec `prepare_push` returns for `path` as forced. A
    /// request gives every repository one refspec, so only the double can mix
    /// forced and ordinary transfers in one push.
    pub(crate) fn force_pushes(&self, path: &Path) {
        self.model().repository(path).forced_pushes = true;
    }

    pub(crate) fn fetch_refspecs(&self, path: &Path, remote: &str) -> Option<Vec<String>> {
        self.model()
            .repositories
            .get(path)?
            .remotes
            .iter()
            .find(|configured| configured.name == remote)
            .map(|configured| configured.fetch_refspecs.clone())
    }

    /// Serve `files` from the tree of `commit` in `path`; any other file is
    /// absent from that commit.
    pub(crate) fn commit_files(&self, path: &Path, commit: &str, files: &[(&str, Vec<u8>)]) {
        let files = files
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), bytes.clone()))
            .collect();
        self.model()
            .committed
            .insert((path.to_path_buf(), commit.to_owned()), files);
    }

    /// Answer `is_ancestor(ancestor, descendant)` from the table; an `Err` is a
    /// Git failure such as missing objects. Undeclared pairs keep the double's
    /// original answer, true, so declare `Ok(false)` wherever a proof must fail.
    pub(crate) fn set_ancestry(
        &self,
        ancestor: &str,
        descendant: &str,
        answer: Result<bool, &str>,
    ) {
        self.model().ancestry.insert(
            (ancestor.to_owned(), descendant.to_owned()),
            answer.map_err(ToOwned::to_owned),
        );
    }

    /// Serve one advertisement store at every URL in `urls`, so one repository
    /// can be reached through several spellings: a push through any of them
    /// moves what all of them advertise.
    pub(crate) fn serve(&self, urls: &[&str], refs: &[(&str, &str)]) {
        let mut model = self.model();
        model.stores.push(
            refs.iter()
                .map(|(name, target)| ((*name).to_owned(), (*target).to_owned()))
                .collect(),
        );
        let store = model.stores.len() - 1;
        for url in urls {
            model.served.insert((*url).to_owned(), store);
        }
    }

    /// The object `name` points at in the store served at `url`.
    pub(crate) fn advertised_ref(&self, url: &str, name: &str) -> Option<String> {
        self.model().store(url)?.get(name).cloned()
    }

    /// Every advertisement read and captured push, in arrival order.
    pub(crate) fn remote_calls(&self) -> Vec<RemoteCall> {
        self.model().calls.clone()
    }

    pub(crate) fn remote_reads(&self) -> Vec<RemoteCall> {
        self.remote_calls()
            .into_iter()
            .filter(|call| matches!(call, RemoteCall::Read { .. }))
            .collect()
    }

    pub(crate) fn prepared_pushes(&self) -> Vec<RemoteCall> {
        self.remote_calls()
            .into_iter()
            .filter(|call| matches!(call, RemoteCall::Push { .. }))
            .collect()
    }

    /// Every `validate_url_identity` call as `(identity repo, remote, URL)`, in
    /// arrival order. The double accepts each one.
    pub(crate) fn url_identity_checks(&self) -> Vec<(Option<PathBuf>, String, String)> {
        self.model().identity_checks.clone()
    }

    fn model(&self) -> MutexGuard<'_, Model> {
        self.model.lock().unwrap()
    }

    fn configured(&self, path: &Path) -> Option<ConfiguredRepository> {
        self.model().repositories.get(path).cloned()
    }
}

/// What a test configured on the double, and the publication calls it saw. A
/// path, commit, URL or ancestry pair nobody configured keeps the double's
/// original answer.
#[derive(Default)]
struct Model {
    repositories: BTreeMap<PathBuf, ConfiguredRepository>,
    committed: BTreeMap<(PathBuf, String), BTreeMap<String, Vec<u8>>>,
    ancestry: BTreeMap<(String, String), Result<bool, String>>,
    stores: Vec<BTreeMap<String, String>>,
    served: BTreeMap<String, usize>,
    /// URLs whose reads fail, each with its failure detail.
    failing_reads: BTreeMap<String, String>,
    calls: Vec<RemoteCall>,
    identity_checks: Vec<(Option<PathBuf>, String, String)>,
}

impl Model {
    fn repository(&mut self, path: &Path) -> &mut ConfiguredRepository {
        self.repositories.entry(path.to_path_buf()).or_default()
    }

    fn store(&self, url: &str) -> Option<&BTreeMap<String, String>> {
        self.served.get(url).map(|store| &self.stores[*store])
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> ModelResult<bool> {
        match self
            .ancestry
            .get(&(ancestor.to_owned(), descendant.to_owned()))
        {
            Some(Ok(answer)) => Ok(*answer),
            Some(Err(detail)) => Err(ModelError::new(ErrorCode::GitCommandFailed, detail.clone())),
            None => Ok(true),
        }
    }

    /// Apply a captured push to the store served at `url`, all or nothing. As
    /// libgit2 does before transfer, refuse an ordinary update of a ref whose
    /// current object is not an ancestor of the pushed one.
    fn accept_push(&mut self, url: &str, refspecs: &[String]) -> ModelResult<()> {
        let Some(&store) = self.served.get(url) else {
            return Ok(());
        };
        let mut refs = self.stores[store].clone();
        for refspec in refspecs {
            let plain = refspec.strip_prefix('+').unwrap_or(refspec);
            let (source, destination) = plain.split_once(':').unwrap_or((plain, plain));
            if source.is_empty() {
                refs.remove(destination);
                continue;
            }
            if let Some(current) = refs.get(destination)
                && current != source
                && !refspec.starts_with('+')
                && !self.is_ancestor(current, source).unwrap_or(false)
            {
                return Err(ModelError::new(
                    ErrorCode::RemoteRejected,
                    format!(
                        "{url} rejected {destination}: cannot push non-fastforwardable reference"
                    ),
                ));
            }
            refs.insert(destination.to_owned(), source.to_owned());
        }
        self.stores[store] = refs;
        Ok(())
    }
}

#[derive(Clone)]
struct ConfiguredRepository {
    materialized: bool,
    head: crate::git::GitHeadState,
    remotes: Vec<ConfiguredRemote>,
    /// Lightweight tags: each name maps to its commit.
    tags: BTreeMap<String, String>,
    forced_pushes: bool,
}

/// An unconfigured repository gets the double's original answers.
impl Default for ConfiguredRepository {
    fn default() -> Self {
        Self {
            materialized: true,
            head: crate::git::GitHeadState {
                branch: Some("main".to_owned()),
                commit: Some(TEST_COMMIT.to_owned()),
                is_detached: false,
            },
            remotes: Vec::new(),
            tags: BTreeMap::new(),
            forced_pushes: false,
        }
    }
}

/// Resolve the attached branch ref, a tag ref, or a full object id, of a
/// configured repository.
fn resolve(repository: &ConfiguredRepository, name: &str) -> Option<String> {
    let name = name.strip_suffix("^{commit}").unwrap_or(name);
    let branch = repository
        .head
        .branch
        .as_ref()
        .map(|branch| format!("refs/heads/{branch}"));
    if branch.as_deref() == Some(name) {
        return repository.head.commit.clone();
    }
    if let Some(tag) = name.strip_prefix("refs/tags/") {
        return repository.tags.get(tag).cloned();
    }
    (name.len() == 40 && name.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| name.to_owned())
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

    fn is_repository(&self, path: &Path) -> ModelResult<bool> {
        Ok(self.configured(path).unwrap_or_default().materialized)
    }

    fn read_file_at_commit(
        &self,
        path: &Path,
        commit: &str,
        relative_path: &str,
    ) -> ModelResult<Option<Vec<u8>>> {
        let model = self.model();
        let files = model
            .committed
            .get(&(path.to_path_buf(), commit.to_owned()))
            .ok_or_else(|| {
                ModelError::new(
                    ErrorCode::UnsupportedOperation,
                    "read_file_at_commit is not implemented by this GitBackend",
                )
            })?;
        Ok(files.get(relative_path).cloned())
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

    fn tag_list(&self, path: &Path) -> ModelResult<Vec<String>> {
        let repository = self.configured(path).unwrap_or_default();
        Ok(repository.tags.into_keys().collect())
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

    fn head(&self, path: &Path) -> ModelResult<crate::git::GitHeadState> {
        Ok(self.configured(path).unwrap_or_default().head)
    }

    fn remotes(&self, path: &Path) -> ModelResult<Vec<crate::git::GitRemote>> {
        let repository = self.configured(path).unwrap_or_default();
        Ok(repository
            .remotes
            .into_iter()
            .map(|remote| crate::git::GitRemote {
                name: remote.name,
                url: remote.url,
                push_url: remote.push_url,
            })
            .collect())
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
        let Some(repository) = self.configured(path) else {
            return Ok(crate::git::GitPreparedPush {
                remote: remote.to_owned(),
                url: format!(
                    "ssh://{}.invalid/repo.git",
                    path.file_name().unwrap().to_string_lossy()
                ),
                refspecs: vec![refspec.to_owned()],
            });
        };
        let configured = repository
            .remotes
            .iter()
            .find(|configured| configured.name == remote)
            .ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MissingRemote,
                    format!("missing remote '{remote}'"),
                )
            })?;
        let url = configured
            .push_url
            .clone()
            .or_else(|| configured.url.clone())
            .ok_or_else(|| {
                ModelError::new(ErrorCode::MissingRemote, "remote has no destination URL")
            })?;
        // Capture the source as an object id, as `Git2Backend` does. The double
        // models explicit `refs/` destinations only.
        let prefix = if refspec.starts_with('+') || repository.forced_pushes {
            "+"
        } else {
            ""
        };
        let plain = refspec.strip_prefix('+').unwrap_or(refspec);
        let (source, destination) = plain.split_once(':').unwrap_or((plain, plain));
        let object = if source.is_empty() {
            Some(String::new())
        } else {
            resolve(&repository, source)
        };
        match object {
            Some(object) if destination.starts_with("refs/") => Ok(crate::git::GitPreparedPush {
                remote: remote.to_owned(),
                url,
                refspecs: vec![format!("{prefix}{object}:{destination}")],
            }),
            _ => Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "push refspec cannot resolve to concrete source objects and destination refs",
            )),
        }
    }

    fn ls_remote_url(
        &self,
        path: &Path,
        url: &str,
        remote: &str,
        identity_repo: Option<&Path>,
    ) -> ModelResult<Vec<crate::git::GitRemoteRef>> {
        let after_push = {
            let mut model = self.model();
            let after_push = model
                .calls
                .iter()
                .any(|call| matches!(call, RemoteCall::Push { .. }));
            model.calls.push(RemoteCall::Read {
                path: path.to_path_buf(),
                url: url.to_owned(),
                remote: remote.to_owned(),
                identity_repo: identity_repo.map(Path::to_path_buf),
            });
            after_push
        };
        if after_push {
            self.post_push_read.run();
        } else {
            self.read.run();
        }
        let (failure, served) = {
            let model = self.model();
            (
                model.failing_reads.get(url).cloned(),
                model.store(url).cloned(),
            )
        };
        if let Some(detail) = failure {
            return Err(ModelError::new(ErrorCode::GitCommandFailed, detail));
        }
        match served {
            Some(refs) => Ok(refs
                .into_iter()
                .map(|(name, target)| crate::git::GitRemoteRef { name, target })
                .collect()),
            None => self.ls_remote(path, remote),
        }
    }

    fn push_prepared(
        &self,
        path: &Path,
        plan: &crate::git::GitPreparedPush,
    ) -> ModelResult<crate::git::GitPushResult> {
        self.model().calls.push(RemoteCall::Push {
            path: path.to_path_buf(),
            remote: plan.remote.clone(),
            url: plan.url.clone(),
            refspecs: plan.refspecs.clone(),
        });
        self.push.run();
        self.model().accept_push(&plan.url, &plan.refspecs)?;
        Ok(crate::git::GitPushResult {
            remote: plan.remote.clone(),
            refspec: plan.refspecs.first().cloned().unwrap_or_default(),
        })
    }

    fn validate_url_identity(
        &self,
        identity_repo: Option<&Path>,
        remote: &str,
        url: &str,
    ) -> ModelResult<()> {
        self.model().identity_checks.push((
            identity_repo.map(Path::to_path_buf),
            remote.to_owned(),
            url.to_owned(),
        ));
        Ok(())
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

    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>> {
        Ok(match self.configured(path) {
            Some(repository) => resolve(&repository, ref_spec),
            None => Some(TEST_COMMIT.to_owned()),
        })
    }

    fn is_ancestor(&self, _path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool> {
        self.model().is_ancestor(ancestor, descendant)
    }
}
