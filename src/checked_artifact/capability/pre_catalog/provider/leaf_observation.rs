//! Owner-private production `LeafObserver`: bounded ordinary observation, and
//! same-handle durable observation across the namespace barrier.
//!
//! The seam this file implements is frozen and unchanged
//! (`GwzM5-8R2DInterfaceFreeze.md` §3.3; `checked_artifact/leaf.rs:74-101`).
//! Four properties are structural here rather than advisory:
//!
//! * **Bounded, fallible reads.** Every payload read is capped by a bound the
//!   *caller* states — `max_bytes` for the ordinary route, the expectation's own
//!   length for the durable route — and the only whole-payload buffer is a
//!   `try_reserve_exact` allocation of exactly that bound. The durable route
//!   materialises nothing: it streams in fixed chunks. This file names no
//!   protocol record kind, so a payload bound can never be a record bound
//!   (ConsumerCheckpoint §8 :236-237).
//! * **One retained handle.** `observe_durable` opens the leaf once and owns
//!   that handle across exact proof, flush, namespace barrier, and exact
//!   reobservation. The post-barrier checks read the retained handle and
//!   cross-check the name against it; they never re-derive the proof from the
//!   name alone.
//! * **Two-sided proofs.** Both `ExactDurable` and `MissingDurable` are proven
//!   before *and* after the barrier. An expectation the name contradicts is a
//!   typed `LeafOther`, never a silent proof.
//! * **No mutation.** The only durable edges are the leaf flush (edge E9,
//!   primitive family P2) and the scheduled namespace barrier (edge E10,
//!   family P5). Opening and identity use the no-follow open plus durable
//!   identity compare (edges E8/E11, family P3). Nothing here creates,
//!   renames, or removes a name (`GwzM5-8R2DInterfaceFreeze.md` §4.3). E9 is
//!   platform-routed because the observation handle is read-only: see
//!   [`flush_observed_leaf`], whose Windows arm states the durability property
//!   that platform relies on in place of a handle flush.

use std::ffi::OsStr;
use std::io::{self, Read, Seek, SeekFrom};

#[cfg(unix)]
use cap_fs_ext::OsMetadataExt;
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, Metadata, OpenOptions};
use sha2::{Digest, Sha256};

use super::retained::encode_identity;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableIdentityProvider,
    DurableObjectIdentityV1, PlatformCapability,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::leaf::{
    DurableLeafExpectation, DurableLeafProof, ExpectedLeafContent, LeafObserver, LeafOther,
    LeafProof,
};
use crate::checked_artifact::namespace::{NamespaceProtocol, RetainedDirectory};

/// The fixed streaming window of the durable route. It bounds the observer's
/// stack footprint independently of payload size, so a payload is never
/// materialised to be compared.
const PAYLOAD_CHUNK_BYTES: usize = 8 * 1024;

/// The retained parent capability both routes observe through.
type RetainedParentV1 = RetainedDirectory<Dir, DurableObjectIdentityV1, CanonicalPathIdentityV1>;

/// The production leaf observer. It carries no state: every fact it proves is
/// re-derived from the retained parent capability it is handed.
pub(in crate::checked_artifact) struct HostLeafObserverV1;

impl LeafObserver for HostLeafObserverV1 {
    type DirectoryHandle = Dir;
    type Identity = DurableObjectIdentityV1;
    type PathProfile = CanonicalPathIdentityV1;

