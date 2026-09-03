use std::io::{Cursor, Read};
use std::path::Path;

use super::super::bootstrap::{WorkspaceRuntimeBootstrapV1, WorkspaceRuntimePaths};
use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, DurablePathV1, GitPathBytes, IndexStage, IndexTimestampV1,
    LosslessIndexEntry, LosslessIndexMetadataV1, PathComponentMode, PlatformCapability,
    PrivateControlDomain, SupportedFilesystemProfile, TrackedWorktreeEntry, TrackedWorktreeKind,
};
use super::super::catalog_names::CatalogPrivateNameV1;
use super::super::entry::{CATALOG_LABEL, render_catalog_refusal};
use super::super::leaf::{
    DurableLeafExpectation, DurableLeafProof, ExpectedLeafContent, LeafOther, LeafProof,
};
use super::super::namespace::test_support::{
    durable_namespace, published_identity, reserved_slot, retained_directory, retained_object,
    retired_identity,
};
use super::super::namespace::{
    ActionNamespace, DurableNamespace, NamespaceObjectKind, NamespaceProtocol, PublishedIdentity,
    ReservedNamespaceSlot, ReservedRetirementSlot, RetainedDirectory, RetainedNamespaceObject,
    RetiredIdentity,
};
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1,
    ActionDirectoryObservationV1, ActionScheduleV1, CleanupAliasSetV1, RecordObservationV1,
    RequestOwnerBindingV1, admit_observed_action,
};
use crate::model::ErrorCode;

#[test]
fn ascii_components_and_component_modes_are_exact() {
    let upper = AsciiComponent::parse(b"Catalog-A").unwrap();
    assert_eq!(upper.as_bytes(), b"Catalog-A");
    assert!(AsciiComponent::parse(b"").is_err());
    assert!(AsciiComponent::parse(b".").is_err());
    assert!(AsciiComponent::parse(b"..").is_err());
    assert!(AsciiComponent::parse(b"a/b").is_err());
    assert!(AsciiComponent::parse(b"a\\b").is_err());
    assert!(AsciiComponent::parse(&[0xff]).is_err());
    assert!(AsciiComponent::parse(&vec![b'a'; 256]).is_err());

    let sensitive = CanonicalComponent::new(upper.clone(), PathComponentMode::Sensitive);
    assert_eq!(sensitive.original().as_bytes(), b"Catalog-A");
    assert_eq!(sensitive.canonical_ascii(), b"Catalog-A");

    let folded = CanonicalComponent::new(upper, PathComponentMode::AsciiCaseFold);
    assert_eq!(folded.canonical_ascii(), b"catalog-a");
    assert_eq!(folded.parent_mode(), PathComponentMode::AsciiCaseFold);
}

#[test]
fn canonical_path_identity_is_nonempty_and_bounded() {
    let component = CanonicalComponent::new(
        AsciiComponent::parse(b"leaf").unwrap(),
        PathComponentMode::Sensitive,
    );
    let path = CanonicalPathIdentityV1::new(vec![component]).unwrap();
    assert_eq!(path.components().len(), 1);
    let durable = DurablePathV1::from_live(&path).unwrap();
    let bytes = durable.encode_canonical();
    assert_eq!(DurablePathV1::decode_canonical(&bytes).unwrap(), durable);
    let mut trailing = bytes;
    trailing.push(0);
    assert!(DurablePathV1::decode_canonical(&trailing).is_err());
    assert!(CanonicalPathIdentityV1::new(Vec::new()).is_err());

    let many = (0..17)
        .map(|_| {
            CanonicalComponent::new(
                AsciiComponent::parse(&vec![b'x'; 255]).unwrap(),
                PathComponentMode::Sensitive,
            )
        })
        .collect();
    assert!(CanonicalPathIdentityV1::new(many).is_err());

    let mixed_profiles = CanonicalPathIdentityV1::new(vec![
        CanonicalComponent::try_bound(
            AsciiComponent::parse(b"workspace").unwrap(),
            PathComponentMode::Sensitive,
            DurableObjectIdentityV1::linux_ext4([1; 16], 1, vec![1]).unwrap(),
            vec![1],
            vec![2],
        )
        .unwrap(),
        CanonicalComponent::try_bound(
            AsciiComponent::parse(b"catalog").unwrap(),
            PathComponentMode::Sensitive,
            DurableObjectIdentityV1::mac([3; 16], [4; 8]).unwrap(),
            vec![3],
            vec![4],
        )
        .unwrap(),
    ]);
    assert!(
        mixed_profiles.is_err(),
        "one durable path cannot mix filesystem support profiles"
    );
}

