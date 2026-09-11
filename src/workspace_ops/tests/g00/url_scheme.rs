//! Step 2.1 of gwz-dev dev-docs/GwzUrlSchemePlan.md: the scheme preference
//! reaches the clone seam, is reported per member, is remembered per
//! workspace, and never changes the private-member or contributor paths.

use super::private_members::RefusingServer;
use super::*;
use crate::git::UrlScheme;
use crate::model::ModelResult;
use crate::workspace_ops::url_scheme_state::{
    EffectiveUrlScheme, URL_SCHEME_STATE_PATH, UrlSchemeSource, append_url_scheme_hint,
    resolve_root_url, url_scheme_refusal_error,
};

fn https_request() -> crate::RequestMeta {
    crate::RequestMeta {
        transport: Some(crate::TransportOptions {
            default_identity: None,
            remote_identities: vec![],
            url_scheme: Some(crate::UrlScheme::Https),
        }),
        ..request_meta()
    }
}

fn manifest_request() -> crate::RequestMeta {
    crate::RequestMeta {
        transport: Some(crate::TransportOptions {
            default_identity: None,
            remote_identities: vec![],
            url_scheme: Some(crate::UrlScheme::Manifest),
        }),
        ..request_meta()
    }
}

fn materialize_with(root: &Path, meta: crate::RequestMeta) -> ModelResult<crate::MaterializeResponse> {
    let mut request = materialize_lock_request(false);
    request.meta = meta;
    handle_materialize(&Git2Backend::new(), root, request, "materialize", &NullSink)
}

#[test]
fn preflight_derives_known_host_clone_url() {
    let temp = TempDir::new("url-scheme-preflight");
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    write_materialize_fixture(temp.path(), "git@github.com:owebeeone/demo.git", "deadbeef");
    let manifest = read_manifest(temp.path()).unwrap();
    let lock = crate::artifact::read_lock(temp.path()).unwrap();
    let scheme = EffectiveUrlScheme {
        scheme: UrlScheme::Https,
        source: UrlSchemeSource::Request,
    };
    let plans = materialize_preflight(
        &Git2Backend::new(),
        temp.path(),
        &manifest,
        &lock,
        &["mem_app".to_owned()],
        false,
        scheme,
    )
    .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].clone_url.as_deref(),
        Some("https://github.com/owebeeone/demo.git")
    );
    let resolution = plans[0].response.url_resolution.as_ref().unwrap();
    assert_eq!(resolution.manifest_url, "git@github.com:owebeeone/demo.git");
    assert_eq!(resolution.effective_url, "https://github.com/owebeeone/demo.git");
    assert_eq!(resolution.scheme, crate::UrlScheme::Https);
    assert_eq!(resolution.source, crate::UrlSchemeSource::Request);
    assert!(resolution.derived);
    assert!(resolution.host_known);
    // Nothing was cloned by planning alone.
    assert!(!temp.path().join("repos/app").exists());
}

#[test]
fn preflight_refuses_an_underivable_known_host_url_before_any_clone() {
    let temp = TempDir::new("url-scheme-refusal");
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    write_materialize_fixture(temp.path(), "ssh://git@github.com:2222/o/r.git", "deadbeef");
    let error = materialize_with(temp.path(), https_request()).unwrap_err();
    assert_eq!(error.code, ErrorCode::UrlSchemeUnavailable);
    assert_eq!(error.member_id.as_deref(), Some("mem_app"));
    assert_eq!(error.member_path.as_deref(), Some("repos/app"));
    assert!(error.message.contains("2222"), "{}", error.message);
    assert!(error.message.contains("--url-scheme manifest"), "{}", error.message);
    assert!(!temp.path().join("repos/app").exists());
    assert!(!temp.path().join(URL_SCHEME_STATE_PATH).exists());
}

