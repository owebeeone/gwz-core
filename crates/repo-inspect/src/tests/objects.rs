//! `LocalObjectReader` against the contract's own `ObjectReader`
//! conformance suite, plus the edges `gwz-history-check`'s traversal needs.
//!
//! The suite compares the reader's answers with a `GraphFixture` whose
//! expectations are built here with `git2` directly — sizes come from the
//! object headers, ids from the objects the fixture wrote — so the reader is
//! never its own oracle.

use git2::ObjectFormat;
use gwz_repo_contract::contract_tests::{GraphFixture, object_reader_conformance};
use gwz_repo_contract::{
    ObjectFormat as ContractFormat, ObjectId, ObjectKind, ObjectReader, ObjectRecord,
    ProtectedRoot, ProtectedRoots, ReadError, ReadLimits, RootSource,
};

use super::admitted;
use crate::fixtures::Fixture;
use crate::{LocalObjectReader, normalise_roots};

fn contract_format(format: ObjectFormat) -> ContractFormat {
    match format {
        ObjectFormat::Sha256 => ContractFormat::Sha256,
        _ => ContractFormat::Sha1,
    }
}

fn id(format: ObjectFormat, oid: git2::Oid) -> ObjectId {
    ObjectId::from_bytes(contract_format(format), oid.as_bytes()).expect("fixture id")
}

/// The commit / tree / blob a one-commit fixture holds, with the sizes its
/// own object headers report and the two roots such a repository has.
fn expectations(fixture: &Fixture, format: ObjectFormat) -> GraphFixture {
    let repository = fixture.open();
    let odb = repository.odb().expect("odb");
    let commit_oid = fixture.head_commit();
    let commit = repository.find_commit(commit_oid).expect("commit");
    let tree_oid = commit.tree_id();
    let tree = repository.find_tree(tree_oid).expect("tree");
    let blob_oid = tree.get(0).expect("one entry").id();

    let size = |oid| odb.read_header(oid).expect("header").0 as u64;
    let contract = contract_format(format);
    let objects = vec![
        ObjectRecord {
            oid: id(format, blob_oid),
            kind: ObjectKind::Blob,
            size: size(blob_oid),
            edges: Vec::new(),
        },
        ObjectRecord {
            oid: id(format, tree_oid),
            kind: ObjectKind::Tree,
            size: size(tree_oid),
            edges: vec![id(format, blob_oid)],
        },
        ObjectRecord {
            oid: id(format, commit_oid),
            kind: ObjectKind::Commit,
            size: size(commit_oid),
            edges: vec![id(format, tree_oid)],
        },
    ];
    let roots = normalise_roots(vec![
        ProtectedRoot {
            source: RootSource::Head,
            oid: id(format, commit_oid),
        },
        ProtectedRoot {
            source: RootSource::Ref {
                name: "refs/heads/main".to_owned(),
            },
            oid: id(format, commit_oid),
        },
    ]);
    GraphFixture {
        objects,
        roots,
        missing: ObjectId::from_bytes(contract, &vec![0xEE; contract.digest_len()])
            .expect("absent id"),
    }
}

#[test]
fn the_object_reader_conformance_suite_passes_in_both_object_formats() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        let fixture = Fixture::checkout(format);
        let expected = expectations(&fixture, format);
        let reader = LocalObjectReader::open(&admitted(fixture.root()));
        object_reader_conformance(&reader, &expected);
    }
}

#[test]
fn a_commits_edges_are_its_tree_then_its_parents() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let first = fixture.head_commit();
    fixture.write("second.txt", b"second\n");
    let second = fixture.commit("second");

    let reader = LocalObjectReader::open(&admitted(fixture.root()));
    let record = reader
        .read_object(&id(ObjectFormat::Sha1, second), &ReadLimits::default())
        .expect("the second commit reads");
    assert_eq!(record.kind, ObjectKind::Commit);
    let tree = fixture
        .open()
        .find_commit(second)
        .expect("commit")
        .tree_id();
    assert_eq!(
        record.edges,
        vec![id(ObjectFormat::Sha1, tree), id(ObjectFormat::Sha1, first)]
    );
}

#[test]
fn a_tags_edge_is_its_target_so_a_traversal_reaches_the_commit() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let tag = fixture.annotated_tag("v1");
    let reader = LocalObjectReader::open(&admitted(fixture.root()));
    let record = reader
        .read_object(&id(ObjectFormat::Sha1, tag), &ReadLimits::default())
        .expect("the tag object reads");
    assert_eq!(record.kind, ObjectKind::Tag);
    assert_eq!(
        record.edges,
        vec![id(ObjectFormat::Sha1, fixture.head_commit())]
    );
}

