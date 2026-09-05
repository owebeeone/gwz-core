//! `local_clone::tests::transport`: the anonymous local ports and the
//! `LocalTransport` adapter over the real Git backend.

use gwz_local_import::{LocalTransport, SourceSelector, TransportError};

use super::fixture::{TempDir, init_repo_with_commit};
use crate::git::{Git2Backend, GitBackend};
use crate::local_clone::transport::{BackendLocalTransport, object_id_from_hex};
use crate::model::ErrorCode;

const IMPORT_REF: &str = "refs/gwz/local-imports/t1";
/// A user's earlier fetch record, planted so the FETCH_HEAD assertion below
/// can tell "untouched" from "never existed".
const STALE_FETCH_HEAD: &[u8] =
    b"0000000000000000000000000000000000000000\t\tbranch 'stale' of earlier\n";

#[test]
fn fetch_anonymous_imports_an_explicit_refspec_without_persisting_a_remote() {
    let temp = TempDir::new("fetch-anon");
    let source = temp.path().join("source");
    let receiver = temp.path().join("receiver");
    let source_commit = init_repo_with_commit(&source, false, "source");
    let _receiver_commit = init_repo_with_commit(&receiver, false, "receiver");
    let backend = Git2Backend::without_credential_helpers();
    let fetch_head = receiver.join(".git").join("FETCH_HEAD");
    assert!(!fetch_head.exists(), "a fresh receiver has no FETCH_HEAD");
    std::fs::write(&fetch_head, STALE_FETCH_HEAD).unwrap();

    let result = backend
        .fetch_anonymous(
            &receiver,
            &source.to_string_lossy(),
            &[&format!("+refs/heads/main:{IMPORT_REF}")],
        )
        .expect("anonymous local fetch");
    assert_eq!(result.remote, source.to_string_lossy());
    assert_eq!(
        backend.read_ref(&receiver, IMPORT_REF).unwrap().as_deref(),
        Some(source_commit.as_str()),
        "the import ref holds the source commit"
    );
    assert!(
        backend.remotes(&receiver).unwrap().is_empty(),
        "no named remote is persisted"
    );
    // LCM1.0c-rem1 (State P2-1 / Code P3-4): `FETCH_HEAD` is MEASURED, not
    // described. The record planted before the fetch distinguishes "left
    // alone" from "absent because nothing was ever there". Measured on
    // libgit2 1.9.7: `update_fetchhead(false)` stops the fetch RECORD from
    // being written, but `git_remote_update_tips` truncates the file to
    // empty on every fetch regardless (`remote.c`, `truncate_fetch_head`,
    // creating it when absent -- pinned in
    // `adapter_maps_every_port_method_onto_the_backend`). So a user's prior
    // fetch record does not survive this port; the contract says so and
    // keeps `FETCH_HEAD` outside its promise. A libgit2 change in either
    // direction fails here instead of quietly invalidating that sentence.
    assert_eq!(
        std::fs::read(&fetch_head).unwrap(),
        b"",
        "libgit2 truncates FETCH_HEAD on every fetch and writes no record into it"
    );
    assert!(
        backend
            .read_ref(&receiver, "refs/tags/v1")
            .unwrap()
            .is_none(),
        "no tag is followed"
    );
    // The receiver's own branch is untouched.
    assert_ne!(
        backend
            .read_ref(&receiver, "refs/heads/main")
            .unwrap()
            .as_deref(),
        Some(source_commit.as_str())
    );
}

