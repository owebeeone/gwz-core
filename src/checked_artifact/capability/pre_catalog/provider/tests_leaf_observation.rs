//! R2-D Step 2.1 — the executed `LeafObserver` contract, and the shared
//! harness the `durable_leaf.*` matrix reuses.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 2.1 ("bounded/
//! streamed read through one retained handle, fingerprint, flush, same-parent
//! reobserve, identity/content/length stability; two-sided durable-absence
//! proof; payload size never conflated with protocol-record size") and
//! `GwzM5-8R2DInterfaceFreeze.md` §3.3 for the frozen seam these tests drive
//! unchanged, §4.3 rows E8-E11 for the primitive families each edge stands on.
//!
//! Living in a `tests`-prefixed file keeps the harness out of
//! `production_rust_files`
//! (`scripts/checks/check_checked_artifact_boundaries.py:670-677`) and out of
//! the injection-site rescan (`interface_tests/fault_expected_keys.rs:391`),
//! mirroring `admission/tests_fault_matrix.rs:18-22`.

use std::cell::Cell;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use sha2::{Digest, Sha256};

use super::directory_mutation::sync_directory_edge;
use super::leaf_observation::HostLeafObserverV1;
use super::platform::HostPlatform;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableIdentityProvider, DurableObjectIdentityV1, PathComponentMode,
};
use crate::checked_artifact::fault_v1::{CheckedArtifactFaultKeyV1 as Fault, run_next_at};
use crate::checked_artifact::leaf::{
    DurableLeafExpectation, DurableLeafProof, ExpectedLeafContent, LeafObserver, LeafOther,
    LeafProof,
};
use crate::checked_artifact::namespace::{
    DurableNamespace, NamespaceProtocol, PublishedIdentity, ReservedNamespaceSlot,
    ReservedRetirementSlot, RetainedDirectory, RetainedNamespaceObject, RetiredIdentity,
    test_support::{durable_namespace, retained_directory},
};
use crate::checked_artifact::protocol::ProtocolRecordKindV1;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// The retained parent shape the production observer is written against.
pub(super) type RetainedParentV1 =
    RetainedDirectory<Dir, DurableObjectIdentityV1, CanonicalPathIdentityV1>;

/// A payload the observer must open **twice** and never rewind
/// (`leaf.rs:54-56`). The open count is recorded so the two-sided proof can be
/// asserted rather than assumed.
pub(super) struct ExpectedPayloadV1 {
    bytes: Vec<u8>,
    opens: Cell<usize>,
}

impl ExpectedPayloadV1 {
    pub(super) fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            opens: Cell::new(0),
        }
    }

    pub(super) fn opens(&self) -> usize {
        self.opens.get()
    }
}

impl ExpectedLeafContent for ExpectedPayloadV1 {
    type Reader = Cursor<Vec<u8>>;

    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn sha256(&self) -> [u8; 32] {
        Sha256::digest(&self.bytes).into()
    }

    fn open(&self) -> io::Result<Self::Reader> {
        self.opens.set(self.opens.get() + 1);
        Ok(Cursor::new(self.bytes.clone()))
    }
}

/// The namespace the durable observation crosses. `barrier` performs the real
/// parent durability edge through the provider's own P2/P5 helper, so Step 2.1
/// crosses a durable barrier without pre-empting Step 2.2's backend; the two
/// mutating operations refuse, because a leaf *observation* performs no
/// namespace mutation.
#[derive(Default)]
pub(super) struct BarrierNamespaceV1 {
    crossed: usize,
}

impl BarrierNamespaceV1 {
    pub(super) fn crossed(&self) -> usize {
        self.crossed
    }
}

impl NamespaceProtocol for BarrierNamespaceV1 {
    type DirectoryHandle = Dir;
    type ObjectHandle = cap_std::fs::File;
    type Identity = DurableObjectIdentityV1;
    type PathProfile = CanonicalPathIdentityV1;
    type ReservationBinding = ();
    type BarrierOrdinal = u8;

