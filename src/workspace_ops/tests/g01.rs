use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::artifact::read_lock;
use crate::git::{Git2Backend, GitBackend};
use crate::model::ModelResult;
use crate::operation::NullSink;

use super::*;

pub(crate) const TEST_COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn force_push_unrelated_main(fixture: &RemoteFixture, backend: &Git2Backend) -> String {
    let oid = create_orphan_ref(&fixture.source, "refs/heads/main", "unrelated remote\n");
    backend
        .push(
            &fixture.source,
            "origin",
            "+refs/heads/main:refs/heads/main",
        )
        .unwrap();
    oid
}

#[derive(Clone)]
pub(crate) struct TrackingBackend {
    pub(crate) fetch: Arc<OverlapTracker>,
    pub(crate) push: Arc<OverlapTracker>,
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

pub(crate) struct OverlapTracker {
    pub(crate) expected_overlap: usize,
    pub(crate) active: AtomicUsize,
    pub(crate) peak: AtomicUsize,
    pub(crate) entered: Mutex<usize>,
    pub(crate) all_entered: Condvar,
}

impl OverlapTracker {
    pub(crate) fn new(expected_overlap: usize) -> Self {
        Self {
            expected_overlap,
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            entered: Mutex::new(0),
            all_entered: Condvar::new(),
        }
    }

    pub(crate) fn run(&self) {
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

    pub(crate) fn record_peak(&self, active: usize) {
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

    pub(crate) fn peak(&self) -> usize {
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

#[test]
pub(crate) fn pull_head_returns_noop_for_local_only_member() {
    let temp = TempDir::new("pull-local-only");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    handle_create_repo(
        &backend,
        temp.path(),
        create_repo_request("repos/app", None, None),
        "op_repo",
    )
    .unwrap();

    let response = handle_pull_head(&backend, temp.path(), pull_head_request(), "op_pull").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Noop
    );
    assert_eq!(
        response.response.members.single().status,
        crate::MemberStatus::Noop
    );
}

#[test]
pub(crate) fn pull_head_noops_member_without_fetch_remote_and_continues() {
    let temp = TempDir::new("pull-no-fetch-remote");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    let local_path = temp.path().join("local-repo");
    backend.create_repo(&local_path).unwrap();
    commit_file(&local_path, "README.md", "one", "initial", &[]).unwrap();
    handle_add_existing_repo(
        &backend,
        temp.path(),
        crate::AddExistingRepoRequest {
            meta: request_meta_with_workspace(),
            repository_path: local_path.to_string_lossy().into_owned(),
            member_path: None,
            member_id: None,
            source_id: None,
        },
        "op_add_local",
    )
    .unwrap();

    let fixture = RemoteFixture::new("pull-no-fetch-source");
    fixture.commit_and_push("README.md", "one", "initial", &backend);
    let remote_path = temp.path().join("remote");
    backend
        .clone_repo(fixture.remote_url(), &remote_path)
        .unwrap();
    handle_add_existing_repo(
        &backend,
        temp.path(),
        crate::AddExistingRepoRequest {
            meta: request_meta_with_workspace(),
            repository_path: remote_path.to_string_lossy().into_owned(),
            member_path: None,
            member_id: None,
            source_id: None,
        },
        "op_add_remote",
    )
    .unwrap();

    let response = handle_pull_head(&backend, temp.path(), pull_head_request(), "op_pull").unwrap();

    assert_eq!(response.response.members.len(), 2);
    let local = response
        .response
        .members
        .iter()
        .find(|member| member.member_path == "local-repo")
        .unwrap();
    assert_eq!(local.status, crate::MemberStatus::Noop);
    assert_eq!(
        local
            .planned
            .as_ref()
            .and_then(|planned| planned.message.as_deref()),
        Some("no fetch remote configured; skipping pull")
    );
    let remote = response
        .response
        .members
        .iter()
        .find(|member| member.member_path == "remote")
        .unwrap();
    assert_eq!(remote.status, crate::MemberStatus::Noop);
}

#[test]
pub(crate) fn pull_head_fast_forwards_clean_member_and_rewrites_lock() {
    let temp = TempDir::new("pull-ff");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let fixture = RemoteFixture::new("pull-ff-source");
    let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
    backend
        .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
        .unwrap();
    let second = fixture.commit_and_push("README.md", "two", "second", &backend);
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &first)],
    );

