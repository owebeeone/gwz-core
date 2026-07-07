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