#[test]
fn materialize_under_https_passes_an_unknown_host_through_and_records_the_preference() {
    let temp = TempDir::new("url-scheme-passthrough");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    let remote = RemoteFixture::new("url-scheme-passthrough-source");
    let commit = remote.commit_and_push("README.md", "hello", "initial", &backend);
    write_materialize_fixture(temp.path(), remote.remote_url(), &commit);

    let response = materialize_with(temp.path(), https_request()).unwrap();
    assert_eq!(response.response.meta.aggregate_status, crate::AggregateStatus::Ok);
    let member = &response.response.members[0];
    let resolution = member.url_resolution.as_ref().expect("cloned member reports its URL");
    assert_eq!(resolution.manifest_url, remote.remote_url());
    assert_eq!(resolution.effective_url, remote.remote_url());
    assert_eq!(resolution.scheme, crate::UrlScheme::Https);
    assert_eq!(resolution.source, crate::UrlSchemeSource::Request);
    assert!(!resolution.derived);
    assert!(!resolution.host_known);
    assert!(temp.path().join("repos/app/README.md").exists());
    let recorded = fs::read_to_string(temp.path().join(URL_SCHEME_STATE_PATH)).unwrap();
    assert!(recorded.contains("schema: gwz.url-scheme/v1"), "{recorded}");
    assert!(recorded.contains("scheme: https"), "{recorded}");
    assert!(recorded.contains("recorded_by: materialize"), "{recorded}");

    // A member that is already checked out is not cloned and carries no resolution.
    let again = materialize_with(temp.path(), request_meta()).unwrap();
    assert!(again.response.members[0].url_resolution.is_none());

    // With nothing requested, the recorded preference applies to the next clone.
    fs::remove_dir_all(temp.path().join("repos/app")).unwrap();
    let remembered = materialize_with(temp.path(), request_meta()).unwrap();
    let resolution = remembered.response.members[0].url_resolution.as_ref().unwrap();
    assert_eq!(resolution.scheme, crate::UrlScheme::Https);
    assert_eq!(resolution.source, crate::UrlSchemeSource::Workspace);

    // An explicit `manifest` clears the record and reports itself.
    fs::remove_dir_all(temp.path().join("repos/app")).unwrap();
    let cleared = materialize_with(temp.path(), manifest_request()).unwrap();
    let resolution = cleared.response.members[0].url_resolution.as_ref().unwrap();
    assert_eq!(resolution.scheme, crate::UrlScheme::Manifest);
    assert_eq!(resolution.source, crate::UrlSchemeSource::Request);
    assert!(!temp.path().join(URL_SCHEME_STATE_PATH).exists());

    // With nothing requested and nothing recorded, the default is reported.
    fs::remove_dir_all(temp.path().join("repos/app")).unwrap();
    let plain = materialize_with(temp.path(), request_meta()).unwrap();
    let resolution = plain.response.members[0].url_resolution.as_ref().unwrap();
    assert_eq!(resolution.scheme, crate::UrlScheme::Manifest);
    assert_eq!(resolution.source, crate::UrlSchemeSource::Default);
    assert!(!temp.path().join(URL_SCHEME_STATE_PATH).exists());
}

#[test]
fn an_unreadable_preference_file_is_refused_and_manifest_clears_it() {
    let temp = TempDir::new("url-scheme-unreadable");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    let remote = RemoteFixture::new("url-scheme-unreadable-source");
    let commit = remote.commit_and_push("README.md", "hello", "initial", &backend);
    write_materialize_fixture(temp.path(), remote.remote_url(), &commit);
    let state = temp.path().join(URL_SCHEME_STATE_PATH);
    fs::create_dir_all(state.parent().unwrap()).unwrap();
    fs::write(&state, "schema: gwz.url-scheme/v1\nscheme: auto\n").unwrap();

    let error = materialize_with(temp.path(), request_meta()).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("url-scheme.yml"), "{}", error.message);
    assert!(error.message.contains("--url-scheme manifest"), "{}", error.message);
    assert!(!temp.path().join("repos/app").exists());

    let cleared = materialize_with(temp.path(), manifest_request()).unwrap();
    assert_eq!(cleared.response.meta.aggregate_status, crate::AggregateStatus::Ok);
    assert!(!state.exists());
}

#[test]
fn clone_without_a_request_reports_the_manifest_scheme_and_records_nothing() {
    let temp = TempDir::new("url-scheme-clone-default");
    let backend = Git2Backend::new();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    handle_create_workspace(create_workspace_request(&source), "create").unwrap();
    let remote = RemoteFixture::new("url-scheme-clone-default-source");
    let commit = remote.commit_and_push("README.md", "public", "initial", &backend);
    write_materialize_fixture(&source, remote.remote_url(), &commit);
    commit_workspace_root(&source);
    let target = temp.path().join("target");
    let response = handle_clone_workspace(
        &backend,
        request_meta(),
        source.to_str().unwrap(),
        target.to_str().unwrap(),
        "clone",
        &NullSink,
    )
    .unwrap();
    assert_eq!(response.response.meta.aggregate_status, crate::AggregateStatus::Ok);
    let resolution = response.response.members[0].url_resolution.as_ref().unwrap();
    assert_eq!(resolution.manifest_url, remote.remote_url());
    assert_eq!(resolution.effective_url, remote.remote_url());
    assert_eq!(resolution.scheme, crate::UrlScheme::Manifest);
    assert_eq!(resolution.source, crate::UrlSchemeSource::Default);
    assert!(!resolution.derived);
    assert!(!resolution.host_known);
    assert!(response.response.meta.message.is_none());
    assert!(!target.join(URL_SCHEME_STATE_PATH).exists());
}

