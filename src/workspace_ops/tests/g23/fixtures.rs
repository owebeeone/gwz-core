use super::*;

pub(super) fn request(dry_run: bool) -> crate::MergeRequest {
    let mut meta = request_meta();
    meta.dry_run = dry_run.then_some(true);
    crate::MergeRequest {
        meta,
        op: crate::MergeOp::Start,
        source_ref: Some("feature/source".to_owned()),
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum FinalizationFault {
    AfterEnteringFinalizing,
    BeforeCandidateCreation,
    AfterCandidatePersistence,
    AfterEvidenceCommit,
    AfterEvidencePersistence,
    AfterLockPublication,
    BeforeArchive,
}

pub(super) struct FaultingMergeStore {
    fault: FinalizationFault,
    pub(super) fired: Cell<bool>,
}

impl FaultingMergeStore {
    pub(super) fn new(fault: FinalizationFault) -> Self {
        Self {
            fault,
            fired: Cell::new(false),
        }
    }

    fn should_fail_write(&self, root: &Path, record: &MergeOperationRecord) -> bool {
        let Some(publication) = record.publication.as_ref() else {
            return false;
        };
        match self.fault {
            FinalizationFault::AfterEnteringFinalizing => {
                publication.step == PublicationStep::NotStarted
            }
            FinalizationFault::BeforeCandidateCreation => {
                publication.step == PublicationStep::PreparingCandidate
                    && publication.candidate.is_none()
            }
            FinalizationFault::AfterCandidatePersistence => {
                publication.step == PublicationStep::CommittingEvidence
                    && publication.candidate.is_some()
                    && publication.composition_commit.is_none()
            }
            FinalizationFault::AfterEvidenceCommit => publication.composition_commit.is_some(),
            FinalizationFault::AfterEvidencePersistence => {
                publication.step == PublicationStep::PublishingCandidate
                    && publication.composition_commit.is_some()
            }
            FinalizationFault::AfterLockPublication => {
                let actual = fs::read(root.join(crate::artifact::LOCK_PATH))
                    .ok()
                    .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
                publication.step == PublicationStep::PublishingCandidate
                    && publication.composition_commit.is_some()
                    && actual.as_deref() == publication.candidate_lock_sha256.as_deref()
            }
            FinalizationFault::BeforeArchive => false,
        }
    }

    fn inject(&self) -> ModelResult<()> {
        self.fired.set(true);
        Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!("injected {:?} failure", self.fault),
        ))
    }
}

impl MergeStore for FaultingMergeStore {
    fn discover_open(&self, root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        FileMergeStore.discover_open(root)
    }

    fn load(&self, root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
        FileMergeStore.load(root, merge_id)
    }

    fn write_open(&self, root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
        if !self.fired.get() && self.should_fail_write(root, record) {
            return self.inject();
        }
        FileMergeStore.write_open(root, record)
    }

    fn archive(&self, root: &Path, merge_id: &str) -> ModelResult<()> {
        if !self.fired.get() && matches!(self.fault, FinalizationFault::BeforeArchive) {
            return self.inject();
        }
        FileMergeStore.archive(root, merge_id)
    }
}

pub(super) fn invoke_with_store(
    backend: &crate::git::Git2Backend,
    store: &FaultingMergeStore,
    root: &Path,
    request: crate::MergeRequest,
    operation_id: &str,
) -> ModelResult<crate::MergeResponse> {
    let clock = FixedClock::new(TimestampMs(1_700_000_000_000));
    let mut ids = SequentialIdProvider::new();
    handle_merge_with_dependencies(
        MergeDependencies {
            backend,
            store,
            clock: &clock,
            ids: &mut ids,
            events: &crate::operation::NullSink,
        },
        root,
        request,
        operation_id,
    )
}

