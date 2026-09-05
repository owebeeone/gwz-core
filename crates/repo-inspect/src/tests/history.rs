//! Design §5.1: "Protect all refs, HEAD (including detached), retained
//! reflog roots, annotated objects, native stashes and older stash entries",
//! plus the objects a decoded GWZ coordination record names.

use git2::ObjectFormat;
use gwz_repo_contract::{
    ObjectFormat as ContractFormat, ObjectId, Observation, ProtectedRoots, RepoInspector,
    RootSource,
};

use super::admitted;
use crate::fixtures::Fixture;
use crate::{CoordinationRoot, LocalRepoInspector};

fn inventory(fixture: &Fixture) -> ProtectedRoots {
    inventory_with(fixture, LocalRepoInspector::new())
}

fn inventory_with(fixture: &Fixture, inspector: LocalRepoInspector) -> ProtectedRoots {
    let info = admitted(fixture.root());
    match inspector.inventory_history(&info) {
        Observation::Known(roots) => roots,
        Observation::Unknown(reasons) => panic!("expected a known inventory, got {reasons:?}"),
    }
}

fn oid_of(roots: &ProtectedRoots, source: &RootSource) -> Option<ObjectId> {
    roots
        .roots
        .iter()
        .find(|root| &root.source == source)
        .map(|root| root.oid.clone())
}

fn hex(fixture: &Fixture) -> String {
    fixture.head_commit().to_string()
}

#[test]
fn refs_and_head_are_roots_at_the_commit_they_name() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let roots = inventory(&fixture);
    let head = oid_of(&roots, &RootSource::Head).expect("HEAD is a root");
    let branch = oid_of(
        &roots,
        &RootSource::Ref {
            name: "refs/heads/main".to_owned(),
        },
    )
    .expect("the branch is a root");
    assert_eq!(head.to_hex(), hex(&fixture));
    assert_eq!(branch, head);
    // One commit, one branch, one HEAD: no reflog root is added for an object
    // an earlier root already names.
    assert_eq!(roots.roots.len(), 2, "{roots:?}");
}

#[test]
fn a_detached_head_is_still_a_root() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let commit = hex(&fixture);
    fixture.detach_head();
    let roots = inventory(&fixture);
    assert_eq!(
        oid_of(&roots, &RootSource::Head).map(|oid| oid.to_hex()),
        Some(commit)
    );
}

#[test]
fn an_unborn_head_contributes_no_root_and_is_not_an_error() {
    let fixture = Fixture::empty_checkout(ObjectFormat::Sha1);
    assert_eq!(inventory(&fixture), ProtectedRoots::default());
}

#[test]
fn an_annotated_tag_is_a_tag_root_and_a_lightweight_tag_stays_a_ref() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let annotated = fixture.annotated_tag("v1");
    fixture.lightweight_tag("v0");

    let roots = inventory(&fixture);
    assert_eq!(
        oid_of(
            &roots,
            &RootSource::AnnotatedTag {
                name: "refs/tags/v1".to_owned()
            }
        )
        .map(|oid| oid.to_hex()),
        Some(annotated.to_string()),
        "the annotated tag root is the tag object itself: {roots:?}"
    );
    assert_eq!(
        oid_of(
            &roots,
            &RootSource::Ref {
                name: "refs/tags/v0".to_owned()
            }
        )
        .map(|oid| oid.to_hex()),
        Some(hex(&fixture)),
        "a lightweight tag is an ordinary ref: {roots:?}"
    );
    // The annotated tag is reported once, not also as a `Ref`.
    assert!(
        oid_of(
            &roots,
            &RootSource::Ref {
                name: "refs/tags/v1".to_owned()
            }
        )
        .is_none(),
        "{roots:?}"
    );
}