    fn observe_bounded(
        &self,
        parent: &RetainedParentV1,
        leaf: &AsciiComponent,
        max_bytes: usize,
    ) -> Result<LeafProof<DurableObjectIdentityV1>, CheckedFsError> {
        let retained = match open_retained_leaf(parent.handle(), leaf)? {
            OpenedLeafV1::Absent => return Ok(LeafProof::Missing),
            OpenedLeafV1::Other(other) => return Ok(LeafProof::Other(other)),
            OpenedLeafV1::Exact(retained) => retained,
        };
        let mut file = retained.file;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(max_bytes).map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "bounded leaf observation allocation failed",
            )
        })?;
        Ok(match retain_payload(&mut file, max_bytes, &mut bytes)? {
            PayloadOutcomeV1::Refused(other) => LeafProof::Other(other),
            PayloadOutcomeV1::Exact(fingerprint) => LeafProof::Exact {
                identity: retained.durable,
                length: fingerprint.length,
                sha256: fingerprint.sha256,
                bytes,
            },
        })
    }

    fn observe_durable<Content, Protocol>(
        &self,
        parent: &RetainedParentV1,
        leaf: &AsciiComponent,
        expected: DurableLeafExpectation<'_, Content>,
        namespace: &mut Protocol,
        barrier_ordinal: Protocol::BarrierOrdinal,
    ) -> Result<DurableLeafProof<DurableObjectIdentityV1>, CheckedFsError>
    where
        Content: ExpectedLeafContent + ?Sized,
        Protocol: NamespaceProtocol<
                DirectoryHandle = Dir,
                Identity = DurableObjectIdentityV1,
                PathProfile = CanonicalPathIdentityV1,
            >,
    {
        match (expected, open_retained_leaf(parent.handle(), leaf)?) {
            (DurableLeafExpectation::Exact(content), OpenedLeafV1::Exact(retained)) => {
                observe_exact_durable(parent, leaf, content, retained, namespace, barrier_ordinal)
            }
            (DurableLeafExpectation::Missing, OpenedLeafV1::Absent) => {
                observe_durable_absence(parent, leaf, namespace, barrier_ordinal)
            }
            (_, OpenedLeafV1::Other(other)) => Ok(DurableLeafProof::Other(other)),
            // The caller's expectation and the way the name resolves disagree.
            // That is a fact about the *name*, on either side of the
            // disagreement, and it is reported before any durable edge is
            // crossed: an exact expectation over a name that does not resolve,
            // and an absence expectation over a name that does, are the same
            // refusal.
            (DurableLeafExpectation::Exact(_), OpenedLeafV1::Absent)
            | (DurableLeafExpectation::Missing, OpenedLeafV1::Exact(_)) => {
                Ok(DurableLeafProof::Other(LeafOther::NameChanged))
            }
        }
    }
}

/// Edges E8-E11 for a resident leaf: exact proof, flush, barrier, then exact
/// reobservation of parent, name, identity, length, and content — every
/// post-barrier fact read through the one handle opened before it.
fn observe_exact_durable<Content, Protocol>(
    parent: &RetainedParentV1,
    leaf: &AsciiComponent,
    content: &Content,
    retained: RetainedLeafV1,
    namespace: &mut Protocol,
    barrier_ordinal: Protocol::BarrierOrdinal,
) -> Result<DurableLeafProof<DurableObjectIdentityV1>, CheckedFsError>
where
    Content: ExpectedLeafContent + ?Sized,
    Protocol: NamespaceProtocol<
            DirectoryHandle = Dir,
            Identity = DurableObjectIdentityV1,
            PathProfile = CanonicalPathIdentityV1,
        >,
{
    let RetainedLeafV1 {
        mut file,
        identity,
        durable,
    } = retained;

    // E8 (P3 + P2): the first exact proof streams the payload through the one
    // retained handle, against its own freshly opened expected reader.
    let first = match compare_payload(&mut file, content, ContentPhaseV1::First)? {
        PayloadOutcomeV1::Refused(other) => return Ok(DurableLeafProof::Other(other)),
        PayloadOutcomeV1::Exact(fingerprint) => fingerprint,
    };

    // E9 (P2): the observed object is durable before the barrier orders it.
    flush_observed_leaf(&file)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::DurableLeafFileFlush);

    // E10 (P5).
    cross_namespace_barrier(namespace, parent, barrier_ordinal)?;

    // E11 (P3 + P4), reobservation through the same parent.
    if !parent_is_unchanged(parent)? {
        return Ok(DurableLeafProof::Other(LeafOther::ParentChanged));
    }
    let named = open_leaf(parent.handle(), leaf)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::DurableLeafNameRevalidate);
    // The post-barrier reobservation names the fact that moved, exactly as the
    // pre-barrier one does: a kind, mode, or link swap keeps its own typed
    // fact, and `NameChanged` is reserved for a name that stopped resolving.
    // This mirrors the absence arm below, which already passes its typed facts
    // through.
    let named = match named {
        OpenedLeafV1::Exact(named) => named,
        OpenedLeafV1::Absent => return Ok(DurableLeafProof::Other(LeafOther::NameChanged)),
        OpenedLeafV1::Other(other) => return Ok(DurableLeafProof::Other(other)),
    };

    // Identity stability, both ways: the retained handle is still the object it
    // was, and the name still resolves to that same object. Comparing the
    // encoded pair rather than the durable half alone refuses a same-name
    // substitution even where the durable identity is reused.
    let stable_identity = named.identity == identity
        && encode_identity(&super::HostPlatform.file_identity(&file)?) == identity;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::DurableLeafHandleRevalidate);
    if !stable_identity {
        return Ok(DurableLeafProof::Other(LeafOther::Substituted));
    }

    // Length stability, read from the retained handle rather than from the
    // name. This is a cheap `stat`-side cross-check of a fact the streamed
    // comparison below also proves, kept for two reasons: it refuses a resized
    // payload in constant work instead of streaming the whole object to
    // discover the same thing, and it proves the fact through a different
    // syscall than the one that produced it.
    let observed_length = file
        .metadata()
        .map_err(|source| CheckedFsError::io("reobserve the durable leaf length", source))?
        .len();
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::DurableLeafLengthRevalidate);
    if observed_length != first.length {
        return Ok(DurableLeafProof::Other(LeafOther::LengthMismatch));
    }

    // Content stability: the payload is streamed a second time through the same
    // retained handle, against a *second* freshly opened expected reader — the
    // observer never assumes one reader can be rewound (`leaf.rs:54-56`).
    let revalidated = match compare_payload(&mut file, content, ContentPhaseV1::Revalidated)? {
        PayloadOutcomeV1::Refused(other) => return Ok(DurableLeafProof::Other(other)),
        PayloadOutcomeV1::Exact(fingerprint) => fingerprint,
    };
    // Like the length cross-check above, this is subsumed on the ordinary path:
    // `compare_payload` returns `Exact` only after pinning each stream to the
    // expectation's own declared length and digest, so for a `Content` whose
    // declarations are stable across its two `open()` calls the two
    // fingerprints are equal by construction. It is kept because that
    // stability is the expectation's promise, not this observer's: a `Content`
    // whose `len()`/`sha256()` drift between the two sides would let both
    // streams pass their own check while describing different payloads, and
    // only comparing the two fingerprints catches it.
    if revalidated != first {
        return Ok(DurableLeafProof::Other(LeafOther::ContentMismatch));
    }

    Ok(DurableLeafProof::ExactDurable {
        identity: durable,
        length: first.length,
        sha256: first.sha256,
    })
}