    fn publish_no_replace(
        &mut self,
        _source: &RetainedNamespaceObject<
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
        Err(no_mutation())
    }

    fn retire_exact(
        &mut self,
        _source: &RetainedNamespaceObject<
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
        Err(no_mutation())
    }

    fn barrier(
        &mut self,
        parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        _ordinal: Self::BarrierOrdinal,
    ) -> Result<DurableNamespace, CheckedFsError> {
        self.crossed += 1;
        sync_directory_edge(parent.handle(), "leaf observation namespace barrier")?;
        Ok(durable_namespace())
    }
}

fn no_mutation() -> CheckedFsError {
    CheckedFsError::ambiguous(
        "leaf observation namespace",
        "a leaf observation performs no namespace mutation",
    )
}

pub(super) struct LeafFixture {
    root: PathBuf,
}

impl LeafFixture {
    pub(super) fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2d-leaf-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("parent")).unwrap();
        Self { root }
    }

    pub(super) fn parent_path(&self) -> PathBuf {
        self.root.join("parent")
    }

    /// A fresh capability on the observed parent, as a restarted process would
    /// acquire it.
    pub(super) fn parent(&self) -> Dir {
        open_dir(&self.parent_path())
    }
}

impl Drop for LeafFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn open_dir(path: &Path) -> Dir {
    Dir::open_ambient_dir(path, ambient_authority()).unwrap()
}

/// Mints the retained-parent capability from a live handle, binding the
/// identity the observer revalidates against after the barrier.
pub(super) fn retain(handle: Dir) -> RetainedParentV1 {
    let identity = HostPlatform
        .dir_identity(&handle)
        .unwrap()
        .durable()
        .clone();
    retain_as(handle, identity)
}

pub(super) fn retain_as(handle: Dir, identity: DurableObjectIdentityV1) -> RetainedParentV1 {
    retained_directory(handle, identity, path_profile())
}

fn path_profile() -> CanonicalPathIdentityV1 {
    CanonicalPathIdentityV1::new(vec![CanonicalComponent::new(
        AsciiComponent::parse(b"parent").unwrap(),
        PathComponentMode::Sensitive,
    )])
    .unwrap()
}

pub(super) fn component(name: &str) -> AsciiComponent {
    AsciiComponent::parse(name.as_bytes()).unwrap()
}

/// Writes a durable leaf the way a payload writer would: create, write, flush
/// the handle, flush the parent.
pub(super) fn write_leaf(parent: &Dir, name: &str, bytes: &[u8]) {
    let mut file = parent.create(OsStr::new(name)).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);
    sync_directory_edge(parent, "leaf fixture write").unwrap();
}

