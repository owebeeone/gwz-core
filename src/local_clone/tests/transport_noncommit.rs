//! Lane 1-A characterization of stock libgit2 local fetches.
//!
//! These tests call libgit2 directly before calling the current backend.  The
//! direct call records the native result; the backend call records whether its
//! existing compatibility path still completes the same requested transfer.

use std::fmt::Write as _;
use std::path::Path;

use super::fixture::{TempDir, init_repo_with_commit};
use crate::git::{Git2Backend, GitBackend};

const IMPORT_REF: &str = "refs/gwz/local-imports/characterization";

#[derive(Clone, Copy, Debug)]
enum RefKind {
    Commit,
    Tree,
    Blob,
    AnnotatedCommit,
    AnnotatedTree,
}

impl RefKind {
    const ALL: [Self; 5] = [
        Self::Commit,
        Self::Tree,
        Self::Blob,
        Self::AnnotatedCommit,
        Self::AnnotatedTree,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Tree => "tree",
            Self::Blob => "blob",
            Self::AnnotatedCommit => "annotated-commit",
            Self::AnnotatedTree => "annotated-tree",
        }
    }

    fn is_noncommit(self) -> bool {
        !matches!(self, Self::Commit)
    }

    fn source_ref(self) -> String {
        match self {
            Self::Commit => "refs/heads/main".to_owned(),
            _ => format!("refs/gwz/characterization/{}", self.label()),
        }
    }
}

#[derive(Debug)]
struct NativeFailure {
    code: String,
    class: String,
    message: String,
}

fn native_fetch(receiver: &Path, source: &Path, refspecs: &[String]) -> Result<(), NativeFailure> {
    let source = std::fs::canonicalize(source).expect("canonical source");
    let peer = url::Url::from_file_path(source)
        .expect("absolute source path")
        .to_string();
    let repository = git2::Repository::open(receiver).expect("open receiver");
    let mut remote = repository
        .remote_anonymous(&peer)
        .expect("anonymous remote");
    let mut options = git2::FetchOptions::new();
    options
        .update_fetchhead(false)
        .download_tags(git2::AutotagOption::None);
    let refs: Vec<&str> = refspecs.iter().map(String::as_str).collect();
    remote
        .fetch(&refs, Some(&mut options), Some("lane 1-A characterization"))
        .map_err(|error| NativeFailure {
            code: format!("{:?}", error.code()),
            class: format!("{:?}", error.class()),
            message: error.message().to_owned(),
        })
}

fn advance_head_with_same_tree(path: &Path) -> String {
    let repository = git2::Repository::open(path).expect("source repository");
    let parent = repository
        .head()
        .expect("source HEAD")
        .peel_to_commit()
        .expect("source parent commit");
    let signature = git2::Signature::new(
        "GWZ Characterization",
        "characterization@example.invalid",
        &git2::Time::new(1_700_000_101, 0),
    )
    .expect("signature");
    repository
        .commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            "lane 1-A advanced source",
            &parent.tree().expect("source tree"),
            &[&parent],
        )
        .expect("advance source")
        .to_string()
}