/// The matching two-sided absence proof: absent through the retained parent
/// before the barrier, and absent through that same parent after it. Anything
/// resident on the second side is a fact about the name, not a proof.
fn observe_durable_absence<Protocol>(
    parent: &RetainedParentV1,
    leaf: &AsciiComponent,
    namespace: &mut Protocol,
    barrier_ordinal: Protocol::BarrierOrdinal,
) -> Result<DurableLeafProof<DurableObjectIdentityV1>, CheckedFsError>
where
    Protocol: NamespaceProtocol<
            DirectoryHandle = Dir,
            Identity = DurableObjectIdentityV1,
            PathProfile = CanonicalPathIdentityV1,
        >,
{
    cross_namespace_barrier(namespace, parent, barrier_ordinal)?;
    if !parent_is_unchanged(parent)? {
        return Ok(DurableLeafProof::Other(LeafOther::ParentChanged));
    }
    let reobserved = open_leaf(parent.handle(), leaf)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::DurableLeafMissingRevalidate);
    Ok(match reobserved {
        OpenedLeafV1::Absent => DurableLeafProof::MissingDurable,
        OpenedLeafV1::Exact(_) => DurableLeafProof::Other(LeafOther::NameChanged),
        OpenedLeafV1::Other(other) => DurableLeafProof::Other(other),
    })
}

/// Edge E9 (primitive family P2), platform-routed on the same pattern the P2
/// family already uses for `sync_parent` (`platform.rs`; interface freeze §4.1
/// P2 row): one arm per platform, and the arm that cannot perform the flush
/// states which durability property carries the edge instead.
///
/// The split exists because the observation handle is **read-only by design** —
/// an observer must not demand write access to the artifact it observes, and a
/// genuinely read-only artifact must remain observable — and the two platforms
/// treat a read-only handle differently.
#[cfg(not(windows))]
fn flush_observed_leaf(file: &File) -> Result<(), CheckedFsError> {
    // `fsync` is legal on an `O_RDONLY` file descriptor, so the observed object
    // is ordered to disk through the very handle the proof was taken from. This
    // is a regular-file descriptor, not a cap-std directory capability, so the
    // Linux `O_PATH`/`EBADF` substrate that `platform::sync_parent` reopens
    // around does not apply here.
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush the observed durable leaf", source))
}