/// Rewrites the leaf **through the same object**, so the durable identity is
/// unchanged and only the payload moves.
pub(super) fn rewrite_in_place(parent_path: &Path, name: &str, bytes: &[u8]) {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(parent_path.join(name))
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

/// Replaces the leaf with a **different object** carrying identical bytes.
pub(super) fn substitute(parent_path: &Path, name: &str, bytes: &[u8]) {
    fs::remove_file(parent_path.join(name)).unwrap();
    let mut file = fs::File::create(parent_path.join(name)).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

const LEAF: &str = "source-payload-v1";

fn observe_exact(
    parent: &RetainedParentV1,
    name: &str,
    expected: &ExpectedPayloadV1,
    namespace: &mut BarrierNamespaceV1,
) -> Result<DurableLeafProof<DurableObjectIdentityV1>, CheckedFsError> {
    HostLeafObserverV1.observe_durable(
        parent,
        &component(name),
        DurableLeafExpectation::Exact(expected),
        namespace,
        0,
    )
}

fn observe_absent(
    parent: &RetainedParentV1,
    name: &str,
    namespace: &mut BarrierNamespaceV1,
) -> Result<DurableLeafProof<DurableObjectIdentityV1>, CheckedFsError> {
    HostLeafObserverV1.observe_durable::<ExpectedPayloadV1, _>(
        parent,
        &component(name),
        DurableLeafExpectation::Missing,
        namespace,
        0,
    )
}

/// Plan §4 Step 2.1: the bounded read is keyed to the caller's stated budget.
/// A payload one byte past it is refused as a length fact, not truncated and
/// not read into an unbounded buffer.
#[test]
fn bounded_observation_is_keyed_to_the_caller_stated_budget() {
    let fixture = LeafFixture::new("bounded-budget");
    let parent = retain(fixture.parent());
    write_leaf(parent.handle(), LEAF, b"0123456789");

    let LeafProof::Exact {
        length,
        sha256,
        bytes,
        ..
    } = HostLeafObserverV1
        .observe_bounded(&parent, &component(LEAF), 10)
        .unwrap()
    else {
        panic!("a payload exactly at the caller budget is observed exactly");
    };
    assert_eq!(length, 10);
    assert_eq!(bytes, b"0123456789");
    assert_eq!(sha256, <[u8; 32]>::from(Sha256::digest(b"0123456789")));

    assert_eq!(
        HostLeafObserverV1
            .observe_bounded(&parent, &component(LEAF), 9)
            .unwrap(),
        LeafProof::Other(LeafOther::LengthMismatch)
    );
    assert_eq!(
        HostLeafObserverV1
            .observe_bounded(&parent, &component("absent-v1"), 10)
            .unwrap(),
        LeafProof::Missing
    );
}

/// ConsumerCheckpoint §8 (:236-237): payload size is never confused with
/// protocol-record size. A payload far past every frozen record bound is
/// observed exactly when the caller states a budget that admits it, and the
/// production observer names no record kind at all.
#[test]
fn payload_size_is_never_conflated_with_protocol_record_size() {
    let record_bound = ProtocolRecordKindV1::Admission.max_bytes();
    let payload = vec![0xA5_u8; record_bound * 4 + 7];
    let fixture = LeafFixture::new("payload-vs-record");
    let parent = retain(fixture.parent());
    write_leaf(parent.handle(), LEAF, &payload);

    let LeafProof::Exact { length, .. } = HostLeafObserverV1
        .observe_bounded(&parent, &component(LEAF), payload.len())
        .unwrap()
    else {
        panic!("a payload larger than every protocol record bound is still a payload");
    };
    assert_eq!(length, payload.len() as u64);

    let expected = ExpectedPayloadV1::new(payload);
    let mut namespace = BarrierNamespaceV1::default();
    assert!(
        observe_exact(&parent, LEAF, &expected, &mut namespace)
            .unwrap()
            .is_exact_durable()
    );

    let source = include_str!("leaf_observation.rs");
    for required in ["max_bytes", "try_reserve_exact", "sync_all"] {
        assert!(
            source.contains(required),
            "the leaf observer lost its bounded, fallible, flushed shape: {required}"
        );
    }
    for forbidden in ["ProtocolRecordKindV1", "read_to_string", "fs::read"] {
        assert!(
            !source.contains(forbidden),
            "the leaf observer keyed a payload read to a protocol record bound or an \
             unbounded read: {forbidden}"
        );
    }
}

/// Plan §4 Step 2.1 and `leaf.rs:54-56`: the expected content is opened once
/// per side of the barrier and never rewound, and the barrier is crossed
/// exactly once.
#[test]
fn a_durable_observation_opens_the_expectation_twice_and_crosses_one_barrier() {
    let fixture = LeafFixture::new("two-sided");
    let parent = retain(fixture.parent());
    let payload = b"gwz-r2d-durable-leaf-payload".to_vec();
    write_leaf(parent.handle(), LEAF, &payload);

    let expected = ExpectedPayloadV1::new(payload.clone());
    let mut namespace = BarrierNamespaceV1::default();
    let proof = observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap();

    let DurableLeafProof::ExactDurable {
        identity,
        length,
        sha256,
    } = proof
    else {
        panic!("an exact durable leaf proves exactly");
    };
    assert_eq!(length, payload.len() as u64);
    assert_eq!(sha256, <[u8; 32]>::from(Sha256::digest(&payload)));
    assert_eq!(
        &identity,
        HostPlatform
            .file_identity(&parent.handle().open(OsStr::new(LEAF)).unwrap())
            .unwrap()
            .durable()
    );
    assert_eq!(
        expected.opens(),
        2,
        "each side of the barrier opens its own reader"
    );
    assert_eq!(namespace.crossed(), 1);
}

/// The two-sided durable-absence proof: absent before the barrier and absent
/// after it is `MissingDurable`; a leaf that appears across the barrier is not.
#[test]
fn durable_absence_is_proven_on_both_sides_of_the_barrier() {
    let fixture = LeafFixture::new("absence");
    let parent = retain(fixture.parent());
    let mut namespace = BarrierNamespaceV1::default();
    assert_eq!(
        observe_absent(&parent, LEAF, &mut namespace).unwrap(),
        DurableLeafProof::MissingDurable
    );
    assert_eq!(namespace.crossed(), 1);

    let appearing = fixture.parent_path();
    run_next_at(Fault::DurableLeafNamespaceBarrier, move || {
        let mut file = fs::File::create(appearing.join(LEAF)).unwrap();
        file.write_all(b"appeared").unwrap();
        file.sync_all().unwrap();
    });
    assert_eq!(
        observe_absent(&parent, LEAF, &mut namespace).unwrap(),
        DurableLeafProof::Other(LeafOther::NameChanged),
        "a leaf that appears across the barrier breaks the second side of the absence proof"
    );
}

/// An expectation that contradicts how the name resolves is typed, never
/// silently exact and never silently absent.
#[test]
fn an_expectation_that_contradicts_the_name_is_typed() {
    let fixture = LeafFixture::new("contradiction");
    let parent = retain(fixture.parent());
    let mut namespace = BarrierNamespaceV1::default();
    let expected = ExpectedPayloadV1::new(b"never written".to_vec());
    assert_eq!(
        observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap(),
        DurableLeafProof::Other(LeafOther::NameChanged)
    );

    write_leaf(parent.handle(), LEAF, b"resident");
    assert_eq!(
        observe_absent(&parent, LEAF, &mut namespace).unwrap(),
        DurableLeafProof::Other(LeafOther::NameChanged)
    );
    assert_eq!(
        namespace.crossed(),
        0,
        "a contradicted expectation crosses no durable edge"
    );
}

/// Identity stability (E11): a same-byte substitution across the barrier is
/// caught by the retained handle, which the substituted name no longer names.
#[test]
fn a_same_byte_substitution_across_the_barrier_is_refused() {
    let fixture = LeafFixture::new("substituted");
    let parent = retain(fixture.parent());
    let payload = b"gwz-r2d-same-bytes".to_vec();
    write_leaf(parent.handle(), LEAF, &payload);

    let parent_path = fixture.parent_path();
    let substituted = payload.clone();
    run_next_at(Fault::DurableLeafFileFlush, move || {
        substitute(&parent_path, LEAF, &substituted);
    });

    let expected = ExpectedPayloadV1::new(payload);
    let mut namespace = BarrierNamespaceV1::default();
    assert_eq!(
        observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap(),
        DurableLeafProof::Other(LeafOther::Substituted)
    );
}

/// Name stability (E11): a leaf removed across the barrier stops resolving,
/// even though the observer still holds the object open.
#[test]
fn a_leaf_removed_across_the_barrier_is_refused() {
    let fixture = LeafFixture::new("removed");
    let parent = retain(fixture.parent());
    let payload = b"gwz-r2d-removed".to_vec();
    write_leaf(parent.handle(), LEAF, &payload);

    let parent_path = fixture.parent_path();
    run_next_at(Fault::DurableLeafNamespaceBarrier, move || {
        fs::remove_file(parent_path.join(LEAF)).unwrap();
    });

    let expected = ExpectedPayloadV1::new(payload);
    let mut namespace = BarrierNamespaceV1::default();
    assert_eq!(
        observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap(),
        DurableLeafProof::Other(LeafOther::NameChanged)
    );
}

/// Kind and mode facts survive the barrier (E11): a leaf whose kind, link, or
/// mode changes across the barrier reports the fact that moved, exactly as the
/// pre-barrier route does — `NameChanged` stays reserved for a name that
/// stopped resolving, which the test above pins.
#[test]
fn a_kind_or_mode_swap_across_the_barrier_keeps_its_typed_fact() {
    type Swap = fn(&Path);

    let replace_with_directory: Swap = |root| {
        fs::remove_file(root.join(LEAF)).unwrap();
        fs::create_dir(root.join(LEAF)).unwrap();
    };
    let mut cases: Vec<(&str, Swap, LeafOther)> =
        vec![("directory", replace_with_directory, LeafOther::WrongKind)];
    #[cfg(unix)]
    {
        let replace_with_symlink: Swap = |root| {
            fs::write(root.join("a-target-v1"), b"gwz-r2d-kind-swap").unwrap();
            fs::remove_file(root.join(LEAF)).unwrap();
            std::os::unix::fs::symlink("a-target-v1", root.join(LEAF)).unwrap();
        };
        let make_executable: Swap = |root| {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join(LEAF), fs::Permissions::from_mode(0o755)).unwrap();
        };
        cases.push(("symlink", replace_with_symlink, LeafOther::Substituted));
        cases.push(("executable", make_executable, LeafOther::Executable));
    }

    for (label, swap, other) in cases {
        let fixture = LeafFixture::new(&format!("kind-swap-{label}"));
        let parent = retain(fixture.parent());
        let payload = b"gwz-r2d-kind-swap".to_vec();
        write_leaf(parent.handle(), LEAF, &payload);

        let parent_path = fixture.parent_path();
        run_next_at(Fault::DurableLeafNamespaceBarrier, move || {
            swap(&parent_path);
        });

        let expected = ExpectedPayloadV1::new(payload);
        let mut namespace = BarrierNamespaceV1::default();
        assert_eq!(
            observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap(),
            DurableLeafProof::Other(other),
            "{label}: the post-barrier reobservation must name the fact that moved"
        );
    }
}

/// Length and content stability (E11): an in-place rewrite keeps the durable
/// identity and still fails, on whichever of the two facts moved.
#[test]
fn an_in_place_rewrite_across_the_barrier_is_refused_on_length_then_on_content() {
    for (label, replacement, other) in [
        (
            "shorter",
            b"gwz-r2d-short".to_vec(),
            LeafOther::LengthMismatch,
        ),
        (
            "same-length",
            b"gwz-r2d-rewritten".to_vec(),
            LeafOther::ContentMismatch,
        ),
    ] {
        let fixture = LeafFixture::new(&format!("rewrite-{label}"));
        let parent = retain(fixture.parent());
        let payload = b"gwz-r2d-original!".to_vec();
        assert_eq!(payload.len(), 17);
        write_leaf(parent.handle(), LEAF, &payload);

        let parent_path = fixture.parent_path();
        run_next_at(Fault::DurableLeafNamespaceBarrier, move || {
            rewrite_in_place(&parent_path, LEAF, &replacement);
        });

        let expected = ExpectedPayloadV1::new(payload);
        let mut namespace = BarrierNamespaceV1::default();
        assert_eq!(
            observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap(),
            DurableLeafProof::Other(other),
            "{label}: the post-barrier reobservation must name the fact that moved"
        );
    }
}

/// Parent stability (E11): a retained parent whose bound identity is not the
/// identity the handle carries is refused before any proof is issued.
#[test]
fn a_foreign_retained_parent_is_refused() {
    let fixture = LeafFixture::new("foreign-parent");
    let foreign = LeafFixture::new("foreign-other");
    let payload = b"gwz-r2d-foreign".to_vec();
    write_leaf(&fixture.parent(), LEAF, &payload);

    let foreign_identity = HostPlatform
        .dir_identity(&foreign.parent())
        .unwrap()
        .durable()
        .clone();
    let parent = retain_as(fixture.parent(), foreign_identity);

    let expected = ExpectedPayloadV1::new(payload);
    let mut namespace = BarrierNamespaceV1::default();
    assert_eq!(
        observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap(),
        DurableLeafProof::Other(LeafOther::ParentChanged)
    );
}

/// A pre-barrier payload disagreement never reaches the durable edges.
#[test]
fn a_payload_that_disagrees_before_the_barrier_crosses_no_durable_edge() {
    let fixture = LeafFixture::new("pre-barrier-disagreement");
    let parent = retain(fixture.parent());
    write_leaf(parent.handle(), LEAF, b"gwz-r2d-resident");

    for (expectation, other) in [
        (b"gwz-r2d-residen".to_vec(), LeafOther::LengthMismatch),
        (b"gwz-r2d-resideNT".to_vec(), LeafOther::ContentMismatch),
    ] {
        let expected = ExpectedPayloadV1::new(expectation);
        let mut namespace = BarrierNamespaceV1::default();
        assert_eq!(
            observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap(),
            DurableLeafProof::Other(other)
        );
        assert_eq!(namespace.crossed(), 0);
    }
}

/// Kind and mode facts are typed refusals on both observation routes.
#[test]
fn a_non_canonical_leaf_is_typed_on_both_routes() {
    let fixture = LeafFixture::new("kinds");
    let parent = retain(fixture.parent());
    let root = fixture.parent_path();
    fs::create_dir(root.join("a-directory-v1")).unwrap();
    write_leaf(parent.handle(), "a-target-v1", b"target");
    #[cfg(unix)]
    std::os::unix::fs::symlink("a-target-v1", root.join("a-symlink-v1")).unwrap();

    let mut cases = vec![("a-directory-v1", LeafOther::WrongKind)];
    #[cfg(unix)]
    cases.push(("a-symlink-v1", LeafOther::Substituted));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        write_leaf(parent.handle(), "an-executable-v1", b"target");
        fs::set_permissions(
            root.join("an-executable-v1"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        cases.push(("an-executable-v1", LeafOther::Executable));
    }

    for (name, other) in cases {
        assert_eq!(
            HostLeafObserverV1
                .observe_bounded(&parent, &component(name), 64)
                .unwrap(),
            LeafProof::Other(other),
            "{name}: bounded observation must type the fact"
        );
        let expected = ExpectedPayloadV1::new(b"target".to_vec());
        let mut namespace = BarrierNamespaceV1::default();
        assert_eq!(
            observe_exact(&parent, name, &expected, &mut namespace).unwrap(),
            DurableLeafProof::Other(other),
            "{name}: durable observation must type the fact"
        );
        assert_eq!(namespace.crossed(), 0);
    }
}

/// A zero-byte payload is an ordinary exact observation on both routes; the
/// bounded route never allocates past the caller's budget for it.
#[test]
fn an_empty_payload_is_exact_on_both_routes() {
    let fixture = LeafFixture::new("empty");
    let parent = retain(fixture.parent());
    write_leaf(parent.handle(), LEAF, b"");

    let LeafProof::Exact { length, bytes, .. } = HostLeafObserverV1
        .observe_bounded(&parent, &component(LEAF), 0)
        .unwrap()
    else {
        panic!("an empty payload is exactly observable");
    };
    assert_eq!(length, 0);
    assert!(bytes.is_empty());

    let expected = ExpectedPayloadV1::new(Vec::new());
    let mut namespace = BarrierNamespaceV1::default();
    assert!(
        observe_exact(&parent, LEAF, &expected, &mut namespace)
            .unwrap()
            .is_exact_durable()
    );
    assert_eq!(expected.opens(), 2);
}

/// A payload larger than one streaming chunk is compared chunk by chunk on
/// both sides of the barrier, so no whole-payload buffer is materialised for
/// the durable route.
#[test]
fn a_multi_chunk_payload_streams_on_both_sides_of_the_barrier() {
    let fixture = LeafFixture::new("multi-chunk");
    let parent = retain(fixture.parent());
    let payload = (0..40_000_u32)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    write_leaf(parent.handle(), LEAF, &payload);

    let expected = ExpectedPayloadV1::new(payload.clone());
    let mut namespace = BarrierNamespaceV1::default();
    let DurableLeafProof::ExactDurable { length, sha256, .. } =
        observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap()
    else {
        panic!("a multi-chunk payload is exactly observable");
    };
    assert_eq!(length, payload.len() as u64);
    assert_eq!(sha256, <[u8; 32]>::from(Sha256::digest(&payload)));

    let mut truncated = payload.clone();
    truncated.truncate(payload.len() - 1);
    let short = ExpectedPayloadV1::new(truncated);
    assert_eq!(
        observe_exact(&parent, LEAF, &short, &mut namespace).unwrap(),
        DurableLeafProof::Other(LeafOther::LengthMismatch)
    );

    let mut flipped = payload;
    let last = flipped.len() - 1;
    flipped[last] ^= 0xFF;
    let diverged = ExpectedPayloadV1::new(flipped);
    assert_eq!(
        observe_exact(&parent, LEAF, &diverged, &mut namespace).unwrap(),
        DurableLeafProof::Other(LeafOther::ContentMismatch)
    );
}

/// The observation is a read: repeating it leaves the parent's row set and the
/// leaf's identity and bytes exactly as they were.
#[test]
fn a_repeated_observation_mutates_nothing() {
    let fixture = LeafFixture::new("no-mutation");
    let parent = retain(fixture.parent());
    let payload = b"gwz-r2d-immutable".to_vec();
    write_leaf(parent.handle(), LEAF, &payload);

    let before = census(&fixture.parent_path());
    let expected = ExpectedPayloadV1::new(payload.clone());
    let mut namespace = BarrierNamespaceV1::default();
    let first = observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap();
    let again = observe_exact(&parent, LEAF, &expected, &mut namespace).unwrap();

    assert_eq!(first, again);
    assert_eq!(census(&fixture.parent_path()), before);
    assert_eq!(fs::read(fixture.parent_path().join(LEAF)).unwrap(), payload);
}

/// Sorted child names plus each child's byte length — the durable facts a read
/// path must leave untouched.
pub(super) fn census(directory: &Path) -> Vec<(String, u64)> {
    let mut rows = fs::read_dir(directory)
        .map(|entries| {
            entries
                .map(|entry| {
                    let entry = entry.unwrap();
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        entry.metadata().unwrap().len(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    rows.sort();
    rows
}

/// A reader that hands back one byte at a time, proving the streamed
/// comparison tolerates short reads rather than assuming a full buffer.
struct DribbleReader(Cursor<Vec<u8>>);

impl Read for DribbleReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.0.read(&mut buffer[..1])
    }
}

struct DribblePayloadV1(Vec<u8>);

impl ExpectedLeafContent for DribblePayloadV1 {
    type Reader = DribbleReader;

    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn sha256(&self) -> [u8; 32] {
        Sha256::digest(&self.0).into()
    }

    fn open(&self) -> io::Result<Self::Reader> {
        Ok(DribbleReader(Cursor::new(self.0.clone())))
    }
}

#[test]
fn a_short_reading_expectation_still_compares_exactly() {
    let fixture = LeafFixture::new("dribble");
    let parent = retain(fixture.parent());
    let payload = b"gwz-r2d-short-reads-are-not-divergence".to_vec();
    write_leaf(parent.handle(), LEAF, &payload);

    let mut namespace = BarrierNamespaceV1::default();
    let proof = HostLeafObserverV1
        .observe_durable(
            &parent,
            &component(LEAF),
            DurableLeafExpectation::Exact(&DribblePayloadV1(payload.clone())),
            &mut namespace,
            0,
        )
        .unwrap();
    assert!(proof.is_exact_durable());
}