fn seed_kind(repository: &git2::Repository, kind: RefKind, commit: git2::Oid) -> String {
    if matches!(kind, RefKind::Commit) {
        return commit.to_string();
    }
    let signature = git2::Signature::new(
        "GWZ Characterization",
        "characterization@example.invalid",
        &git2::Time::new(1_700_000_100, 0),
    )
    .expect("signature");
    let tree = repository
        .find_commit(commit)
        .expect("commit")
        .tree()
        .expect("tree");
    let blob = repository
        .blob(format!("characterization {}\n", kind.label()).as_bytes())
        .expect("blob");
    let object = match kind {
        RefKind::Tree => tree.id(),
        RefKind::Blob => blob,
        RefKind::AnnotatedCommit => repository
            .tag(
                "lane-1-a-annotated-commit",
                &repository.find_object(commit, None).expect("commit object"),
                &signature,
                "lane 1-A annotated commit",
                true,
            )
            .expect("annotated commit tag"),
        RefKind::AnnotatedTree => repository
            .tag(
                "lane-1-a-annotated-tree",
                &repository
                    .find_object(tree.id(), None)
                    .expect("tree object"),
                &signature,
                "lane 1-A annotated tree",
                true,
            )
            .expect("annotated tree tag"),
        RefKind::Commit => unreachable!(),
    };
    if !matches!(kind, RefKind::AnnotatedCommit | RefKind::AnnotatedTree) {
        repository
            .reference(
                &kind.source_ref(),
                object,
                true,
                "lane 1-A noncommit characterization",
            )
            .expect("direct noncommit ref");
    }
    if matches!(kind, RefKind::AnnotatedCommit) {
        repository
            .reference(
                &kind.source_ref(),
                object,
                true,
                "lane 1-A annotated commit ref",
            )
            .expect("annotated commit ref");
    }
    if matches!(kind, RefKind::AnnotatedTree) {
        repository
            .reference(
                &kind.source_ref(),
                object,
                true,
                "lane 1-A annotated tree ref",
            )
            .expect("annotated tree ref");
    }
    object.to_string()
}

fn seed_receiver_hint(repository: &git2::Repository, kind: RefKind, commit: git2::Oid) {
    let _ = seed_kind(repository, kind, commit);
    if matches!(kind, RefKind::AnnotatedCommit | RefKind::AnnotatedTree) {
        let tag_name = match kind {
            RefKind::AnnotatedCommit => "lane-1-a-annotated-commit",
            RefKind::AnnotatedTree => "lane-1-a-annotated-tree",
            _ => unreachable!(),
        };
        repository
            .reference(
                &format!("refs/gwz/characterization/{}", kind.label()),
                repository
                    .refname_to_id(&format!("refs/tags/{tag_name}"))
                    .expect("tag ref"),
                true,
                "lane 1-A receiver hint",
            )
            .expect("receiver tag hint");
    }
}

fn read_fetch_head(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path.join(".git").join("FETCH_HEAD")).ok()
}

fn direct_refs(path: &Path) -> Vec<String> {
    git2::Repository::open(path)
        .expect("open refs")
        .references_glob("refs/gwz/local-imports/*")
        .expect("list import refs")
        .map(|reference| {
            reference
                .expect("reference")
                .name()
                .expect("reference name")
                .to_owned()
        })
        .collect()
}

fn receiver_names(path: &Path) -> (Vec<String>, Vec<String>) {
    let repository = git2::Repository::open(path).expect("open receiver state");
    let remotes = repository
        .remotes()
        .expect("list remotes")
        .iter()
        .filter_map(|name| name.ok().flatten().map(str::to_owned))
        .collect();
    let tracking = repository
        .references_glob("refs/remotes/*")
        .expect("list tracking refs")
        .filter_map(|reference| {
            reference
                .ok()
                .and_then(|reference| reference.name().ok().map(str::to_owned))
        })
        .collect();
    (remotes, tracking)
}

fn native_report(
    source_kind: RefKind,
    receiver_kind: RefKind,
    native: &Result<(), NativeFailure>,
    receiver: &Path,
) -> String {
    let (remotes, tracking) = receiver_names(receiver);
    let mut report = format!(
        "source={} receiver={} native={native:?} import_refs={:?} remotes={remotes:?} tracking={tracking:?} fetch_head={:?}",
        source_kind.label(),
        receiver_kind.label(),
        direct_refs(receiver),
        read_fetch_head(receiver),
    );
    if let Err(error) = native {
        let _ = write!(
            report,
            " native_error={{code:{}, class:{}, message:{}}}",
            error.code, error.class, error.message
        );
    }
    report
}