#[test]
fn durable_identity_profiles_validate_and_round_trip() {
    let values = [
        DurableObjectIdentityV1::linux_ext4([1; 16], 1, vec![2; 24]).unwrap(),
        DurableObjectIdentityV1::mac([3; 16], [4; 8]).unwrap(),
        DurableObjectIdentityV1::windows_ntfs(vec![5; 32], [6; 16]).unwrap(),
    ];
    for value in values {
        let bytes = value.encode_canonical();
        assert_eq!(
            DurableObjectIdentityV1::decode_canonical(&bytes).unwrap(),
            value
        );
    }
    assert!(DurableObjectIdentityV1::linux_ext4([0; 16], 1, vec![1]).is_err());
    assert!(DurableObjectIdentityV1::linux_ext4([1; 16], 0, vec![1]).is_err());
    assert!(DurableObjectIdentityV1::linux_ext4([1; 16], 1, vec![1; 129]).is_err());
}

#[test]
fn supported_profiles_and_typed_errors_are_closed() {
    assert_eq!(
        SupportedFilesystemProfile::ALL,
        &[
            SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
            SupportedFilesystemProfile::MacPersistentObjectIdV1,
            SupportedFilesystemProfile::WindowsNtfsFileId128V1,
        ]
    );
    let unsupported = CheckedFsError::unsupported(
        PlatformCapability::DurableObjectIdentity,
        "unsupported filesystem",
    );
    assert!(matches!(unsupported, CheckedFsError::Unsupported { .. }));
    let io = CheckedFsError::io("identity query", std::io::Error::other("failed"));
    assert!(matches!(io, CheckedFsError::Io { .. }));
    let ambiguity = CheckedFsError::ambiguous("parent", "identity changed");
    assert!(matches!(ambiguity, CheckedFsError::Ambiguous { .. }));
}

/// R2-E E4.1 precondition 1: the durable-identity gap is a capability of its
/// own, and it is the ONLY one that carries an actionable sentence — because it
/// is the only platform gap a user meets on a supported OS and can do something
/// about. The value-shape contract keeps `DurableObjectIdentity`; the substrate
/// gap is `PersistentFilesystemIdentity`, and its refusal names persistent file
/// handles, the admitted filesystems, and the escapes.
///
/// **MOVED at DR-1 ship (1) W3** (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.6,
/// 2026-09-03), three terms at once and each for a stated reason:
/// - `--no-ff` is DROPPED. It was the escape "a new merge can be started
///   without --no-ff", and it is no longer one: a below-bar `--no-ff` start now
///   warns and runs rather than refusing, so there is nothing to escape from.
/// - `--filesystem-strict` is ADDED. It is the flag that produces this sentence
///   at all now, so the sentence must name the way back off it. This term is
///   what keeps the remedy and the decision point from drifting apart: delete
///   the flag's handling and this pin fails.
/// - `mount identity` becomes `durable filesystem identity`. §3.6 requires
///   identity-based wording; "mount identity" named the LEGACY probe's second
///   half (`statx MNT_ID`), which is not the catalog's bar and never was.
///
/// `persistent file handles` and `--abort` are unchanged — the first is the
/// substrate the sentence exists to name, the second is [P2-1]'s cure, the exit
/// that does exist for a merge already open.
#[test]
fn only_the_substrate_identity_capability_carries_an_actionable_remedy() {
    let remedy = PlatformCapability::PersistentFilesystemIdentity
        .remedy()
        .expect("the substrate identity gap is actionable");
    for named in [
        "persistent file handles",
        "durable filesystem identity",
        "--abort",
        "--filesystem-strict",
    ] {
        assert!(remedy.contains(named), "the remedy never names {named}");
    }
    // The identity-based bar is a CONTRACT, not a name list: the filesystems
    // are named as examples of answering `FS_IOC_GETFSUUID`, and the "ext4
    // only" clause W2 dated stale is gone.
    assert!(remedy.contains("FS_IOC_GETFSUUID"), "{remedy}");
    assert!(!remedy.contains("ext4 only"), "{remedy}");
    assert!(!remedy.contains("--no-ff"), "{remedy}");
    assert_eq!(PlatformCapability::DurableObjectIdentity.remedy(), None);
    assert_eq!(PlatformCapability::RuntimeAdvisoryLock.remedy(), None);
}