#[test]
fn tree_edges_exclude_a_gitlink_owned_by_a_member_repository() {
    let outer = Fixture::checkout(ObjectFormat::Sha1);
    let member = Fixture::checkout(ObjectFormat::Sha1);
    member.write("member.txt", b"member-only\n");
    member.commit("member commit");
    let member_commit = member.head_commit();
    let repository = outer.open();
    let base_tree = repository
        .find_commit(outer.head_commit())
        .expect("outer commit")
        .tree()
        .expect("outer tree");
    let mut builder = repository
        .treebuilder(Some(&base_tree))
        .expect("tree builder");
    builder
        .insert("member", member_commit, 0o160000)
        .expect("gitlink entry");
    let tree = builder.write().expect("tree with gitlink");

    let reader = LocalObjectReader::open(&admitted(outer.root()));
    let record = reader
        .read_object(&id(ObjectFormat::Sha1, tree), &ReadLimits::default())
        .expect("outer tree reads without the member object");
    assert_eq!(record.kind, ObjectKind::Tree);
    assert_eq!(
        record.edges,
        vec![id(ObjectFormat::Sha1, base_tree.get(0).expect("blob").id())]
    );
    assert_eq!(
        reader.read_object(
            &id(ObjectFormat::Sha1, member_commit),
            &ReadLimits::default()
        ),
        Err(ReadError::Missing {
            oid: id(ObjectFormat::Sha1, member_commit)
        })
    );
}

#[test]
fn a_missing_object_refuses_typed_and_is_never_fetched() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let reader = LocalObjectReader::open(&admitted(fixture.root()));
    let absent = ObjectId::from_bytes(ContractFormat::Sha1, &[0x11; 20]).expect("absent id");
    assert_eq!(
        reader.read_object(&absent, &ReadLimits::default()),
        Err(ReadError::Missing { oid: absent })
    );
}

#[test]
fn an_object_over_the_limit_refuses_with_its_size_and_the_limit() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let body = vec![b'x'; 4096];
    fixture.write("large.txt", &body);
    fixture.commit("large");
    let blob = fixture
        .open()
        .find_commit(fixture.head_commit())
        .expect("commit")
        .tree()
        .expect("tree")
        .get_name("large.txt")
        .expect("entry")
        .id();

    let reader = LocalObjectReader::open(&admitted(fixture.root()));
    let oid = id(ObjectFormat::Sha1, blob);
    match reader.read_object(&oid, &ReadLimits::new(4095)) {
        Err(ReadError::LimitExceeded { size, limit, .. }) => {
            assert_eq!(size, 4096);
            assert_eq!(limit, 4095);
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
    assert!(reader.read_object(&oid, &ReadLimits::new(4096)).is_ok());
}

#[test]
fn an_id_in_the_wrong_object_format_refuses_rather_than_being_truncated() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let reader = LocalObjectReader::open(&admitted(fixture.root()));
    let wrong = ObjectId::from_bytes(ContractFormat::Sha256, &[0x22; 32]).expect("sha256 id");
    assert!(matches!(
        reader.read_object(&wrong, &ReadLimits::default()),
        Err(ReadError::ReadFailed { .. })
    ));
}

#[test]
fn retained_roots_keep_persistent_import_refs_and_drop_open_operation_refs() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.reference("refs/gwz/local-imports/t1");
    fixture.reference("refs/gwz/merge/m1/root/head");

    let reader = LocalObjectReader::open(&admitted(fixture.root()));
    let roots: ProtectedRoots = reader.retained_roots().expect("roots");
    let names: Vec<_> = roots
        .roots
        .iter()
        .filter_map(|root| match &root.source {
            RootSource::Ref { name } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert!(
        names.contains(&"refs/gwz/local-imports/t1".to_owned()),
        "a retained import ref is a witness root: {names:?}"
    );
    assert!(
        !names.contains(&"refs/gwz/merge/m1/root/head".to_owned()),
        "an open-operation ref is not a witness: {names:?}"
    );
}

#[test]
fn a_cloned_reader_serves_the_same_repository() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let info = admitted(fixture.root());
    let reader = LocalObjectReader::open(&info);
    let copy = reader.clone();
    assert_eq!(reader, copy);
    assert_eq!(reader.repository(), fixture.real_root());
    assert_eq!(
        reader.retained_roots().expect("roots"),
        copy.retained_roots().expect("roots")
    );
}