pub(super) fn feature_commit(
    backend: &crate::git::Git2Backend,
    repo: &std::path::Path,
    file: &str,
    content: &str,
) -> (String, String) {
    let base = backend.head(repo).unwrap().commit.unwrap();
    backend
        .branch_create(repo, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(repo, "feature/source").unwrap();
    let source = commit_file(
        repo,
        file,
        content,
        "source",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    backend.switch_branch(repo, "main").unwrap();
    (base, source)
}

pub(super) fn init_two_member_workspace(
    root: &std::path::Path,
    backend: &crate::git::Git2Backend,
) -> (RemoteFixture, RemoteFixture) {
    let app = RemoteFixture::new("merge-start-app");
    let lib = RemoteFixture::new("merge-start-lib");
    app.commit_and_push("README.md", "base\n", "initial", backend);
    lib.commit_and_push("README.md", "base\n", "initial", backend);
    handle_init_from_sources(
        backend,
        root,
        crate::InitFromSourcesRequest {
            meta: request_meta(),
            workspace_root: root.to_string_lossy().into_owned(),
            sources: vec![
                crate::SourceUrl {
                    url: app.remote_url().to_owned(),
                    path: Some("app".to_owned()),
                    remote_name: None,
                    branch: None,
                },
                crate::SourceUrl {
                    url: lib.remote_url().to_owned(),
                    path: Some("lib".to_owned()),
                    remote_name: None,
                    branch: None,
                },
            ],
            target: None,
            workspace_id: Some("ws_ops".to_owned()),
        },
        "op_init",
        &CollectingSink::default(),
    )
    .unwrap();
    set_identity(root);
    set_identity(&root.join("app"));
    set_identity(&root.join("lib"));
    (app, lib)
}

pub(super) struct MixedMergeFixture {
    _remotes: [RemoteFixture; 3],
    pub(super) app_before: String,
    pub(super) lib_before: String,
    pub(super) docs_before: String,
    pub(super) docs_source: String,
}

pub(super) fn init_mixed_merge_workspace(
    root: &std::path::Path,
    backend: &crate::git::Git2Backend,
) -> MixedMergeFixture {
    let app = RemoteFixture::new("merge-mixed-app");
    let lib = RemoteFixture::new("merge-mixed-lib");
    let docs = RemoteFixture::new("merge-mixed-docs");
    for fixture in [&app, &lib, &docs] {
        fixture.commit_and_push("README.md", "base\n", "initial", backend);
    }
    handle_init_from_sources(
        backend,
        root,
        crate::InitFromSourcesRequest {
            meta: request_meta(),
            workspace_root: root.to_string_lossy().into_owned(),
            sources: [(&app, "app"), (&lib, "lib"), (&docs, "docs")]
                .into_iter()
                .map(|(fixture, path)| crate::SourceUrl {
                    url: fixture.remote_url().to_owned(),
                    path: Some(path.to_owned()),
                    remote_name: None,
                    branch: None,
                })
                .collect(),
            target: None,
            workspace_id: Some("ws_ops".to_owned()),
        },
        "op_init",
        &CollectingSink::default(),
    )
    .unwrap();
    set_identity(root);
    for path in ["app", "lib", "docs"] {
        set_identity(&root.join(path));
    }

    let app_path = root.join("app");
    let lib_path = root.join("lib");
    let docs_path = root.join("docs");
    let app_before = backend.head(&app_path).unwrap().commit.unwrap();
    backend
        .branch_create(&app_path, "feature/source", "HEAD")
        .unwrap();

    let (lib_base, _) = feature_commit(backend, &lib_path, "source.txt", "source\n");
    let lib_before = commit_file(
        &lib_path,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&lib_base).unwrap()],
    )
    .unwrap();
    let (docs_base, docs_source) = feature_commit(backend, &docs_path, "README.md", "source\n");
    let docs_before = commit_file(
        &docs_path,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&docs_base).unwrap()],
    )
    .unwrap();
    MixedMergeFixture {
        _remotes: [app, lib, docs],
        app_before,
        lib_before,
        docs_before,
        docs_source,
    }
}

pub(super) fn recovery_request(
    op: crate::MergeOp,
    merge_id: Option<String>,
) -> crate::MergeRequest {
    crate::MergeRequest {
        meta: request_meta(),
        op,
        merge_id,
        ..Default::default()
    }
}

pub(super) fn merge_repo<'a>(
    response: &'a crate::MergeResponse,
    target_id: &str,
) -> &'a crate::MergeRepoSummary {
    response
        .repos
        .iter()
        .find(|repo| repo.target_id == target_id)
        .unwrap()
}