/// R2-E E4.1 review [P3-2], carried to E4.2: precondition 1's rendering, driven.
///
/// The sentence a user reads on a substrate without persistent file handles was
/// verified by hand on a real FAT32 volume and by nothing in-suite, the renderer
/// being inline. Named, every arm takes a direct-constructor row here.
///
/// **DR-1 ship (1) W3 (charter §3.6, 2026-09-03) keeps this pin as written and
/// re-reasons WHAT it now covers.** The decision point (§2) takes the
/// `Unsupported` arm off the default `--no-ff` path — it warns instead — so
/// this renderer is no longer how a below-bar user meets the bar. It still
/// renders every activation refusal that happens AFTER a `Supported` decision:
/// a race, an `Io`, an `Ambiguous`. The rows below are unchanged because the
/// renderer is: only the remedy STRING it carries was rewritten, and the terms
/// asserted here (`persistent file handles`, `--abort`) survived that rewrite
/// deliberately — see the pin above.
#[test]
fn the_catalog_refusal_renderer_carries_the_remedy_into_every_arm() {
    let identity = PlatformCapability::PersistentFilesystemIdentity;
    let rows = [
        // The actionable sentence AND the substrate's own words, together.
        (
            CheckedFsError::unsupported(identity, "vfat lacks handles"),
            ErrorCode::UnsupportedOperation,
            ["persistent file handles", "--abort", "vfat lacks handles"],
        ),
        // A capability with no remedy must not borrow the actionable one.
        (
            CheckedFsError::unsupported(PlatformCapability::RuntimeAdvisoryLock, "no flock"),
            ErrorCode::UnsupportedOperation,
            ["is unsupported", "no flock", "no flock"],
        ),
        (
            CheckedFsError::io("open catalog root", std::io::Error::other("gone")),
            ErrorCode::IoError,
            ["open catalog root", "gone", "gone"],
        ),
        (
            CheckedFsError::ambiguous("catalog root", "identity changed"),
            ErrorCode::IoError,
            ["rejected catalog root", "identity changed", "changed"],
        ),
    ];
    for (index, (cause, code, named)) in rows.into_iter().enumerate() {
        let rendered = render_catalog_refusal(CATALOG_LABEL, cause);
        assert_eq!(rendered.code, code);
        assert!(rendered.message.contains(CATALOG_LABEL));
        assert_eq!(rendered.message.contains("--abort"), index == 0);
        for term in named {
            assert!(rendered.message.contains(term), "{}", rendered.message);
        }
    }
}

