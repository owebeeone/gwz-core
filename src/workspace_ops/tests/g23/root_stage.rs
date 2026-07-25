use super::*;

fn conflicted_root_workspace(
    root: &Path,
    backend: &crate::git::Git2Backend,
    name: &str,
) -> PathBuf {
    let _fixture = init_one_member_workspace(root, backend, name);
    let manifest_path = root.join(crate::workspace::WORKSPACE_MANIFEST);
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    let base = commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();

    backend
        .branch_create(root, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(root, "feature/source").unwrap();
    let feature = manifest.replacen(
        "schema: gwz.workspace/v0",
        "schema: gwz.workspace/v0 # feature",
        1,
    );
    commit_file(
        root,
        crate::workspace::WORKSPACE_MANIFEST,
        &feature,
        "feature manifest",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();

    backend.switch_branch(root, "main").unwrap();
    let main = manifest.replacen(
        "schema: gwz.workspace/v0",
        "schema: gwz.workspace/v0 # main",
        1,
    );
    commit_file(
        root,
        crate::workspace::WORKSPACE_MANIFEST,
        &main,
        "main manifest",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();

    let mut merge = request(false);
    merge.meta.selection = Some(crate::Selection {
        targets: vec!["@root".to_owned()],
        ..Default::default()
    });
    let response = handle_merge(backend, root, merge, format!("op_{name}")).unwrap();
    assert_eq!(
        merge_repo(&response, "@root").state,
        crate::MergeParticipantState::Conflicted
    );
    root.join("remote")
}

#[test]
fn deleted_root_manifest_can_be_staged_from_root_without_an_explicit_workspace() {
    stage_deleted_root_manifest(false);
}

#[test]
fn deleted_root_manifest_can_be_staged_from_a_member_without_an_explicit_workspace() {
    stage_deleted_root_manifest(true);
}

fn stage_deleted_root_manifest(from_member: bool) {
    let temp = TempDir::new(if from_member {
        "merge-root-stage-member"
    } else {
        "merge-root-stage-root"
    });
    let backend = crate::git::Git2Backend::new();
    let member = conflicted_root_workspace(
        temp.path(),
        &backend,
        if from_member {
            "root_stage_member"
        } else {
            "root_stage_root"
        },
    );
    fs::remove_file(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
    let (start, cwd, pathspec) = if from_member {
        (
            member.as_path(),
            member.as_path(),
            "../gwz.conf/gwz.yml".to_owned(),
        )
    } else {
        (temp.path(), temp.path(), "gwz.conf/gwz.yml".to_owned())
    };
    let response = handle_stage(
        &backend,
        start,
        crate::StageRequest {
            meta: request_meta(),
            cwd: cwd.to_string_lossy().into_owned(),
            pathspecs: vec![pathspec],
            all: None,
        },
        "op_stage_deleted_root_manifest",
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    let manifest = backend
        .status(temp.path())
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.path == crate::workspace::WORKSPACE_MANIFEST)
        .unwrap();
    assert_eq!(manifest.index_status, "D");
}