#[test]
fn anonymous_ports_refuse_non_local_peers_and_empty_refspecs_before_any_effect() {
    let temp = TempDir::new("fetch-anon-refuse");
    let receiver = temp.path().join("receiver");
    init_repo_with_commit(&receiver, false, "receiver");
    let backend = Git2Backend::without_credential_helpers();

    for peer in [
        "https://example.invalid/repo.git",
        "ssh://git@example.invalid/repo.git",
        "git@example.invalid:org/repo.git",
        "file:///tmp/anything",
    ] {
        let error = backend
            .fetch_anonymous(
                &receiver,
                peer,
                &[&format!("+refs/heads/main:{IMPORT_REF}")],
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{peer}");
        let error = backend
            .push_anonymous(&receiver, peer, "refs/heads/main:refs/heads/main")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{peer}");
    }
    let missing = temp.path().join("absent");
    let error = backend
        .fetch_anonymous(
            &receiver,
            &missing.to_string_lossy(),
            &[&format!("+refs/heads/main:{IMPORT_REF}")],
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    let source = temp.path().join("source");
    init_repo_with_commit(&source, false, "source");
    let error = backend
        .fetch_anonymous(&receiver, &source.to_string_lossy(), &[])
        .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::InvalidRequest,
        "explicit refspecs are required"
    );
    assert!(backend.read_ref(&receiver, IMPORT_REF).unwrap().is_none());
    assert!(backend.remotes(&receiver).unwrap().is_empty());
}

#[test]
fn push_anonymous_updates_a_bare_receiver_and_reports_a_rejection_as_an_error() {
    let temp = TempDir::new("push-anon");
    let source = temp.path().join("source");
    let hub = temp.path().join("hub");
    let source_commit = init_repo_with_commit(&source, false, "source");
    init_repo_with_commit(&hub, true, "hub");
    let backend = Git2Backend::without_credential_helpers();

    let result = backend
        .push_anonymous(
            &source,
            &hub.to_string_lossy(),
            "refs/heads/main:refs/heads/lane/from-A",
        )
        .expect("anonymous local push");
    assert_eq!(result.refspec, "refs/heads/main:refs/heads/lane/from-A");
    assert_eq!(
        backend
            .read_ref(&hub, "refs/heads/lane/from-A")
            .unwrap()
            .as_deref(),
        Some(source_commit.as_str())
    );
    assert!(backend.remotes(&source).unwrap().is_empty());
    assert_ne!(
        backend
            .read_ref(&hub, "refs/heads/main")
            .unwrap()
            .as_deref(),
        Some(source_commit.as_str()),
        "the hub's own main is untouched"
    );

    // A non-fast-forward update of an existing branch without `+` is
    // rejected by the receiving side; the port reports it as an error and
    // the ref is unchanged.
    let hub_main_before = backend.read_ref(&hub, "refs/heads/main").unwrap();
    let error = backend
        .push_anonymous(
            &source,
            &hub.to_string_lossy(),
            "refs/heads/main:refs/heads/main",
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RemoteRejected, "{}", error.message);
    assert_eq!(
        backend.read_ref(&hub, "refs/heads/main").unwrap(),
        hub_main_before
    );
}

#[test]
fn push_anonymous_into_a_non_bare_receiver_is_refused_by_libgit2() {
    // Design §6.1 requires the family push wrapper (lane X) to refuse a push
    // into a checked-out branch. This records what the raw port does so the
    // wrapper is built on an observation, not an assumption: libgit2's local
    // transport refuses EVERY push into a non-bare repository ("local push
    // doesn't (yet) support pushing to non-bare repos"), reported here as
    // `git_command_failed` with the receiver unchanged. A family push
    // therefore reaches a bare hub only; publishing into a checkout member
    // needs the receiver-side fetch form of the same port, and the
    // checked-out-branch protection belongs to the wrapper either way.
    let temp = TempDir::new("push-anon-non-bare");
    let source = temp.path().join("source");
    let receiver = temp.path().join("receiver");
    init_repo_with_commit(&source, false, "source");
    init_repo_with_commit(&receiver, false, "receiver");
    let backend = Git2Backend::without_credential_helpers();
    let before = backend.read_ref(&receiver, "refs/heads/main").unwrap();
    let error = backend
        .push_anonymous(
            &source,
            &receiver.to_string_lossy(),
            "+refs/heads/main:refs/heads/lane",
        )
        .expect_err("libgit2 refuses a local push into a non-bare repository");
    assert_eq!(error.code, ErrorCode::GitCommandFailed, "{}", error.message);
    assert!(error.message.contains("non-bare"), "{}", error.message);
    assert_eq!(
        backend.read_ref(&receiver, "refs/heads/main").unwrap(),
        before
    );
    assert!(
        backend
            .read_ref(&receiver, "refs/heads/lane")
            .unwrap()
            .is_none()
    );
}

/// LCM1.0c-rem1, Code P2-1. libgit2 consults its transport table by URL
/// prefix first and only then by heuristic, and on non-Windows hosts the
/// heuristic treats ANY `:` in a bare string as scp-style SSH syntax before it
/// tests for a directory (`transport.c`, `transport_find_fn`); this crate
/// builds libgit2 with SSH support, so a bare path under a `:`-containing
/// directory -- ordinary on macOS -- left the local transport. The port hands
/// libgit2 a canonical `file://` URL for the admitted directory instead, so
/// both transfers stay local whatever the path contains. Windows spells `:`
/// only in drive letters and tests for a directory first, so the case is
/// unix-only.
#[cfg(unix)]
#[test]
fn anonymous_ports_stay_local_for_a_peer_path_containing_a_colon() {
    let temp = TempDir::new("peer:colon");
    assert!(
        temp.path().to_string_lossy().contains(':'),
        "the fixture directory carries the `:` under test: {}",
        temp.path().display()
    );
    let source = temp.path().join("source");
    let receiver = temp.path().join("receiver");
    let hub = temp.path().join("hub");
    let source_commit = init_repo_with_commit(&source, false, "source");
    init_repo_with_commit(&receiver, false, "receiver");
    init_repo_with_commit(&hub, true, "hub");
    let backend = Git2Backend::without_credential_helpers();

    let fetched = backend
        .fetch_anonymous(
            &receiver,
            &source.to_string_lossy(),
            &[&format!("+refs/heads/main:{IMPORT_REF}")],
        )
        .expect("a `:` in the peer path selects the local transport, not SSH");
    assert_eq!(fetched.remote, source.to_string_lossy());
    assert_eq!(
        backend.read_ref(&receiver, IMPORT_REF).unwrap().as_deref(),
        Some(source_commit.as_str()),
        "the import ref holds the source commit"
    );

    let pushed = backend
        .push_anonymous(
            &source,
            &hub.to_string_lossy(),
            "refs/heads/main:refs/heads/lane/from-A",
        )
        .expect("a `:` in the peer path pushes through the local transport");
    assert_eq!(pushed.remote, hub.to_string_lossy());
    assert_eq!(
        backend
            .read_ref(&hub, "refs/heads/lane/from-A")
            .unwrap()
            .as_deref(),
        Some(source_commit.as_str()),
        "the hub branch holds the source commit"
    );
    assert!(backend.remotes(&receiver).unwrap().is_empty());
    assert!(backend.remotes(&source).unwrap().is_empty());
}

#[test]
fn adapter_maps_every_port_method_onto_the_backend() {
    let temp = TempDir::new("adapter");
    let source = temp.path().join("source");
    let receiver = temp.path().join("receiver");
    let hub = temp.path().join("hub");
    let source_commit = init_repo_with_commit(&source, false, "source");
    init_repo_with_commit(&receiver, false, "receiver");
    init_repo_with_commit(&hub, true, "hub");
    let backend = Git2Backend::without_credential_helpers();
    let mut transport = BackendLocalTransport::new(&backend);

    let head = transport
        .resolve_source(&source, &SourceSelector::Head)
        .unwrap();
    assert_eq!(head.to_hex(), source_commit);
    let by_ref = transport
        .resolve_source(&source, &SourceSelector::Ref("refs/heads/main".to_owned()))
        .unwrap();
    assert_eq!(by_ref, head);
    assert!(matches!(
        transport.resolve_source(&source, &SourceSelector::Ref("refs/heads/nope".to_owned())),
        Err(TransportError::Repository { .. })
    ));
    assert_eq!(transport.ref_exists(&receiver, IMPORT_REF), Ok(false));
    let fetch_head = receiver.join(".git").join("FETCH_HEAD");
    assert!(!fetch_head.exists());
    transport
        .fetch_anonymous(
            &receiver,
            &source,
            &[format!("+refs/heads/main:{IMPORT_REF}")],
        )
        .unwrap();
    assert_eq!(transport.ref_exists(&receiver, IMPORT_REF), Ok(true));
    // The absent-before arm of the FETCH_HEAD measurement (see the port test
    // above): libgit2 creates the file empty when the receiver had none.
    assert_eq!(
        std::fs::read(&fetch_head).unwrap(),
        b"",
        "an absent FETCH_HEAD is created empty by libgit2's fetch"
    );
    assert_eq!(
        transport.read_ref(&receiver, IMPORT_REF),
        Ok(Some(head.clone()))
    );
    assert_eq!(transport.read_ref(&receiver, "refs/heads/nope"), Ok(None));
    transport
        .push_anonymous(&source, &hub, "refs/heads/main:refs/heads/lane")
        .unwrap();
    assert_eq!(transport.read_ref(&hub, "refs/heads/lane"), Ok(Some(head)));
    assert!(matches!(
        transport.fetch_anonymous(
            &receiver,
            &temp.path().join("absent"),
            &["+refs/heads/main:refs/x".to_owned()]
        ),
        Err(TransportError::NotLocal { .. })
    ));
    assert!(matches!(
        transport.push_anonymous(&source, &hub, "refs/heads/main:refs/heads/main"),
        Err(TransportError::Rejected { .. })
    ));
}

#[test]
fn object_ids_infer_their_format_from_hex_length() {
    assert_eq!(
        object_id_from_hex(&"ab".repeat(20)).unwrap().format(),
        gwz_repo_contract::ObjectFormat::Sha1
    );
    assert_eq!(
        object_id_from_hex(&"ab".repeat(32)).unwrap().format(),
        gwz_repo_contract::ObjectFormat::Sha256
    );
    assert!(object_id_from_hex("abc").is_err());
    assert!(object_id_from_hex(&"zz".repeat(20)).is_err());
}