#[test]
fn collision_facts_keep_raw_paths_stages_and_flags() {
    let path = GitPathBytes::new(vec![0xff, b'/', b'a']).unwrap();
    let metadata = LosslessIndexMetadataV1::new(
        IndexTimestampV1::new(11, 12).unwrap(),
        IndexTimestampV1::new(21, 22).unwrap(),
        [31, 32, 33, 34, 35],
        vec![0x44; 20],
    )
    .unwrap();
    let entry =
        LosslessIndexEntry::new(path.clone(), 2, 0o100644, 0xc000, 0x6000, metadata.clone())
            .unwrap();
    assert_eq!(entry.path(), &path);
    assert_eq!(entry.stage(), IndexStage::Ours);
    assert_eq!(entry.raw_flags(), 0xc000);
    assert_eq!(entry.raw_extended_flags(), 0x6000);
    assert_eq!(entry.metadata(), &metadata);
    assert_eq!(entry.metadata().ctime().seconds(), 11);
    assert_eq!(entry.metadata().mtime().nanoseconds(), 22);
    assert_eq!(entry.metadata().stat(), &[31, 32, 33, 34, 35]);
    assert_eq!(entry.metadata().object_id(), &[0x44; 20]);
    assert!(LosslessIndexEntry::new(path.clone(), 4, 0, 0, 0, metadata).is_err());
    assert!(IndexTimestampV1::new(0, 1_000_000_000).is_err());

    let tracked = TrackedWorktreeEntry::new(path, TrackedWorktreeKind::Symlink);
    assert_eq!(tracked.kind(), TrackedWorktreeKind::Symlink);
    assert_eq!(PrivateControlDomain::checked_v1().members().len(), 5);
}

#[test]
fn collision_domain_is_fixed_and_digest_bound() {
    let domain = PrivateControlDomain::checked_v1();
    assert_eq!(domain.members().len(), 5);
    assert_ne!(domain.version_digest(), [0; 32]);
}

/// `PrivateControlDomain::checked_v1().version_digest()` measured at gwz-core
/// `ea3a924` (v0.12.1), immediately before R2-F R1.1 — the four-member domain
/// whose `Final` member was `.gwz/checked-artifacts`.
const PRE_R2F_WORKSPACE_COLLISION_DIGEST: [u8; 32] = [
    0x3b, 0xa1, 0x85, 0x95, 0xfb, 0x99, 0x1a, 0x64, 0x3f, 0xdb, 0x5c, 0xe7, 0xcc, 0x51, 0x49, 0xa8,
    0xe2, 0xd2, 0x57, 0xa8, 0x04, 0x25, 0xdc, 0x30, 0x2b, 0x48, 0xa7, 0xb5, 0xc1, 0x86, 0x85, 0xc6,
];

/// R2-F R1.1, 2026-09-01 — the persisted-field movements, asserted once each.
///
/// THE TRADE, complete (plan §1, [R2-P3-1]/[RC-P2-1]): the split moves exactly
/// three durable surfaces, and declares all three free. (1)
/// `historical_collision_digest`, CBOR key 4 of `CheckedCatalogBootstrapV1` —
/// moved by `LegacyPrivate` joining `ALL`. (2) `final_name`, CBOR key 8
/// (`protocol/catalog_bootstrap_record.rs:75`, validated `:242`) — moved by
/// `Final`'s bytes. (3) the on-disk scratch DIRECTORY name
/// (`capability/pre_catalog.rs:331-335`), which frames the same digest.
///
/// They are free because no durable record exists to invalidate: the movement
/// lands strictly before the first catalog activation
/// (`catalog_names.rs`'s ordering ground; `recover_or_create` has zero
/// production callers). Deliberately ONE assertion per persisted field — the
/// digest's ~57 read references are NOT enumerated; enumerating them was
/// withdrawn as scope inflation ([RC-P2-1]).
#[test]
fn the_split_moves_both_persisted_catalog_fields_and_is_free_before_activation() {
    // Field 1 — historical_collision_digest, via the domain's member set.
    let domain = PrivateControlDomain::checked_v1();
    assert_ne!(domain.version_digest(), PRE_R2F_WORKSPACE_COLLISION_DIGEST);
    // Field 2 — final_name, the catalog's own leaf, now disjoint from the
    // legacy writer's. `LegacyPrivate` is what `policy.rs` composes; the two
    // names are the split.
    assert_eq!(
        CatalogPrivateNameV1::Final.leaf_bytes(),
        b"catalog-final".as_slice()
    );
    assert_eq!(
        CatalogPrivateNameV1::LegacyPrivate.leaf_bytes(),
        b"checked-artifacts".as_slice()
    );
}