    let response = handle_pull_head(&backend, temp.path(), pull_head_request(), "op_pull").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        backend.head(&temp.path().join("repos/app")).unwrap().commit,
        Some(second.clone())
    );
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_app"].commit,
        Some(second)
    );
}

#[test]
pub(crate) fn pull_head_merge_conflict_surfaces_conflicted_status() {
    // `--sync merge` on a member that diverged from its remote on the same file:
    // the merge conflicts, and that must surface as Conflicted end-to-end.
    let temp = TempDir::new("pull-merge-conflict");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let fixture = RemoteFixture::new("pull-merge-conflict-source");
    let a = fixture.commit_and_push("README.md", "base\n", "A", &backend);
    let repos_app = temp.path().join("repos/app");
    backend
        .clone_repo(fixture.remote_url(), &repos_app)
        .unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    let b = commit_file(&repos_app, "README.md", "local\n", "B", &[a_oid]).unwrap();
    fixture.commit_and_push("README.md", "remote\n", "C", &backend);
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &b)],
    );

    let response = handle_pull_head(
        &backend,
        temp.path(),
        pull_head_request_with_sync(crate::SyncBehavior::Merge),
        "op_pull",
    )
    .unwrap();

    let member = response.response.members.single();
    assert_eq!(member.status, crate::MemberStatus::Conflicted);
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Conflicted
    );
    // HEAD did not move; the conflict is left `git merge --continue`-able.
    assert_eq!(backend.head(&repos_app).unwrap().commit, Some(b));
    assert!(repos_app.join(".git/MERGE_HEAD").exists());
}

#[test]
pub(crate) fn pull_head_merge_rejects_unrelated_history_without_mutation() {
    let temp = TempDir::new("pull-merge-unrelated");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let fixture = RemoteFixture::new("pull-merge-unrelated-source");
    let local = fixture.commit_and_push("README.md", "local\n", "local", &backend);
    let repos_app = temp.path().join("repos/app");
    backend
        .clone_repo(fixture.remote_url(), &repos_app)
        .unwrap();
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &local)],
    );
    force_push_unrelated_main(&fixture, &backend);
    let head = backend.head(&repos_app).unwrap();
    let target_ref = backend.read_ref(&repos_app, "refs/heads/main").unwrap();
    let index = std::fs::read(repos_app.join(".git/index")).unwrap();
    let worktree = std::fs::read(repos_app.join("README.md")).unwrap();
    let status = backend.status(&repos_app).unwrap();
    let native_state = backend.merge_state(&repos_app).unwrap();
    let lock = std::fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let error = handle_pull_head(
        &backend,
        temp.path(),
        pull_head_request_with_sync(crate::SyncBehavior::Merge),
        "op_pull",
    )
    .unwrap_err();

    assert_eq!(error.code, crate::model::ErrorCode::GitCommandFailed);
    assert!(error.message.contains("do not share a merge base"));
    assert_eq!(backend.head(&repos_app).unwrap(), head);
    assert_eq!(
        backend.read_ref(&repos_app, "refs/heads/main").unwrap(),
        target_ref
    );
    assert_eq!(std::fs::read(repos_app.join(".git/index")).unwrap(), index);
    assert_eq!(
        std::fs::read(repos_app.join("README.md")).unwrap(),
        worktree
    );
    assert_eq!(backend.status(&repos_app).unwrap(), status);
    assert_eq!(backend.merge_state(&repos_app).unwrap(), native_state);
    assert_eq!(
        std::fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(!repos_app.join(".git/MERGE_HEAD").exists());
}

