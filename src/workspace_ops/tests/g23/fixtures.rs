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

/// Force the one open record's `state` scalar, in place, without decoding it.
///
/// **M5d.** This was a `FileMergeStore` read/modify/write of the v0 record.
/// There is no such store any more — the v1 lifecycle owns its own writer and
/// validates every body it reads — so the fixture edits the durable YAML
/// directly. That is the right level for what these tests assert: the
/// open-merge gate classifies by ENVELOPE and never decodes the body, so a
/// forced state must not have to be a state the lifecycle would have written.
pub(super) fn force_open_merge_state(root: &Path, state: OperationState) -> String {
    let directory = root.join(".gwz/merge");
    let path = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("yaml"))
        .expect("an open merge record");
    let merge_id = path.file_stem().unwrap().to_string_lossy().into_owned();
    let text = fs::read_to_string(&path).unwrap();
    let wire = serde_yaml::to_string(&state).unwrap();
    let wire = wire.trim().trim_start_matches("- ");
    let patched = text
        .lines()
        .map(|line| {
            if line.starts_with("state:") {
                format!("state: {wire}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, format!("{patched}\n")).unwrap();
    merge_id
}

/// The one open merge record under `root`, read the way every non-merge
/// consumer reads it.
///
/// **M5d.** This replaces `FileMergeStore.discover_open`, which decoded a v0
/// body through a store that no longer exists. A pre-0.14 (v0) envelope
/// refuses here with the charter §2 sentence rather than answering `None`,
/// which is exactly what these suites want: a fixture that silently read
/// "no merge" for an occupancy would hide the defect.
pub(super) fn open_record(root: &Path) -> Option<crate::workspace_ops::merge::OpenMergeRecord> {
    crate::workspace_ops::merge::discover_open_v1_record(root).unwrap()
}

/// Whether `.gwz/merge` holds no open record at all.
pub(super) fn no_open_record(root: &Path) -> bool {
    open_record(root).is_none()
}

/// The archived (`done/`) record for `merge_id`, as the I2 §7 projection reads
/// it. Replaces `FileMergeStore.load`.
pub(super) fn archived_record(
    root: &Path,
    merge_id: &str,
) -> crate::workspace_ops::merge::ArchivedMergeRecord {
    crate::workspace_ops::merge::read_archived_record(root, merge_id).unwrap()
}

/// Read the one open record's durable YAML, let the caller edit it as a
/// `serde_yaml::Value`, and write it back.
///
/// **M5d.** The v0 suites did this with `FileMergeStore` read/modify/write.
/// The v1 store is not a general record writer — it publishes through the
/// checked door and validates every transition — so a fixture that wants to
/// stage a durable state the lifecycle would not itself produce edits the
/// bytes. Returns the merge id.
pub(super) fn patch_open_record(root: &Path, edit: impl FnOnce(&mut serde_yaml::Value)) -> String {
    let directory = root.join(".gwz/merge");
    let path = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("yaml"))
        .expect("an open merge record");
    let mut value: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
    edit(&mut value);
    fs::write(&path, serde_yaml::to_string(&value).unwrap()).unwrap();
    path.file_stem().unwrap().to_string_lossy().into_owned()
}

/// Move an archived record back into `.gwz/merge` at `state`, so the
/// open-merge gate sees an occupancy in that state.
pub(super) fn reopen_archived_record(root: &Path, merge_id: &str, state: OperationState) {
    let done = root.join(format!(".gwz/merge/done/{merge_id}.yaml"));
    let mut value: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&done).unwrap()).unwrap();
    value["state"] = serde_yaml::to_value(state).unwrap();
    fs::write(
        root.join(format!(".gwz/merge/{merge_id}.yaml")),
        serde_yaml::to_string(&value).unwrap(),
    )
    .unwrap();
    fs::remove_file(&done).unwrap();
}
