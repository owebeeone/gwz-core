use super::*;

#[test]
fn ids_parse_and_display_with_expected_prefixes() {
    let workspace: WorkspaceId = "ws_01".parse().expect("workspace id");
    let source: SourceId = "src_01".parse().expect("source id");
    let member: MemberId = "mem_01".parse().expect("member id");
    let operation: OperationId = "op_01".parse().expect("operation id");

    assert_eq!(workspace.to_string(), "ws_01");
    assert_eq!(source.to_string(), "src_01");
    assert_eq!(member.to_string(), "mem_01");
    assert_eq!(operation.to_string(), "op_01");
    assert_eq!(
        "bad id".parse::<WorkspaceId>().unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        "src_01".parse::<WorkspaceId>().unwrap_err().code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn desired_ref_accepts_exactly_one_target() {
    assert!(DesiredRef::branch("main").validate().is_ok());
    assert!(DesiredRef::git_tag("v1.0.0").validate().is_ok());
    assert!(DesiredRef::local_only().validate().is_ok());

    let invalid = DesiredRef {
        branch: Some("main".to_owned()),
        commit: Some("abc123".to_owned()),
        ..DesiredRef::default()
    };
    assert_eq!(
        invalid.validate().unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        DesiredRef::default().validate().unwrap_err().code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn attribution_requires_non_empty_actor_and_git_identity_fields() {
    let attribution = OperationAttribution {
        actor: Some(OperationActor::new("agent://local/session")),
        git_author: Some(GitObjectIdentity::new("Agent", "agent@example.invalid")),
        git_committer: Some(GitObjectIdentity::new("Bot", "bot@example.invalid")),
        credential_ref: Some("cred:test".to_owned()),
    };
    assert!(attribution.validate().is_ok());

    let invalid_actor = OperationAttribution {
        actor: Some(OperationActor::new("")),
        ..OperationAttribution::default()
    };
    assert_eq!(
        invalid_actor.validate().unwrap_err().code,
        ErrorCode::InvalidRequest
    );

    let invalid_git = OperationAttribution {
        git_author: Some(GitObjectIdentity::new("Agent", "")),
        ..OperationAttribution::default()
    };
    assert_eq!(
        invalid_git.validate().unwrap_err().code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn git_identity_rejects_values_git_signatures_cannot_represent() {
    for (field, value) in [
        ("name", "Alice <work>"),
        ("name", "Alice\0Work"),
        ("name", "Alice\nWork"),
        ("name", ",;:\"\\'"),
        ("email", "alice>example.invalid"),
        ("email", "alice\0@example.invalid"),
        ("email", "alice\r@example.invalid"),
    ] {
        let mut identity = GitObjectIdentity::new("Alice", "alice@example.invalid");
        match field {
            "name" => identity.name = value.to_owned(),
            "email" => identity.email = value.to_owned(),
            _ => unreachable!(),
        }

        let error = identity.validate().unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{field}={value:?}");
        assert!(error.message.contains(field), "{field}={value:?}: {error}");
    }
}

#[test]
fn model_error_can_carry_member_context_without_changing_its_code() {
    let error = ModelError::new(ErrorCode::GitCommandFailed, "revspec not found")
        .with_member("mem_a", "repos/a");

    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert_eq!(error.member_id.as_deref(), Some("mem_a"));
    assert_eq!(error.member_path.as_deref(), Some("repos/a"));
    assert_eq!(
        error.message,
        "member 'mem_a' at 'repos/a': revspec not found"
    );
}

#[test]
fn model_error_formats_the_workspace_root_as_a_root_target() {
    let error =
        ModelError::new(ErrorCode::MergeDrift, "post-merge work exists").with_member("@root", ".");

    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(error.member_path.as_deref(), Some("."));
    assert_eq!(
        error.message,
        "workspace root '@root' at '.': post-merge work exists"
    );
}

#[test]
fn member_spec_rejects_duplicate_remote_names() {
    let remotes = vec![
        RemoteSpec::new("origin", "git@example.invalid:one.git"),
        RemoteSpec::new("origin", "git@example.invalid:two.git"),
    ];

    let result = MemberSpec::new(
        MemberId::parse_str("mem_01").unwrap(),
        "repos/core",
        SourceId::parse_str("src_01").unwrap(),
        SourceKind::Git,
        true,
        None,
        remotes,
    );

    assert_eq!(result.unwrap_err().code, ErrorCode::InvalidRequest);
}