/// R2-E E4.1 (plan §3's E4 gate note, rider 4): the two private paths are
/// spelled a SECOND time, as string constants, inside the merge preservation
/// image — `CHECKED_ARTIFACT_PRIVATE_PATH` (the legacy writer's, `:8`) and
/// `CATALOG_PRIVATE_PATH` (the catalog's, `:20`). The git-status dirt exemption
/// and preservation blindness compile against those strings, not against
/// `CatalogPrivateNameV1`, and neither side can see the other
/// (`pub(in crate::checked_artifact)` versus `pub(super)`), so a future leaf
/// rename would move the catalog while merge silently kept exempting the OLD
/// path. Fixtures are fail-loud; this pair is the silent one. Source-scan
/// idiom, beside the `leaf_bytes` pins above. `//` comments are stripped first,
/// per the R1.2 tripwire's [P3-8] precedent: a `// was: …` remnant must not
/// satisfy a pin after a real rename.
#[test]
fn the_preservation_image_spells_both_private_paths_beside_their_names() {
    let image = include_str!("../../git/gitbackend/preservation_image.rs")
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(kept, _)| kept))
        .collect::<String>();

    for (name, literal) in [
        (
            "CHECKED_ARTIFACT_PRIVATE_PATH",
            r#"".gwz/checked-artifacts""#,
        ),
        ("CATALOG_PRIVATE_PATH", r#"".gwz/catalog-final""#),
    ] {
        assert!(
            image.contains(&format!("{name}: &str = {literal};")),
            "the preservation image's second authority for {name} drifted from \
             {literal}; the dirt exemption and preservation blindness compile \
             against this string, not against `CatalogPrivateNameV1`"
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Identity(u8);

#[derive(Clone)]
struct Bytes(Vec<u8>);

impl ExpectedLeafContent for Bytes {
    type Reader = Cursor<Vec<u8>>;

    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn sha256(&self) -> [u8; 32] {
        [9; 32]
    }

    fn open(&self) -> std::io::Result<Self::Reader> {
        Ok(Cursor::new(self.0.clone()))
    }
}

#[test]
fn expected_content_is_repeatable_and_leaf_proofs_are_typed() {
    let expected = Bytes(b"bytes".to_vec());
    let mut first = expected.open().unwrap();
    let mut second = expected.open().unwrap();
    let mut left = Vec::new();
    let mut right = Vec::new();
    first.read_to_end(&mut left).unwrap();
    second.read_to_end(&mut right).unwrap();
    assert_eq!(left, right);

    let ordinary: LeafProof<Identity> = LeafProof::Exact {
        identity: Identity(1),
        length: 5,
        sha256: [2; 32],
        bytes: b"bytes".to_vec(),
    };
    assert!(ordinary.is_exact());
    let durable: DurableLeafProof<Identity> = DurableLeafProof::ExactDurable {
        identity: Identity(1),
        length: 5,
        sha256: [2; 32],
    };
    assert!(durable.is_exact_durable());
    let other: DurableLeafProof<Identity> = DurableLeafProof::Other(LeafOther::Substituted);
    assert!(!other.is_exact_durable());
    assert!(matches!(
        DurableLeafExpectation::Exact(&expected),
        DurableLeafExpectation::Exact(_)
    ));
}

#[derive(Default)]
struct RecordingNamespace {
    barriers: Vec<u8>,
}

impl NamespaceProtocol for RecordingNamespace {
    type DirectoryHandle = u8;
    type ObjectHandle = u8;
    type Identity = Identity;
    type PathProfile = CanonicalPathIdentityV1;
    type ReservationBinding = u8;
    type BarrierOrdinal = u8;

    fn publish_no_replace(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        _destination: &ReservedNamespaceSlot<
            Self::DirectoryHandle,
            Self::Identity,
            Self::PathProfile,
            Self::ReservationBinding,
        >,
    ) -> Result<PublishedIdentity<Self::Identity>, CheckedFsError> {
        Ok(published_identity(source.identity().clone()))
    }

    fn retire_exact(
        &mut self,
        source: &RetainedNamespaceObject<
            Self::DirectoryHandle,
            Self::ObjectHandle,
            Self::Identity,
            Self::PathProfile,
        >,
        _destination: &ReservedRetirementSlot<
            Self::DirectoryHandle,
            Self::Identity,
            Self::PathProfile,
            Self::ReservationBinding,
        >,
    ) -> Result<RetiredIdentity<Self::Identity>, CheckedFsError> {
        Ok(retired_identity(source.identity().clone()))
    }

    fn barrier(
        &mut self,
        _parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        ordinal: Self::BarrierOrdinal,
    ) -> Result<DurableNamespace, CheckedFsError> {
        self.barriers.push(ordinal);
        Ok(durable_namespace())
    }
}

#[test]
fn namespace_protocol_requires_retained_sources_reserved_destinations_and_ordinal() {
    let path = one_component_profile();
    let source_parent = retained_directory(1, Identity(1), path.clone());
    let destination_parent = retained_directory(2, Identity(2), path.clone());
    let source = retained_object(
        source_parent,
        AsciiComponent::parse(b"source").unwrap(),
        9,
        Identity(9),
        NamespaceObjectKind::RegularFile,
    );
    let destination = reserved_slot(
        destination_parent,
        AsciiComponent::parse(b"goal").unwrap(),
        3,
    );
    let mut protocol = RecordingNamespace::default();
    assert_eq!(
        protocol
            .publish_no_replace(&source, &destination)
            .unwrap()
            .identity(),
        &Identity(9)
    );
    protocol.barrier(source.parent(), 7).unwrap();
    assert_eq!(protocol.barriers, vec![7]);
}

#[test]
fn action_namespace_is_issued_only_after_idle_exact_admission() {
    let reservation = ActionCapacityReservationV1::new(
        ActionDigestV1::new([3; 32]),
        RequestOwnerBindingV1::new([4; 32]),
        ActionScheduleV1::try_new(1, Vec::new(), CleanupAliasSetV1::all()).unwrap(),
    );
    let missing = ActionDirectoryObservationV1::Missing;
    let exact = ActionDirectoryObservationV1::exact(
        linux_identity(8),
        RecordObservationV1::Exact(reservation.clone()),
    );
    assert!(
        admit_observed_action(
            &ActionDirectoryAdmissionV1::preparing(&reservation),
            &reservation,
            &missing,
            &exact,
        )
        .is_none()
    );
    let admitted = admit_observed_action(
        &ActionDirectoryAdmissionV1::idle(),
        &reservation,
        &missing,
        &exact,
    )
    .unwrap();
    assert_eq!(admitted.directory_identity(), &linux_identity(8));
    let action_namespace = ActionNamespace::from_admitted(RecordingNamespace::default(), admitted);
    assert_eq!(
        action_namespace
            .scheduled_barrier(0)
            .unwrap()
            .ordinal()
            .index(),
        0
    );
    assert!(action_namespace.scheduled_barrier(1).is_err());
    let destination =
        action_namespace.publish_destination(super::super::namespace::PublishRoleV1::GoalPayload);
    assert_eq!(
        destination.reservation_binding(),
        reservation.record_digest()
    );
    assert!(destination.leaf().as_bytes().ends_with(b"goal-payload-v1"));
}

struct RuntimeBootstrap;

impl WorkspaceRuntimeBootstrapV1 for RuntimeBootstrap {
    type Lease = String;

    fn try_acquire(
        &self,
        paths: WorkspaceRuntimePaths<'_>,
    ) -> Result<Option<Self::Lease>, CheckedFsError> {
        Ok(Some(format!(
            "{}:{}",
            paths.workspace_root().display(),
            paths.workspace_git_dir().display()
        )))
    }
}

#[test]
fn runtime_bootstrap_contract_remains_capability_neutral() {
    let runtime = RuntimeBootstrap;
    let paths = WorkspaceRuntimePaths::new(Path::new("/workspace"), Path::new("/workspace/.git"));
    assert!(runtime.try_acquire(paths).unwrap().is_some());
}

fn one_component_profile() -> CanonicalPathIdentityV1 {
    CanonicalPathIdentityV1::new(vec![CanonicalComponent::new(
        AsciiComponent::parse(b"leaf").unwrap(),
        PathComponentMode::Sensitive,
    )])
    .unwrap()
}

fn linux_identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
}