#[test]
fn clone_under_https_keeps_a_private_refusal_quiet_and_records_the_preference() {
    let temp = TempDir::new("url-scheme-clone-private");
    let backend = Git2Backend::new();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    handle_create_workspace(create_workspace_request(&source), "create").unwrap();
    let remote = RemoteFixture::new("url-scheme-clone-private-public");
    let commit = remote.commit_and_push("README.md", "public", "initial", &backend);
    let server = RefusingServer::new(401);
    write_pull_fixture(
        &source,
        vec![
            ("mem_app", "repos/app", remote.remote_url(), &commit),
            ("mem_secret", "repos/secret", &server.url, &commit),
        ],
    );
    let mut manifest = read_manifest(&source).unwrap();
    manifest.members[1].private = true;
    crate::artifact::write_manifest(&source, &manifest).unwrap();
    commit_workspace_root(&source);

    let target = temp.path().join("target");
    let events = CollectingSink::default();
    let response = handle_clone_workspace(
        &backend,
        https_request(),
        source.to_str().unwrap(),
        target.to_str().unwrap(),
        "clone",
        &events,
    )
    .unwrap();
    assert_eq!(response.response.meta.aggregate_status, crate::AggregateStatus::Ok);
    assert_eq!(response.response.members.len(), 1);
    assert_eq!(response.response.members[0].member_id, "mem_app");
    let resolution = response.response.members[0].url_resolution.as_ref().unwrap();
    assert_eq!(resolution.scheme, crate::UrlScheme::Https);
    assert!(!resolution.derived);
    assert!(target.join("repos/app/README.md").exists());
    assert!(!target.join("repos/secret").exists());
    let rendered = format!("{response:?} {:?}", events.take());
    assert!(!rendered.contains("mem_secret"), "{rendered}");
    let recorded = fs::read_to_string(target.join(URL_SCHEME_STATE_PATH)).unwrap();
    assert!(recorded.contains("scheme: https"), "{recorded}");
    assert!(recorded.contains("recorded_by: clone"), "{recorded}");
}

#[test]
fn a_remote_identity_override_conflicts_with_https_before_any_network() {
    let temp = TempDir::new("url-scheme-identity-conflict");
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    write_materialize_fixture(temp.path(), "git@github.com:o/r.git", "deadbeef");
    let key = temp.path().join("key");
    fs::write(&key, "fixture: never read as a key").unwrap();
    let mut request = materialize_lock_request(false);
    request.meta.transport = Some(crate::TransportOptions {
        default_identity: None,
        remote_identities: vec![crate::RemoteSshIdentity {
            remote: "origin".to_owned(),
            private_key_path: key.to_str().unwrap().to_owned(),
        }],
        url_scheme: Some(crate::UrlScheme::Https),
    });
    let error = handle_materialize(&Git2Backend::new(), temp.path(), request, "materialize", &NullSink)
        .unwrap_err();
    assert!(
        error.message.contains("non-SSH destination"),
        "{}",
        error.message
    );
    assert!(!temp.path().join("repos/app").exists());
}

#[test]
fn hints_name_the_https_remedy_only_for_ssh_forms_on_known_hosts() {
    let identity = "SSH key authentication failed (no usable identity in the ssh-agent); run `ssh-add` or check your SSH setup".to_owned();
    let hinted = append_url_scheme_hint(identity.clone(), "git@github.com:o/r.git", UrlScheme::Manifest);
    assert!(hinted.starts_with(&identity));
    assert!(hinted.ends_with("retry with --url-scheme https or set GWZ_URL_SCHEME=https"), "{hinted}");
    assert_eq!(
        append_url_scheme_hint(identity.clone(), "git@github.com:o/r.git", UrlScheme::Https),
        identity
    );
    assert_eq!(
        append_url_scheme_hint(identity.clone(), "git@example.com:o/r.git", UrlScheme::Manifest),
        identity
    );
    assert_eq!(
        append_url_scheme_hint(identity.clone(), "https://github.com/o/r.git", UrlScheme::Manifest),
        identity
    );
    let hostkey = "invalid or unknown remote ssh hostkey".to_owned();
    let hinted = append_url_scheme_hint(hostkey.clone(), "ssh://git@gitlab.com/o/r.git", UrlScheme::Ssh);
    assert!(hinted.contains("ssh -T git@gitlab.com"), "{hinted}");
    assert!(hinted.ends_with("retry with --url-scheme https"), "{hinted}");
    let other = "unexpected http status code: 500".to_owned();
    assert_eq!(append_url_scheme_hint(other.clone(), "git@github.com:o/r.git", UrlScheme::Ssh), other);
}

