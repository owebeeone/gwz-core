pub(crate) fn test_lock(
    member_id: &str,
    path: &str,
    commit: Option<String>,
    dirty: bool,
) -> crate::artifact::LockArtifact {
    crate::artifact::LockArtifact {
        schema: crate::artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: "ws_ops".to_owned(),
        manifest_schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
        members: std::collections::BTreeMap::from([(
            member_id.to_owned(),
            test_member_state(path, commit, dirty),
        )]),
    }
}

pub(crate) fn test_member_state(
    path: &str,
    commit: Option<String>,
    dirty: bool,
) -> crate::artifact::ResolvedMemberArtifact {
    crate::artifact::ResolvedMemberArtifact {
        path: path.to_owned(),
        source_id: Some("src_app".to_owned()),
        source_kind: crate::artifact::ArtifactSourceKind::Git,
        commit,
        branch: Some("main".to_owned()),
        detached: Some(false),
        upstream: None,
        dirty: Some(dirty),
        materialized: Some(true),
    }
}

pub(crate) fn create_orphan_ref(path: &std::path::Path, ref_name: &str, content: &str) -> String {
    let repo = git2::Repository::open(path).unwrap();
    let blob = repo.blob(content.as_bytes()).unwrap();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert("unrelated.txt", blob, 0o100644).unwrap();
    let tree = repo.find_tree(builder.write().unwrap()).unwrap();
    let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
    let oid = repo
        .commit(None, &signature, &signature, "unrelated", &tree, &[])
        .unwrap();
    repo.reference(ref_name, oid, true, "test unrelated history")
        .unwrap();
    oid.to_string()
}