pub(super) fn workspace_file_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            let path = entry.path();
            if file_type.is_dir() {
                visit(root, &path, files);
            } else if file_type.is_file() {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

pub(super) fn assert_open_merge_blocks_all_starts_without_mutation(
    root: &Path,
    backend: &crate::git::Git2Backend,
    merge_id: &str,
) {
    let unrelated = TempDir::new("merge-gate-unrelated-cwd");
    let before = workspace_file_snapshot(root);

    for dry_run in [true, false] {
        let mut rejected = request(dry_run);
        rejected.meta.workspace = Some(crate::WorkspaceRef {
            root: Some(root.to_string_lossy().into_owned()),
            workspace_id: None,
        });

        let error = handle_merge(
            backend,
            unrelated.path(),
            rejected,
            if dry_run {
                "op_rejected_dry_run"
            } else {
                "op_rejected_real"
            },
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::OpenOperation);
        assert!(error.message.contains(merge_id));
        assert_eq!(workspace_file_snapshot(root), before);
    }
}

pub(super) fn force_open_merge_state(root: &Path, state: OperationState) -> String {
    let store = FileMergeStore;
    let mut record = store.discover_open(root).unwrap().unwrap();
    record.state = state;
    let merge_id = record.merge_id.clone();
    store.write_open(root, &record).unwrap();
    merge_id
}

pub(super) fn assert_root_evidence_abort_recovers(fault: FinalizationFault, born_root: bool) {
    let kind = if born_root { "born" } else { "unborn" };
    let temp = TempDir::new(&format!("merge-evidence-abort-{kind}-{fault:?}"));
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(
        temp.path(),
        &backend,
        &format!("merge-abort-{kind}-{fault:?}"),
    );
    let member = temp.path().join("remote");
    let (member_before, _) = feature_commit(&backend, &member, "README.md", "source\n");
    if born_root {
        backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
        let parents = backend
            .head(temp.path())
            .unwrap()
            .commit
            .into_iter()
            .map(|commit| git2::Oid::from_str(&commit).unwrap())
            .collect::<Vec<_>>();
        commit_file(
            temp.path(),
            "root-note.txt",
            "baseline\n",
            "root baseline",
            &parents,
        )
        .unwrap();
    }
    let root_before = backend.head(temp.path()).unwrap().commit;
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    fs::write(temp.path().join("staged-user.txt"), "staged\n").unwrap();
    backend
        .stage_paths(temp.path(), &["staged-user.txt"])
        .unwrap();
    fs::write(temp.path().join("staged-user.txt"), "dirty over staged\n").unwrap();
    if born_root {
        fs::write(temp.path().join("root-note.txt"), "dirty\n").unwrap();
    }
    fs::write(temp.path().join("untracked-user.txt"), "untracked\n").unwrap();
    let staged_before = git2::Repository::open(temp.path())
        .unwrap()
        .index()
        .unwrap()
        .get_path(Path::new("staged-user.txt"), 0)
        .unwrap()
        .id;

    let store = FaultingMergeStore::new(fault);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_evidence_abort",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    assert_ne!(
        backend.head(temp.path()).unwrap().commit,
        root_before,
        "{kind} {fault:?}"
    );
    assert_eq!(
        record
            .publication
            .as_ref()
            .and_then(|publication| publication.composition_commit.as_ref())
            .is_some(),
        matches!(fault, FinalizationFault::AfterEvidencePersistence),
        "{kind} {fault:?}"
    );

    let aborted = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(record.merge_id.clone())),
        "op_evidence_abort_resume",
    )
    .unwrap();
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(backend.head(temp.path()).unwrap().commit, root_before);
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(member_before.as_str())
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("staged-user.txt")).unwrap(),
        "dirty over staged\n"
    );
    if born_root {
        assert_eq!(
            fs::read_to_string(temp.path().join("root-note.txt")).unwrap(),
            "dirty\n"
        );
    }
    assert_eq!(
        fs::read_to_string(temp.path().join("untracked-user.txt")).unwrap(),
        "untracked\n"
    );
    let staged_after = git2::Repository::open(temp.path())
        .unwrap()
        .index()
        .unwrap()
        .get_path(Path::new("staged-user.txt"), 0)
        .unwrap()
        .id;
    assert_eq!(staged_after, staged_before);
    assert!(
        crate::artifact::list_markers(temp.path())
            .unwrap()
            .is_empty()
    );
}