#[test]
fn the_root_url_is_derived_before_the_root_clone() {
    let https = EffectiveUrlScheme {
        scheme: UrlScheme::Https,
        source: UrlSchemeSource::Request,
    };
    let derived = resolve_root_url("git@github.com:o/ws.git", https).unwrap();
    assert_eq!(derived.effective_url, "https://github.com/o/ws.git");
    assert!(derived.derived);
    let local = resolve_root_url("/tmp/some/workspace", https).unwrap();
    assert_eq!(local.effective_url, "/tmp/some/workspace");
    assert!(!local.derived);
    let refused = resolve_root_url("http://github.com/o/ws.git", https).unwrap_err();
    assert_eq!(refused.code, ErrorCode::UrlSchemeUnavailable);
    assert!(refused.member_id.is_none());
    assert!(refused.message.contains("workspace root"), "{}", refused.message);
    let untouched = resolve_root_url("git@github.com:o/ws.git", EffectiveUrlScheme::MANIFEST).unwrap();
    assert_eq!(untouched.effective_url, "git@github.com:o/ws.git");
}

#[test]
fn a_refusal_error_carries_member_context() {
    let refusal = crate::git::derive("git@github.com:", UrlScheme::Https).unwrap_err();
    let error = url_scheme_refusal_error(&refusal, Some(("mem_x", "repos/x")));
    assert_eq!(error.code, ErrorCode::UrlSchemeUnavailable);
    assert_eq!(error.member_id.as_deref(), Some("mem_x"));
    assert_eq!(error.member_path.as_deref(), Some("repos/x"));
    assert!(error.message.starts_with("member 'mem_x' (repos/x): "), "{}", error.message);
    assert!(error.message.contains("empty repository path"), "{}", error.message);
}

#[test]
fn repo_sync_keeps_a_manifest_url_that_differs_only_by_scheme_unless_forced() {
    let temp = TempDir::new("url-scheme-sync");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    handle_create_repo(
        &backend,
        temp.path(),
        create_repo_request("repos/app", None, None),
        "member",
    )
    .unwrap();
    let mut manifest = read_manifest(temp.path()).unwrap();
    manifest.members[0].remotes = vec![crate::artifact::RemoteArtifact {
        name: "origin".to_owned(),
        url: "git@github.com:o/r.git".to_owned(),
        fetch: true,
        push: true,
    }];
    crate::artifact::write_manifest(temp.path(), &manifest).unwrap();
    backend
        .add_remote(&temp.path().join("repos/app"), "origin", "https://github.com/o/r.git")
        .unwrap();

    let response = handle_repo_sync(
        &backend,
        temp.path(),
        crate::RepoSyncRequest {
            private: None,
            meta: request_meta(),
        },
        "sync",
    )
    .unwrap();
    let member = &response.response.members[0];
    let resolution = member.url_resolution.as_ref().expect("scheme-only drift is reported");
    assert_eq!(resolution.manifest_url, "git@github.com:o/r.git");
    assert_eq!(resolution.effective_url, "https://github.com/o/r.git");
    assert_eq!(resolution.scheme, crate::UrlScheme::Https);
    assert!(resolution.derived);
    assert_eq!(
        read_manifest(temp.path()).unwrap().members[0].remotes[0].url,
        "git@github.com:o/r.git",
        "the manifest keeps the recorded URL"
    );
    let message = response.response.meta.message.clone().unwrap_or_default();
    assert!(message.contains("only by URL scheme"), "{message}");
    assert!(message.contains("--force"), "{message}");

    let forced = handle_repo_sync(
        &backend,
        temp.path(),
        crate::RepoSyncRequest {
            private: None,
            meta: request_meta_with_force(),
        },
        "sync",
    )
    .unwrap();
    assert!(forced.response.members[0].url_resolution.is_none());
    assert_eq!(
        read_manifest(temp.path()).unwrap().members[0].remotes[0].url,
        "https://github.com/o/r.git",
        "--force records the configured form"
    );

    // A remote that names a different repository is synced as before.
    backend
        .add_remote(&temp.path().join("repos/app"), "upstream", "https://github.com/o/other.git")
        .unwrap();
    let other = handle_repo_sync(
        &backend,
        temp.path(),
        crate::RepoSyncRequest {
            private: None,
            meta: request_meta(),
        },
        "sync",
    )
    .unwrap();
    assert!(other.response.members[0].url_resolution.is_none());
    assert_eq!(read_manifest(temp.path()).unwrap().members[0].remotes.len(), 2);
}