#[cfg(windows)]
fn flush_observed_leaf(_file: &File) -> Result<(), CheckedFsError> {
    // `FlushFileBuffers` requires the handle to hold `GENERIC_WRITE`, which a
    // read-only observation handle does not, so it would fail with
    // `ERROR_ACCESS_DENIED` rather than order anything. A handle flush is
    // therefore unavailable on this platform, exactly as a directory flush is
    // for `sync_parent`.
    //
    // The property the Windows path relies on instead is the P2 family's own,
    // stated once for `sync_parent` and unchanged here: the leaf's *writer*
    // opened it through `durable_write_options`, which sets
    // `FILE_FLAG_WRITE_THROUGH` (`directory_mutation.rs`), so the bytes this
    // observation reads are already through the cache before the observation
    // begins — there is no unflushed writer state for the observer to order.
    // What the observer still owes the caller is ordering against what follows,
    // and that is edge E10's scheduled namespace barrier — family P5, whose
    // Windows column is the anchor round trip for the anchored private area and
    // the writer-class-conditional arm documented at
    // `platform::private_barrier` for an exact interior, never a directory
    // fsync. So the observer adds no ordering of its own here, deliberately and
    // by argument, instead of failing to add any.
    Ok(())
}

/// Edge E10: the barrier is the scheduled namespace protocol's, never a
/// platform call this observer makes for itself.
fn cross_namespace_barrier<Protocol>(
    namespace: &mut Protocol,
    parent: &RetainedParentV1,
    barrier_ordinal: Protocol::BarrierOrdinal,
) -> Result<(), CheckedFsError>
where
    Protocol: NamespaceProtocol<
            DirectoryHandle = Dir,
            Identity = DurableObjectIdentityV1,
            PathProfile = CanonicalPathIdentityV1,
        >,
{
    let _durable = namespace.barrier(parent, barrier_ordinal)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::DurableLeafNamespaceBarrier);
    Ok(())
}

/// Edge E11: the retained parent handle still carries the durable identity the
/// capability was issued against, reobserved after the barrier.
fn parent_is_unchanged(parent: &RetainedParentV1) -> Result<bool, CheckedFsError> {
    let observed = super::HostPlatform.dir_identity(parent.handle())?;
    let unchanged = observed.durable() == parent.identity();
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::DurableLeafParentRevalidate);
    Ok(unchanged)
}

/// The one retained leaf handle, with both halves of its identity: the durable
/// half the proof carries, and the encoded pair the invocation compares on.
struct RetainedLeafV1 {
    file: File,
    identity: Vec<u8>,
    durable: DurableObjectIdentityV1,
}

enum OpenedLeafV1 {
    Absent,
    Other(LeafOther),
    Exact(RetainedLeafV1),
}

/// The first open of an observation, announcing the two boundaries it crosses.
/// Reobservations use [`open_leaf`] directly, so they announce their own
/// post-barrier boundaries instead of the first ones.
fn open_retained_leaf(parent: &Dir, leaf: &AsciiComponent) -> Result<OpenedLeafV1, CheckedFsError> {
    let opened = open_leaf(parent, leaf)?;
    #[cfg(test)]
    {
        crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::DurableLeafFirstOpen);
        if matches!(opened, OpenedLeafV1::Exact(_)) {
            crate::checked_artifact::fault_v1::hit(
                CheckedArtifactFaultKeyV1::DurableLeafFirstIdentity,
            );
        }
    }
    Ok(opened)
}

/// Edge E8 (P3): classify the name, then open it no-follow and take the
/// object's identity from the open handle — never from the name.
fn open_leaf(parent: &Dir, leaf: &AsciiComponent) -> Result<OpenedLeafV1, CheckedFsError> {
    let name = leaf_name(leaf)?;
    let metadata = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(OpenedLeafV1::Absent);
        }
        Err(source) => return Err(CheckedFsError::io("observe the durable leaf", source)),
    };
    if let Some(other) = non_canonical(&metadata) {
        return Ok(OpenedLeafV1::Other(other));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = match parent.open_with(name, &options) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(OpenedLeafV1::Absent);
        }
        Err(source) => {
            return Err(CheckedFsError::io(
                "open the durable leaf no-follow",
                source,
            ));
        }
    };
    let fact = super::HostPlatform.file_identity(&file)?;
    Ok(OpenedLeafV1::Exact(RetainedLeafV1 {
        durable: fact.durable().clone(),
        identity: encode_identity(&fact),
        file,
    }))
}