#[test]
fn a_commit_only_the_reflog_still_names_is_a_reflog_root() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("second.txt", b"second\n");
    fixture.commit("second");
    let abandoned = hex(&fixture);
    // Move the branch back: the second commit survives only in the reflog.
    fixture.reset_branch_to_first_parent();
    assert_ne!(hex(&fixture), abandoned);

    let roots = inventory(&fixture);
    let reflog_roots: Vec<_> = roots
        .roots
        .iter()
        .filter(|root| matches!(root.source, RootSource::Reflog { .. }))
        .collect();
    assert!(
        reflog_roots
            .iter()
            .any(|root| root.oid.to_hex() == abandoned),
        "{roots:?}"
    );
}

#[test]
fn native_stash_entries_including_older_ones_are_roots() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.write("tracked.txt", b"first change\n");
    fixture.stash("first");
    fixture.write("tracked.txt", b"second change\n");
    fixture.stash("second");

    let roots = inventory(&fixture);
    let newest = oid_of(&roots, &RootSource::Stash { index: 0 }).expect("stash@{0}");
    let older = oid_of(&roots, &RootSource::Stash { index: 1 }).expect("stash@{1}");
    assert_ne!(newest, older, "{roots:?}");
}

#[test]
fn the_objects_a_coordination_record_names_are_reported_as_named_roots() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    let commit = ObjectId::parse_hex(ContractFormat::Sha1, &hex(&fixture)).expect("head id");
    let inspector = LocalRepoInspector::new().with_coordination_roots([CoordinationRoot {
        record: "stash gwz_stash_0007".to_owned(),
        object: "base".to_owned(),
        oid: commit.clone(),
    }]);

    let roots = inventory_with(&fixture, inspector);
    assert_eq!(
        oid_of(
            &roots,
            &RootSource::CoordinationRecord {
                record: "stash gwz_stash_0007".to_owned(),
                object: "base".to_owned()
            }
        ),
        Some(commit)
    );
    // Without the record, the same repository has no such root: the ids come
    // from core's decoding, never from this crate guessing at a bundle.
    assert!(
        !inventory(&fixture)
            .roots
            .iter()
            .any(|root| matches!(root.source, RootSource::CoordinationRecord { .. }))
    );
}

#[test]
fn a_bare_repository_inventories_its_own_refs() {
    let source = Fixture::checkout(ObjectFormat::Sha1);
    let hub = source.clone_into_bare("hub.git");
    let roots = inventory(&hub);
    assert!(
        roots.roots.iter().any(|root| matches!(
            &root.source,
            RootSource::Ref { name } if name == "refs/heads/main"
        )),
        "{roots:?}"
    );
}

#[test]
fn a_sha256_inventory_reports_thirty_two_byte_ids() {
    let fixture = Fixture::checkout(ObjectFormat::Sha256);
    let roots = inventory(&fixture);
    assert!(!roots.roots.is_empty());
    for root in &roots.roots {
        assert_eq!(root.oid.format(), ContractFormat::Sha256);
        assert_eq!(root.oid.as_bytes().len(), 32);
    }
}

#[test]
fn the_inventory_is_repeatable_and_order_independent() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    fixture.annotated_tag("v1");
    fixture.write("tracked.txt", b"stashed\n");
    fixture.stash("stash");
    assert_eq!(inventory(&fixture), inventory(&fixture));
}

#[test]
fn an_unreadable_reference_makes_the_inventory_unknown_never_a_short_list() {
    let fixture = Fixture::checkout(ObjectFormat::Sha1);
    // A loose ref file that is not an object id: the inventory cannot say
    // what this reference protects, so it says unknown rather than omitting
    // it (design §5.1: "Unknown layouts/evidence refuse ordinary deletion").
    std::fs::write(
        fixture.git_dir().join("refs/heads/broken"),
        b"not-an-object-id\n",
    )
    .expect("broken ref");

    let info = admitted(fixture.root());
    let Observation::Unknown(reasons) = LocalRepoInspector::new().inventory_history(&info) else {
        panic!("a reference that cannot be read must make the inventory unknown");
    };
    assert!(
        reasons
            .iter()
            .any(|reason| reason.kind == gwz_repo_contract::UnknownKind::Unreadable),
        "{reasons:?}"
    );
}
