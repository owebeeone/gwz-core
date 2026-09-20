use std::fs;
use std::path::Path;

use git2::{AutotagOption, FetchOptions, ObjectFormat as NativeFormat, Oid, Signature, Time};
use gwz_git::{ErrorKind, ObjectFormat, ObjectId, Repository};

fn init_bare(path: &Path, format: ObjectFormat) -> git2::Repository {
    let mut options = git2::RepositoryInitOptions::new();
    options.bare(true);
    options.initial_head("main");
    options.object_format(match format {
        ObjectFormat::Sha1 => NativeFormat::Sha1,
        ObjectFormat::Sha256 => NativeFormat::Sha256,
        _ => panic!("unsupported object format"),
    });
    git2::Repository::init_opts(path, &options).unwrap()
}

fn fixture_signature() -> Signature<'static> {
    Signature::new("Fixture", "fixture@example.invalid", &Time::new(1000, 0)).unwrap()
}

fn payload_tree(repo: &git2::Repository, blob: Oid) -> Oid {
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert("payload", blob, 0o100644).unwrap();
    builder.write().unwrap()
}

fn commit(repo: &git2::Repository, tree: Oid, parents: &[Oid], message: &str) -> Oid {
    let signature = fixture_signature();
    let tree = repo.find_tree(tree).unwrap();
    let commits = parents
        .iter()
        .map(|parent| repo.find_commit(*parent).unwrap())
        .collect::<Vec<_>>();
    let parent_refs = commits.iter().collect::<Vec<_>>();
    repo.commit(
        Some("refs/heads/main"),
        &signature,
        &signature,
        message,
        &tree,
        &parent_refs,
    )
    .unwrap()
}

fn assert_record(path: &Path, format: ObjectFormat, blob: Oid, tree: Oid, base: Oid, child: Oid) {
    let repository = Repository::open_exact(path).unwrap();
    assert_eq!(repository.object_format(), format);
    let child_id = ObjectId::parse_hex(format, &child.to_string()).unwrap();
    let base_id = ObjectId::parse_hex(format, &base.to_string()).unwrap();
    let tree_id = ObjectId::parse_hex(format, &tree.to_string()).unwrap();
    let record = repository.read_commit(child_id).unwrap();
    assert_eq!(record.id, child_id);
    assert_eq!(record.tree, tree_id);
    assert_eq!(record.parents, vec![base_id]);
    assert_eq!(record.message, b"Q3 child\nleading\n");
    assert_eq!(record.author.name, b"Fixture");
    assert_eq!(record.author.email, b"fixture@example.invalid");
    assert_eq!(record.author.seconds, 1000);
    assert_eq!(record.author.offset_minutes, 0);
    assert_eq!(record.committer, record.author);
    assert_eq!(repository.object_format(), format);
    let native = git2::Repository::open(path).unwrap();
    assert_eq!(
        native.find_blob(blob).unwrap().content(),
        b"Q3 payload\0\xff\n"
    );
}

fn qualify_fetch(root: &Path, format: ObjectFormat, label: &str) -> (Oid, Oid, Oid, Oid) {
    let source_path = root.join(format!("{label}-fetch-source"));
    let receiver_path = root.join(format!("{label}-receiver"));
    let source = init_bare(&source_path, format);
    let receiver = init_bare(&receiver_path, format);
    let blob = source.blob(b"Q3 payload\0\xff\n").unwrap();
    let tree = payload_tree(&source, blob);
    let base = commit(&source, tree, &[], "Q3 base\n");
    let url = format!("file://{}", source_path.canonicalize().unwrap().display());
    let mut remote = receiver.remote_anonymous(&url).unwrap();
    let mut options = FetchOptions::new();
    options
        .download_tags(AutotagOption::None)
        .update_fetchhead(false);
    remote
        .fetch(
            &["refs/heads/main:refs/heads/main"],
            Some(&mut options),
            None,
        )
        .unwrap();
    assert_eq!(receiver.refname_to_id("refs/heads/main").unwrap(), base);

    let shared = source.blob(b"Q3 shared hint").unwrap();
    assert_eq!(receiver.blob(b"Q3 shared hint").unwrap(), shared);
    receiver
        .reference("refs/checkpoints/hint", shared, false, "hint")
        .unwrap();
    let hint_before = receiver.refname_to_id("refs/checkpoints/hint").unwrap();
    let child = commit(&source, tree, &[base], "Q3 child\nleading\n");
    assert_eq!(
        receiver.find_commit(child).err().unwrap().code(),
        git2::ErrorCode::NotFound
    );
    if let Err(error) = remote.fetch(
        &["refs/heads/main:refs/import/wanted"],
        Some(&mut options),
        None,
    ) {
        panic!("Q3 receiver noncommit hint fetch failed: {error}");
    }
    assert_eq!(receiver.refname_to_id("refs/import/wanted").unwrap(), child);
    assert_eq!(
        receiver.refname_to_id("refs/checkpoints/hint").unwrap(),
        hint_before
    );
    (blob, tree, base, child)
}

fn assert_malformed_grafts(root: &Path, format: ObjectFormat, label: &str) {
    let path = root.join(format!("{label}-malformed-grafts"));
    let native = init_bare(&path, format);
    let metadata = native.path().join("info/grafts");
    fs::create_dir_all(metadata.parent().unwrap()).unwrap();
    drop(native);
    fs::write(&metadata, b"invalid\n").unwrap();
    let error = match Repository::open_exact(&path) {
        Ok(_) => panic!("malformed grafts unexpectedly opened"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ErrorKind::RepositoryOpen);
    let diagnostic = error.native().unwrap();
    assert_eq!(diagnostic.class, 36);
    assert_eq!(diagnostic.code, -1);
}

pub fn run(root: &Path) -> String {
    fs::create_dir(root).unwrap();
    let version = git2::Version::get();
    assert_eq!(version.libgit2_version(), (1, 9, 7));
    assert!(version.vendored());
    let mut rows = vec![format!(
        "native\t1.9.7\tvendored\t{}\t{}",
        version.https(),
        version.ssh()
    )];
    for (format, label) in [
        (ObjectFormat::Sha1, "sha1"),
        (ObjectFormat::Sha256, "sha256"),
    ] {
        let source_path = root.join(format!("{label}-source"));
        let native = init_bare(&source_path, format);
        let blob = native.blob(b"Q3 payload\0\xff\n").unwrap();
        let tree = payload_tree(&native, blob);
        let base = commit(&native, tree, &[], "Q3 base\n");
        let child = commit(&native, tree, &[base], "Q3 child\nleading\n");
        drop(native);
        assert_record(&source_path, format, blob, tree, base, child);
        let (_, _, fetched_base, fetched_child) = qualify_fetch(root, format, label);
        assert_eq!((fetched_base, fetched_child), (base, child));
        assert_malformed_grafts(root, format, label);
        rows.push(format!(
            "{label}\t{blob}\t{base}\t{child}\tfetch-ok\traw-class-36"
        ));
    }
    rows.join("\n")
}