/// The leaf grammar a checked artifact admits: a non-executable regular file
/// reached without following a link. The mode rule is the one the legacy leaf
/// observer already applies (`observation.rs:216`, `:366-373`).
fn non_canonical(metadata: &Metadata) -> Option<LeafOther> {
    if metadata.is_symlink() {
        Some(LeafOther::Substituted)
    } else if !metadata.is_file() {
        Some(LeafOther::WrongKind)
    } else if executable(metadata) {
        Some(LeafOther::Executable)
    } else {
        None
    }
}

#[cfg(unix)]
fn executable(metadata: &Metadata) -> bool {
    OsMetadataExt::mode(metadata) & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &Metadata) -> bool {
    false
}

fn leaf_name(leaf: &AsciiComponent) -> Result<&OsStr, CheckedFsError> {
    std::str::from_utf8(leaf.as_bytes())
        .map(OsStr::new)
        .map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::AsciiProtocolPath,
                "leaf component is not an ASCII path component",
            )
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PayloadFingerprintV1 {
    length: u64,
    sha256: [u8; 32],
}

enum PayloadOutcomeV1 {
    Exact(PayloadFingerprintV1),
    Refused(LeafOther),
}

/// Which side of the namespace barrier a streamed payload proof is on. One
/// streaming primitive serves both, so the phase — never a name minted at
/// runtime — selects the stable `durable_leaf.*` boundary it announces
/// (`fault_v1.rs:3-5`; the `AdmissionRecordRowV1::write_faults` idiom at
/// `admission_mutation.rs:349-369`).
#[derive(Clone, Copy)]
enum ContentPhaseV1 {
    First,
    Revalidated,
}

impl ContentPhaseV1 {
    #[cfg(test)]
    const fn fault(self) -> CheckedArtifactFaultKeyV1 {
        match self {
            Self::First => CheckedArtifactFaultKeyV1::DurableLeafFirstContent,
            Self::Revalidated => CheckedArtifactFaultKeyV1::DurableLeafContentRevalidate,
        }
    }

    /// Finalizes a streamed fingerprint and announces this phase's content
    /// boundary. Both routes finish here, so each of the two content keys is
    /// announced from exactly one place.
    fn finish(self, digest: Sha256, length: u64) -> PayloadFingerprintV1 {
        let fingerprint = PayloadFingerprintV1 {
            length,
            sha256: digest.finalize().into(),
        };
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(self.fault());
        fingerprint
    }
}

/// The ordinary bounded read: at most the caller's budget plus the one byte
/// that proves the payload exceeded it, into an allocation already reserved
/// for exactly the budget.
fn retain_payload(
    file: &mut File,
    max_bytes: usize,
    bytes: &mut Vec<u8>,
) -> Result<PayloadOutcomeV1, CheckedFsError> {
    let budget = read_budget(max_bytes as u64)?;
    rewind(file)?;
    file.take(budget)
        .read_to_end(bytes)
        .map_err(|source| CheckedFsError::io("read the bounded leaf payload", source))?;
    if bytes.len() > max_bytes {
        return Ok(PayloadOutcomeV1::Refused(LeafOther::LengthMismatch));
    }
    let fingerprint =
        ContentPhaseV1::First.finish(Sha256::new().chain_update(&*bytes), bytes.len() as u64);
    Ok(PayloadOutcomeV1::Exact(fingerprint))
}