#[test]
fn native_and_backend_matrix_covers_requested_and_receiver_noncommit_refs() {
    assert_eq!(git2::Version::get().libgit2_version(), (1, 9, 7));
    let backend = Git2Backend::without_credential_helpers();
    let mut reports = Vec::new();
    for source_kind in RefKind::ALL {
        for receiver_kind in RefKind::ALL {
            let temp = TempDir::new(&format!(
                "noncommit-{}-{}",
                source_kind.label(),
                receiver_kind.label()
            ));
            let source = temp.path().join("source");
            let receiver = temp.path().join("receiver-native");
            let backend_receiver = temp.path().join("receiver-backend");
            let source_commit =
                init_repo_with_commit(&source, false, &format!("source-{}", source_kind.label()));
            let receiver_commit = init_repo_with_commit(
                &receiver,
                false,
                &format!("receiver-{}", receiver_kind.label()),
            );
            init_repo_with_commit(
                &backend_receiver,
                false,
                &format!("receiver-{}", receiver_kind.label()),
            );
            let source_repository = git2::Repository::open(&source).expect("source repository");
            let receiver_repository =
                git2::Repository::open(&receiver).expect("receiver repository");
            let backend_repository =
                git2::Repository::open(&backend_receiver).expect("backend receiver repository");
            let source_oid = git2::Oid::from_str(&source_commit).expect("source commit oid");
            let receiver_oid = git2::Oid::from_str(&receiver_commit).expect("receiver commit oid");
            let source_target = seed_kind(&source_repository, source_kind, source_oid);
            if receiver_kind.is_noncommit() {
                seed_receiver_hint(&receiver_repository, receiver_kind, receiver_oid);
                let backend_oid = backend_repository
                    .head()
                    .expect("backend HEAD")
                    .target()
                    .expect("backend HEAD target");
                seed_receiver_hint(&backend_repository, receiver_kind, backend_oid);
            }
            let spec = format!("+{}:{IMPORT_REF}", source_kind.source_ref());
            let native = native_fetch(&receiver, &source, std::slice::from_ref(&spec));
            let report = native_report(source_kind, receiver_kind, &native, &receiver);
            assert!(
                native.is_ok(),
                "stock libgit2 completed the matrix row: {report}"
            );
            if native.is_ok() {
                assert_eq!(
                    git2::Repository::open(&receiver)
                        .expect("receiver")
                        .refname_to_id(IMPORT_REF)
                        .ok()
                        .map(|oid| oid.to_string()),
                    Some(source_target.clone()),
                    "native route published the requested ref ({report})"
                );
            }
            backend
                .fetch_anonymous(
                    &backend_receiver,
                    &source.to_string_lossy(),
                    &[spec.as_str()],
                )
                .unwrap_or_else(|error| panic!("backend route failed: {report}; {error}"));
            assert_eq!(
                backend.read_ref(&backend_receiver, IMPORT_REF).unwrap(),
                Some(source_target),
                "backend route must complete the requested {} transfer ({})",
                source_kind.label(),
                report
            );
            reports.push(report);
        }
    }
    for report in reports {
        eprintln!("{report}");
    }
}

