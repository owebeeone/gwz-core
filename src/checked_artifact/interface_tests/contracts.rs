use std::io::{Cursor, Read};
use std::path::Path;

use super::super::bootstrap::{WorkspaceRuntimeBootstrapV1, WorkspaceRuntimePaths};
use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, GitPathBytes, IndexStage, LosslessIndexEntry, PathComponentMode,
    PlatformCapability, PrivateControlDomain, SupportedFilesystemProfile, TrackedWorktreeEntry,
    TrackedWorktreeKind,
};
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
    let bytes = path.encode_canonical();
    assert_eq!(
        CanonicalPathIdentityV1::decode_canonical_for_test(&bytes).unwrap(),
        path
    );
    let mut trailing = bytes;
    trailing.push(0);
    assert!(CanonicalPathIdentityV1::decode_canonical_for_test(&trailing).is_err());
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

#[test]
fn collision_facts_keep_raw_paths_stages_and_flags() {
    let path = GitPathBytes::new(vec![0xff, b'/', b'a']).unwrap();
    let entry = LosslessIndexEntry::new(path.clone(), 2, 0o100644, 0xc000, 0x6000).unwrap();
    assert_eq!(entry.path(), &path);
    assert_eq!(entry.stage(), IndexStage::Ours);
    assert_eq!(entry.raw_flags(), 0xc000);
    assert_eq!(entry.raw_extended_flags(), 0x6000);
    assert!(LosslessIndexEntry::new(path.clone(), 4, 0, 0, 0).is_err());

    let tracked = TrackedWorktreeEntry::new(path, TrackedWorktreeKind::Symlink);
    assert_eq!(tracked.kind(), TrackedWorktreeKind::Symlink);
    assert_eq!(PrivateControlDomain::checked_v1().members().len(), 4);
}

#[test]
fn collision_domain_is_fixed_and_digest_bound() {
    let domain = PrivateControlDomain::checked_v1();
    assert_eq!(domain.members().len(), 4);
    assert_ne!(domain.version_digest(), [0; 32]);
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