/// The durable streamed comparison: the retained handle and a freshly opened
/// expected reader are walked together in fixed chunks, so neither side is
/// materialised and the first divergence names the fact that moved. The read is
/// bounded by the expectation's own length, which is the caller-stated budget
/// of this route.
fn compare_payload<Content: ExpectedLeafContent + ?Sized>(
    file: &mut File,
    content: &Content,
    phase: ContentPhaseV1,
) -> Result<PayloadOutcomeV1, CheckedFsError> {
    let mut expected = content
        .open()
        .map_err(|source| CheckedFsError::io("open the expected leaf content", source))?;
    let budget = read_budget(content.len())?;
    rewind(file)?;
    let mut observed = file.take(budget);
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut left = [0_u8; PAYLOAD_CHUNK_BYTES];
    let mut right = [0_u8; PAYLOAD_CHUNK_BYTES];
    loop {
        let read = fill(&mut observed, &mut left)
            .map_err(|source| CheckedFsError::io("read the durable leaf payload", source))?;
        let against = fill(&mut expected, &mut right[..])
            .map_err(|source| CheckedFsError::io("read the expected leaf content", source))?;
        if read != against {
            return Ok(PayloadOutcomeV1::Refused(LeafOther::LengthMismatch));
        }
        if left[..read] != right[..read] {
            return Ok(PayloadOutcomeV1::Refused(LeafOther::ContentMismatch));
        }
        if read == 0 {
            break;
        }
        digest.update(&left[..read]);
        length += read as u64;
    }
    let fingerprint = phase.finish(digest, length);
    if fingerprint.length != content.len() {
        return Ok(PayloadOutcomeV1::Refused(LeafOther::LengthMismatch));
    }
    if fingerprint.sha256 != content.sha256() {
        return Ok(PayloadOutcomeV1::Refused(LeafOther::ContentMismatch));
    }
    Ok(PayloadOutcomeV1::Exact(fingerprint))
}

/// The caller's budget plus the single byte that proves it was exceeded. The
/// bound is always stated by the caller — through `max_bytes` or through the
/// expectation's length — and never derived from a protocol record kind.
fn read_budget(max_bytes: u64) -> Result<u64, CheckedFsError> {
    max_bytes.checked_add(1).ok_or_else(|| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "stated leaf payload bound does not fit a bounded read",
        )
    })
}

fn rewind(file: &mut File) -> Result<(), CheckedFsError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind the retained leaf handle", source))?;
    Ok(())
}

/// Fills `buffer` from `reader`, returning how many bytes were available. A
/// short read is not a divergence: only a genuine end of stream is.
fn fill(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

/// Pins the E9 platform split. The arm that runs differs by platform, so each
/// arm asserts its own documented behaviour and the Windows arm additionally
/// pins the constraint it exists for — mirroring the in-file arm test the P2
/// family already carries for `sync_parent` (`platform.rs`, `linux_tests`).
#[cfg(test)]
mod platform_tests {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::ambient_authority;
    use cap_std::fs::{Dir, File, OpenOptions};
    use std::io::Write;

    /// A leaf opened exactly as `open_leaf` opens it: read-only, no-follow.
    fn read_only_leaf(label: &str) -> (std::path::PathBuf, File) {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2d-leaf-e9-{label}-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir(&root).unwrap();
        let parent = Dir::open_ambient_dir(&root, ambient_authority()).unwrap();
        let mut written = parent.create("source-payload-v1").unwrap();
        written.write_all(b"gwz-r2d-e9-payload").unwrap();
        written.sync_all().unwrap();
        drop(written);
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = parent.open_with("source-payload-v1", &options).unwrap();
        (root, file)
    }

    /// Unix arm: the flush really executes, on the read-only handle the proof
    /// was taken from.
    #[cfg(not(windows))]
    #[test]
    fn the_leaf_flush_executes_on_a_read_only_handle() {
        let (root, file) = read_only_leaf("unix");
        let flushed = super::flush_observed_leaf(&file);
        let _ = std::fs::remove_dir_all(&root);
        flushed.expect("fsync is legal on an O_RDONLY descriptor");
    }

    /// Windows arm: the documented no-op, plus the constraint that forces it.
    /// If `FlushFileBuffers` ever accepted a `GENERIC_READ`-only handle the
    /// second assertion goes red, which is the signal to revisit the arm rather
    /// than keep a stale justification.
    #[cfg(windows)]
    #[test]
    fn the_leaf_flush_is_a_documented_no_op_on_a_read_only_handle() {
        let (root, file) = read_only_leaf("windows");
        let routed = super::flush_observed_leaf(&file);
        let raw = file.sync_all();
        let _ = std::fs::remove_dir_all(&root);
        routed.expect("the Windows arm performs no handle flush and cannot fail");
        let raw = raw.expect_err(
            "FlushFileBuffers requires GENERIC_WRITE, so a read-only handle flush must fail",
        );
        assert_eq!(
            raw.raw_os_error(),
            Some(5),
            "the read-only handle flush must fail with ERROR_ACCESS_DENIED, not another error"
        );
    }
}