#[test]
pub(crate) fn pull_head_reset_discards_local_divergence() {
    // `--sync reset` on a member with a divergent local commit: throw the local work
    // away and snap onto the remote. Clean worktree ⇒ no destructive flag needed.
    let temp = TempDir::new("pull-reset");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let fixture = RemoteFixture::new("pull-reset-source");
    let a = fixture.commit_and_push("README.md", "base\n", "A", &backend);
    let repos_app = temp.path().join("repos/app");
    backend
        .clone_repo(fixture.remote_url(), &repos_app)
        .unwrap();
    let a_oid = git2::Oid::from_str(&a).unwrap();
    let b = commit_file(&repos_app, "README.md", "local\n", "B", &[a_oid]).unwrap();
    let c = fixture.commit_and_push("README.md", "remote\n", "C", &backend);
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &b)],
    );

    let response = handle_pull_head(
        &backend,
        temp.path(),
        pull_head_request_with_sync(crate::SyncBehavior::Reset),
        "op_pull",
    )
    .unwrap();

    let member = response.response.members.single();
    assert_eq!(member.status, crate::MemberStatus::Ok);
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    // Local B is discarded; the member sits exactly on the remote commit C.
    assert_eq!(backend.head(&repos_app).unwrap().commit, Some(c));
    assert_ne!(backend.head(&repos_app).unwrap().commit, Some(b));
}

#[test]
pub(crate) fn pull_head_fetches_selected_members_in_parallel() {
    let temp = TempDir::new("pull-parallel");
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let backend = TrackingBackend::new(2);
    write_pull_fixture(
        temp.path(),
        vec![
            (
                "mem_app",
                "repos/app",
                "ssh://one.invalid/app.git",
                TEST_COMMIT,
            ),
            (
                "mem_lib",
                "repos/lib",
                "ssh://two.invalid/lib.git",
                TEST_COMMIT,
            ),
        ],
    );

    let response = handle_pull_head_with_events(
        &backend,
        temp.path(),
        pull_head_request(),
        "op_pull",
        &NullSink,
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Noop
    );
    assert_eq!(backend.fetch_peak(), 2);
}

#[test]
pub(crate) fn push_runs_selected_members_in_parallel() {
    let temp = TempDir::new("push-parallel");
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let backend = TrackingBackend::new(2);
    write_pull_fixture(
        temp.path(),
        vec![
            (
                "mem_app",
                "repos/app",
                "ssh://one.invalid/app.git",
                TEST_COMMIT,
            ),
            (
                "mem_lib",
                "repos/lib",
                "ssh://two.invalid/lib.git",
                TEST_COMMIT,
            ),
        ],
    );

    let response = handle_push_with_events(
        &backend,
        temp.path(),
        push_request(None, None),
        "op_push",
        &NullSink,
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(backend.push_peak(), 2);
}

pub(crate) fn pull_head_request() -> crate::PullHeadRequest {
    crate::PullHeadRequest {
        meta: request_meta_with_workspace(),
    }
}

pub(crate) fn pull_head_request_with_sync(sync: crate::SyncBehavior) -> crate::PullHeadRequest {
    crate::PullHeadRequest {
        meta: crate::RequestMeta {
            policy: Some(crate::OperationPolicy {
                sync: Some(sync),
                ..Default::default()
            }),
            ..request_meta_with_workspace()
        },
    }
}

pub(crate) fn write_pull_fixture(root: &Path, members: Vec<(&str, &str, &str, &str)>) {
    crate::artifact::write_manifest(
        root,
        &crate::artifact::ManifestArtifact {
            schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: crate::artifact::WorkspaceHeader {
                id: "ws_ops".to_owned(),
            },
            members: members
                .iter()
                .map(
                    |(member_id, path, remote_url, _)| crate::artifact::ManifestMember {
                        id: (*member_id).to_owned(),
                        path: (*path).to_owned(),
                        source_kind: crate::artifact::ArtifactSourceKind::Git,
                        source_id: format!("src_{}", member_id.trim_start_matches("mem_")),
                        active: true,
                        desired: Some(crate::artifact::DesiredRefArtifact {
                            branch: Some("main".to_owned()),
                            ..Default::default()
                        }),
                        remotes: vec![crate::artifact::RemoteArtifact {
                            name: "origin".to_owned(),
                            url: (*remote_url).to_owned(),
                            fetch: true,
                            push: true,
                        }],
                    },
                )
                .collect(),
        },
    )
    .unwrap();
    crate::artifact::write_lock(
        root,
        &crate::artifact::LockArtifact {
            schema: crate::artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: "ws_ops".to_owned(),
            manifest_schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            members: members
                .into_iter()
                .map(|(member_id, path, _, commit)| {
                    (
                        member_id.to_owned(),
                        test_member_state(path, Some(commit.to_owned()), false),
                    )
                })
                .collect(),
        },
    )
    .unwrap();
}