#[test]
fn native_multi_refspec_errors_record_partial_refs_and_fetch_head_effects() {
    let temp = TempDir::new("noncommit-partial");
    let source = temp.path().join("source");
    let receiver = temp.path().join("receiver");
    let source_commit = init_repo_with_commit(&source, false, "partial-source");
    init_repo_with_commit(&receiver, false, "partial-receiver");
    let stale = receiver.join(".git").join("FETCH_HEAD");
    std::fs::write(&stale, b"stale fetch record\n").expect("plant FETCH_HEAD");
    let valid = "+refs/heads/main:refs/gwz/local-imports/partial-first".to_owned();
    let missing = "+refs/heads/missing:refs/gwz/local-imports/partial-second".to_owned();
    let native = native_fetch(&receiver, &source, &[valid, missing]);
    eprintln!(
        "multi-refspec native={native:?} first={:?} second={:?} fetch_head={:?}",
        git2::Repository::open(&receiver)
            .expect("receiver")
            .refname_to_id("refs/gwz/local-imports/partial-first")
            .ok(),
        git2::Repository::open(&receiver)
            .expect("receiver")
            .refname_to_id("refs/gwz/local-imports/partial-second")
            .ok(),
        read_fetch_head(&receiver),
    );
    assert!(native.is_ok(), "the native route reports partial success");
    let repository = git2::Repository::open(&receiver).expect("receiver");
    let first = repository
        .refname_to_id("refs/gwz/local-imports/partial-first")
        .ok();
    let second = repository
        .refname_to_id("refs/gwz/local-imports/partial-second")
        .ok();
    assert!(
        second.is_none(),
        "missing source ref must not publish its destination"
    );
    assert_eq!(first.map(|oid| oid.to_string()), Some(source_commit));

    let malformed = native_fetch(
        &receiver,
        &source,
        &["+refs/heads/main:refs/heads/[invalid]".to_owned()],
    );
    let malformed_error = malformed.expect_err("malformed refspec must fail");
    eprintln!("malformed refspec native error={malformed_error:?}");
    assert!(
        !malformed_error.message.contains("committish"),
        "malformed refspec must remain distinguishable: {malformed_error:?}"
    );

    let corrupt = "+refs/gwz/characterization/corrupt:refs/gwz/local-imports/corrupt";
    let corrupt_ref = source.join(".git/refs/gwz/characterization/corrupt");
    std::fs::create_dir_all(corrupt_ref.parent().expect("corrupt ref parent"))
        .expect("corrupt ref directory");
    std::fs::write(&corrupt_ref, "1111111111111111111111111111111111111111\n")
        .expect("corrupt source ref");
    let corrupt_result = native_fetch(&receiver, &source, &[corrupt.to_owned()]);
    let corrupt_error = corrupt_result.expect_err("missing object must fail");
    eprintln!("corrupt object native error={corrupt_error:?}");
    assert!(
        !corrupt_error.message.contains("committish"),
        "missing object must remain distinguishable: {corrupt_error:?}"
    );
}

#[test]
fn native_same_tree_receiver_ref_reproduces_noncommittish_error() {
    let temp = TempDir::new("noncommit-shared-tree");
    let source = temp.path().join("source");
    let native_receiver = temp.path().join("receiver-native");
    let backend_receiver = temp.path().join("receiver-backend");
    let initial = init_repo_with_commit(&source, false, "shared-tree");
    assert_eq!(
        initial,
        init_repo_with_commit(&native_receiver, false, "shared-tree")
    );
    assert_eq!(
        initial,
        init_repo_with_commit(&backend_receiver, false, "shared-tree")
    );
    let source_repository = git2::Repository::open(&source).expect("source repository");
    let shared_tree = source_repository
        .head()
        .expect("source HEAD")
        .peel_to_commit()
        .expect("source commit")
        .tree()
        .expect("source tree")
        .id();
    for receiver in [&native_receiver, &backend_receiver] {
        git2::Repository::open(receiver)
            .expect("receiver repository")
            .reference(
                "refs/codex/checkpoints/tree",
                shared_tree,
                true,
                "shared tree checkpoint",
            )
            .expect("tree checkpoint");
    }
    let advanced = advance_head_with_same_tree(&source);
    let spec = format!("+refs/heads/main:{IMPORT_REF}");
    let native = native_fetch(&native_receiver, &source, std::slice::from_ref(&spec));
    eprintln!("shared-tree native={native:?} expected={advanced}");
    let native_error = native.expect_err("shared receiver tree must trigger native error");
    assert_eq!(native_error.code, "InvalidSpec");
    assert_eq!(native_error.class, "Invalid");
    assert_eq!(native_error.message, "object is not a committish");

    let backend = Git2Backend::without_credential_helpers();
    backend
        .fetch_anonymous(
            &backend_receiver,
            &source.to_string_lossy(),
            &[spec.as_str()],
        )
        .expect("backend fallback for shared receiver tree");
    assert_eq!(
        backend.read_ref(&backend_receiver, IMPORT_REF).unwrap(),
        Some(advanced)
    );
}
