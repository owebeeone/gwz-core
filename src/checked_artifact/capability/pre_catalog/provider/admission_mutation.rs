//! Owner-private physical transitions for the action-admission triad.
//!
//! Every namespace edge here runs through the sealed source-associated
//! publication primitive (`GwzM5-8R2CCatalogBootstrapAmendment.md` §4.1, §8.13),
//! and every name is deterministic and indexed — the retry path reuses the same
//! `ActionAdmission*` slot names and the same derived final action name and
//! never allocates a nonce (`GwzM5-8R4bP1P2-RemPlan-4.md` §4 R2 stop clause
//! :1089-1092).
//!
//! The slots are the ones R1+C0 already froze; this file mints no name
//! (`GwzM5-8R2DInterfaceFreeze.md` §3.1 persisted-home pin).

use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom, Write};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use super::directory_mutation::{
    ObservedFileV1, durable_write_options, sync_directory_edge, verify_named_file, verify_open_file,
};
use super::interior;
use super::publication::{
    DestinationRecheckV1, DirectoryInteriorExpectationV1, DirectoryInteriorRecheckV1,
    PublicationSourceV1, publish_verified_no_replace,
};
use super::retained::encode_identity;
use super::{RawCatalogBytesV1, RawCatalogInteriorFactV1, RawCatalogInteriorObservationV1};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1, PlatformCapability,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::{
    ActionAdmissionEdgeV1, ActionAdmissionObservationV1, ActionCapacityReservationV1,
    ActionDirectoryAdmissionV1, ActionSlotV1, BaseActionSlotV1, CatalogAdmissionOccupancyV1,
    CatalogBootstrapRecordV1, CatalogOccupancyV1, InfrastructureSlotV1, ProtocolRecordKindV1,
    RecordObservationV1, RootEntryNameV1, decode_action_directory_admission,
};

/// One complete read-only observation of the admission state resident in the
/// catalog root: `ConsumerCheckpoint` §7 steps 1-2 (derive from read-only
/// observations) and step 8 (reobserve the complete catalog) share this path,
/// so deriving a plan performs no mutation.
pub(super) fn observe(
    final_directory: &Dir,
    expected: &ActionCapacityReservationV1,
) -> Result<ActionAdmissionObservationV1, CheckedFsError> {
    let fresh = interior::observe(final_directory, &super::HostPlatform)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::AdmissionOccupancyObserve);
    let record = admission_record_row(&fresh, InfrastructureSlotV1::ActionAdmissionActive)?;
    let scratch = admission_record_row(&fresh, InfrastructureSlotV1::ActionAdmissionScratch)?;
    let staging = interior::observe_action_directory(
        final_directory,
        OsStr::new(InfrastructureSlotV1::ActionAdmissionStaging.name()),
        expected,
        &super::HostPlatform,
    )?;
    let published = final_action_name(expected);
    let published = interior::observe_action_directory(
        final_directory,
        OsStr::new(published.as_str()),
        expected,
        &super::HostPlatform,
    )?;
    // T1 widening: the retired root's bounded count is now readable, so the
    // observation carries the one term the frozen retirement credit needs. A
    // retired root that does not read as a retired root at all is exactly the
    // fact `completed_record` refuses on, so it is a typed refusal here rather
    // than a silently defaulted zero.
    let retired_action_dirs = interior::retired_action_dirs(&fresh).ok_or_else(|| {
        CheckedFsError::ambiguous(
            "action admission",
            "the catalog's retired-action root is not a bounded retired root",
        )
    })?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::AdmissionCapacityCheck);
    Ok(ActionAdmissionObservationV1 {
        record,
        scratch,
        staging,
        final_directory: published,
        census: fresh.census,
        retired_action_dirs,
    })
}

/// Executes exactly one bounded durable edge of the §7 (:209-221) sequence.
pub(super) fn execute(
    final_directory: &Dir,
    final_identity: &DurableObjectIdentityV1,
    bootstrap: &CatalogBootstrapRecordV1,
    edge: ActionAdmissionEdgeV1<'_>,
    expected: &ActionCapacityReservationV1,
) -> Result<(), CheckedFsError> {
    match edge {
        ActionAdmissionEdgeV1::WriteAdmissionScratch(record) => {
            write_admission_scratch(final_directory, record)
        }
        ActionAdmissionEdgeV1::RetireAdmissionRecord => retire_admission_record(final_directory),
        ActionAdmissionEdgeV1::PublishAdmissionRecord => {
            publish_admission_record(final_directory, final_identity, bootstrap)
        }
        ActionAdmissionEdgeV1::CreateStagingDirectory => create_staging_directory(final_directory),
        ActionAdmissionEdgeV1::WriteResidentReservation => {
            write_resident_reservation(final_directory, expected)
        }
        ActionAdmissionEdgeV1::PublishStagingAction => {
            publish_staging_action(final_directory, final_identity, bootstrap, expected)
        }
    }
}

fn admission_record_row(
    observed: &RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
) -> Result<RecordObservationV1<ActionDirectoryAdmissionV1>, CheckedFsError> {
    let Some(fact) = interior::row(observed, slot) else {
        return Ok(RecordObservationV1::Missing);
    };
    let RawCatalogInteriorFactV1::RegularFile {
        bytes: RawCatalogBytesV1::Bounded(bytes),
        ..
    } = fact
    else {
        return Ok(RecordObservationV1::Other);
    };
    Ok(
        match decode_action_directory_admission(std::io::Cursor::new(bytes)) {
            Ok(value) => RecordObservationV1::Exact(value),
            Err(_) => RecordObservationV1::Other,
        },
    )
}

/// §7 steps 3 and 7, write-ahead half (edge E4, primitive family P2): the next
/// durable admission state is written and flushed into the scratch slot before
/// the active slot is touched, so the active record is never torn.
fn write_admission_scratch(
    final_directory: &Dir,
    record: &ActionDirectoryAdmissionV1,
) -> Result<(), CheckedFsError> {
    let bytes = record.encode_canonical().map_err(|_| {
        CheckedFsError::ambiguous(
            "admission record",
            "admission record is not canonically encodable",
        )
    })?;
    write_durable_record(
        final_directory,
        OsStr::new(InfrastructureSlotV1::ActionAdmissionScratch.name()),
        &bytes,
        AdmissionRecordRowV1::of_admission_record(record),
    )
}

/// §7 steps 3 and 7, install half, part one. The sealed primitive publishes
/// without replacement, so the superseded active record is retired before the
/// scratch is published onto its name. The window this opens is closed by
/// construction: an absent active slot beside an exact scratch resolves to
/// "finish installing the scratch", and an absent active slot beside no
/// scratch is the frozen `ActionDirectoryAdmissionV1::idle()` state itself.
fn retire_admission_record(final_directory: &Dir) -> Result<(), CheckedFsError> {
    final_directory
        .remove_file(OsStr::new(
            InfrastructureSlotV1::ActionAdmissionActive.name(),
        ))
        .map_err(|source| CheckedFsError::io("retire the superseded admission record", source))?;
    sync_directory_edge(final_directory, "flush admission record retirement")
}

/// §7 steps 3 and 7, install half, part two (edge E4, primitive family P1).
fn publish_admission_record(
    final_directory: &Dir,
    final_identity: &DurableObjectIdentityV1,
    bootstrap: &CatalogBootstrapRecordV1,
) -> Result<(), CheckedFsError> {
    let scratch = OsStr::new(InfrastructureSlotV1::ActionAdmissionScratch.name());
    let active = OsStr::new(InfrastructureSlotV1::ActionAdmissionActive.name());
    let (identity, bytes) = observed_record(final_directory, scratch, "admission scratch")?;
    #[cfg(test)]
    let faults = install_faults(&bytes);
    let source = ObservedFileV1 {
        identity: &identity,
        bytes: &bytes,
    };
    publish_verified_no_replace(
        final_directory,
        scratch,
        final_directory,
        active,
        PublicationSourceV1::regular_file(source.identity, source.bytes),
        DestinationRecheckV1::AdmissionCatalogInterior {
            durable_identity: final_identity,
            expected: bootstrap,
            absent: RootEntryNameV1::Infrastructure(InfrastructureSlotV1::ActionAdmissionActive),
        },
        "publish admission record",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(faults[0]);
    verify_named_file(
        final_directory,
        active,
        source,
        "published admission record",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(faults[1]);
    sync_directory_edge(final_directory, "flush admission record publication")
}

/// §7 step 4 (edge E1): the one indexed staging action directory. `create_dir`
/// is itself no-replace, so a resumed attempt reuses the same deterministic
/// name rather than choosing a fresh one.
fn create_staging_directory(final_directory: &Dir) -> Result<(), CheckedFsError> {
    let staging = OsStr::new(InfrastructureSlotV1::ActionAdmissionStaging.name());
    final_directory
        .create_dir(staging)
        .map_err(|source| CheckedFsError::io("create admission staging no-replace", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::AdmissionStagingCreate);
    let directory = final_directory
        .open_dir_nofollow(staging)
        .map_err(|source| CheckedFsError::io("reopen admission staging", source))?;
    super::HostPlatform.dir_identity(&directory)?;
    sync_directory_edge(final_directory, "flush admission staging creation")
}

/// §7 step 5 (edge E2): the complete derived capacity is resident and flushed
/// before the action is published, so §7 (:223-224)'s "capacity includes all
/// barrier, managed-generation, marker, cleanup, and terminal retirement slots
/// before the first action mutation" holds at the moment of publication.
fn write_resident_reservation(
    final_directory: &Dir,
    expected: &ActionCapacityReservationV1,
) -> Result<(), CheckedFsError> {
    let staging = final_directory
        .open_dir_nofollow(OsStr::new(
            InfrastructureSlotV1::ActionAdmissionStaging.name(),
        ))
        .map_err(|source| CheckedFsError::io("open admission staging", source))?;
    let bytes = expected.encode_canonical().map_err(|_| {
        CheckedFsError::ambiguous(
            "action capacity reservation",
            "capacity record is not canonically encodable",
        )
    })?;
    let name = ActionSlotV1::Base(BaseActionSlotV1::Reservation).name(expected.action_digest());
    write_durable_record(
        &staging,
        OsStr::new(name.as_str()),
        &bytes,
        AdmissionRecordRowV1::Reservation,
    )
}

/// §7 step 6 (edge E3, primitive family P1): the staging action directory is
/// published onto the deterministic final action name without replacement,
/// carrying both C-2 arms — the admission source-interior expectation and the
/// admission destination expectation.
fn publish_staging_action(
    final_directory: &Dir,
    final_identity: &DurableObjectIdentityV1,
    bootstrap: &CatalogBootstrapRecordV1,
    expected: &ActionCapacityReservationV1,
) -> Result<(), CheckedFsError> {
    let staging_name = OsStr::new(InfrastructureSlotV1::ActionAdmissionStaging.name());
    let published = final_action_name(expected);
    let staging = final_directory
        .open_dir_nofollow(staging_name)
        .map_err(|source| CheckedFsError::io("open admission staging", source))?;
    let fact = super::HostPlatform.dir_identity(&staging)?;
    let identity = encode_identity(&fact);
    let durable_identity = fact.durable().clone();
    sync_directory_edge(&staging, "flush exact admission staging")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::AdmissionStagingFlush);
    // Release the caller's staging capability before the rename edge: on
    // Windows a directory rename fails with a sharing violation while any
    // handle into the source tree survives, and the sealed primitive
    // re-establishes source identity and interior through its own
    // capabilities (the `publish_final_directory` precedent,
    // `directory_mutation.rs:237-243`).
    drop(staging);
    publish_verified_no_replace(
        final_directory,
        staging_name,
        final_directory,
        OsStr::new(published.as_str()),
        PublicationSourceV1::directory(
            &identity,
            DirectoryInteriorRecheckV1 {
                durable_identity: &durable_identity,
                expected: DirectoryInteriorExpectationV1::AdmissionStaging(expected),
            },
        ),
        DestinationRecheckV1::AdmissionCatalogInterior {
            durable_identity: final_identity,
            expected: bootstrap,
            absent: RootEntryNameV1::ActiveAction(expected.action_digest()),
        },
        "publish admission action directory",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::AdmissionFinalPublish);
    let republished = final_directory
        .open_dir_nofollow(OsStr::new(published.as_str()))
        .map_err(|source| CheckedFsError::io("reopen published action directory", source))?;
    if encode_identity(&super::HostPlatform.dir_identity(&republished)?) != identity {
        return Err(CheckedFsError::ambiguous(
            "published action directory",
            "opened directory identity does not match the published staging object",
        ));
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::AdmissionFinalReobserve);
    sync_directory_edge(final_directory, "flush admission action publication")
}

fn final_action_name(expected: &ActionCapacityReservationV1) -> String {
    RootEntryNameV1::ActiveAction(expected.action_digest()).name()
}

/// R2-E E3.1 — `terminal.*` key #6: the retirement destination row is proved
/// free and the frozen occupancy credit is proved available.
///
/// **DECISION T-C′** (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §8): keys
/// #6-#10 are edges of the **catalog root** and its retired root, which is the
/// capability this file owns. The retired root arrives through the one new
/// owner-private forward E3.1 mints — `RetainedCompletedCatalogV1::retired_root`
/// — and it is a handle *inside* the sealed provider owner, never a capability
/// a consumer receives.
///
/// **DECISION T-A**: the destination name is the already-derived
/// `RootEntryNameV1::ActiveAction(digest).name()` under a different parent.
/// `RootEntryNameV1::name` is parent-independent, and
/// `MAX_RETIRED_ACTION_DIRS == MAX_ACTIVE_ACTION_DIRS == 64` — the frozen
/// bounds already model a one-for-one retired twin of each active row, which
/// only a same-name-different-parent scheme gives. Nothing is minted.
fn reserve_retired_slot(
    retired_root: &Dir,
    destination: &OsStr,
    active_action_dirs: usize,
    retired_action_dirs: usize,
) -> Result<(), CheckedFsError> {
    match retired_root.symlink_metadata(destination) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(CheckedFsError::io("observe retirement destination", source)),
        Ok(_) => {
            return Err(CheckedFsError::ambiguous(
                "terminal retirement",
                "the derived retirement destination row is already occupied",
            ));
        }
    }
    // The retirement is about to move one active row into the retired root, so
    // the occupancy it must satisfy is the post-edge one. `validate` refuses
    // `RetiredLimitExceeded` and the retirement-credit inequality that reserves
    // one retired slot for every action still outstanding.
    CatalogOccupancyV1::new(
        active_action_dirs.saturating_sub(1),
        retired_action_dirs + 1,
        CatalogAdmissionOccupancyV1::Idle,
    )
    .map_err(|_| {
        CheckedFsError::ambiguous(
            "terminal retirement",
            "the retirement would leave the catalog outside its frozen occupancy bounds",
        )
    })?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::TerminalRetiredSlotReserve);
    Ok(())
}

/// R2-E E3.1 — `terminal.*` keys #6, #7 and #8: the whole admitted action
/// directory retires into the catalog's retired root.
///
/// Freeze §4.3 row E7, "P1 with destination recheck". The rename is the commit
/// point; the flush that follows orders the *observation*, not the atomicity —
/// the E16 cross-parent record (freeze §4.3) applies unchanged, this being the
/// second cross-directory durable edge in the provider's namespace family.
///
/// The source recheck is the `TerminalActionDirectory` arm and the destination
/// recheck is DECISION T-B′'s `TerminalRetiredRoot`, whose two observation
/// inputs — the retired root and the catalog root — are both re-proved inside
/// the acquisition window.
#[allow(
    clippy::too_many_arguments,
    reason = "the terminal retirement binds two retained parents, two identities, the frozen \
              bootstrap record, the reservation, and both observed occupancy terms; collapsing \
              them into a struct would hide which of the two roots each fact belongs to"
)]
pub(super) fn retire_action_directory(
    final_directory: &Dir,
    final_identity: &DurableObjectIdentityV1,
    retired_root: &Dir,
    bootstrap: &CatalogBootstrapRecordV1,
    expected: &ActionCapacityReservationV1,
    active_action_dirs: usize,
    retired_action_dirs: usize,
) -> Result<(), CheckedFsError> {
    let child = RootEntryNameV1::ActiveAction(expected.action_digest());
    let name = child.name();
    let name = OsStr::new(name.as_str());

    reserve_retired_slot(retired_root, name, active_action_dirs, retired_action_dirs)?;

    let action = final_directory
        .open_dir_nofollow(name)
        .map_err(|source| CheckedFsError::io("open retiring action directory", source))?;
    let fact = super::HostPlatform.dir_identity(&action)?;
    let identity = encode_identity(&fact);
    let durable_identity = fact.durable().clone();
    // Release this owner's own handle into the source tree before the rename
    // edge, for the reason `publish_staging_action` states: on Windows a
    // directory rename fails with a sharing violation while any handle into the
    // source tree survives, and the sealed primitive re-establishes source
    // identity and interior through its own capabilities.
    drop(action);
    publish_verified_no_replace(
        final_directory,
        name,
        retired_root,
        name,
        PublicationSourceV1::directory(
            &identity,
            DirectoryInteriorRecheckV1 {
                durable_identity: &durable_identity,
                expected: DirectoryInteriorExpectationV1::TerminalActionDirectory(expected),
            },
        ),
        DestinationRecheckV1::TerminalRetiredRoot {
            catalog_root: final_directory,
            catalog_identity: final_identity,
            expected: bootstrap,
            absent_child: child,
        },
        "retire action directory",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::TerminalActionDirectoryRetire,
    );

    let retired = retired_root
        .open_dir_nofollow(name)
        .map_err(|source| CheckedFsError::io("reopen retired action directory", source))?;
    if encode_identity(&super::HostPlatform.dir_identity(&retired)?) != identity {
        return Err(CheckedFsError::ambiguous(
            "retired action directory",
            "opened directory identity does not match the retired action object",
        ));
    }
    sync_directory_edge(retired_root, "flush terminal action retirement")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::TerminalRetiredDirectoryReobserve,
    );
    Ok(())
}

/// R2-E E3.1 — `terminal.*` keys #9 and #10: the catalog root's dirent barrier,
/// and the completed-catalog predicate re-proved **after** the retirement.
///
/// Key #9 is the `ExactInterior` class of the admitted dirent-barrier family
/// (freeze §4.1 row P5): the catalog root is the retirement's *source* parent,
/// so its barrier is what orders the namespace transition. Key #10 is this
/// function's tail, per E0.2 §4.3: `interior::completed_record` is `Some` under
/// T1's widened reading, **and** the retained-catalog revalidation passes —
/// which is the boundary that could not exist before the widening, since it is
/// by construction the one a populated retired root made unpassable.
pub(super) fn barrier_catalog_root(
    final_directory: &Dir,
    final_identity: &DurableObjectIdentityV1,
    bootstrap: &CatalogBootstrapRecordV1,
    revalidate_retained: impl FnOnce() -> Result<(), CheckedFsError>,
) -> Result<(), CheckedFsError> {
    crate::checked_artifact::platform::private_barrier(
        final_directory,
        crate::checked_artifact::platform::DirentBarrierClass::ExactInterior,
        crate::model::ErrorCode::IoError,
        "terminal catalog barrier",
    )
    .map_err(|source| CheckedFsError::ambiguous("terminal catalog barrier", source.message))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::TerminalCatalogBarrier);

    let fresh = interior::observe(final_directory, &super::HostPlatform)?;
    if interior::completed_record(final_identity, &fresh, bootstrap).is_none() {
        return Err(CheckedFsError::ambiguous(
            "terminal catalog revalidation",
            "the catalog is not exactly complete after the terminal retirement",
        ));
    }
    revalidate_retained()?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::TerminalTerminalRevalidate);
    Ok(())
}

/// Which durable admission row a shared edge is crossing.
///
/// Three durable rows share one record-write helper, so the row — never a name
/// minted at runtime — selects the stable `admission.*` boundaries that helper
/// announces (`fault_v1.rs:3-5`). The row also carries the diagnostic fact each
/// call site named before, so the shared helper keeps reporting exactly the
/// fact it reported when the fact was passed literally.
#[derive(Clone, Copy)]
enum AdmissionRecordRowV1 {
    Preparing,
    Idle,
    Reservation,
}

impl AdmissionRecordRowV1 {
    /// The durable state a §7 step 3 / step 7 scratch write installs: the
    /// frozen `idle()` record, or this action's `preparing` record. The two are
    /// already distinguished by the same equality the driver resolves on
    /// (`admission/driver.rs:157`), so no new predicate is introduced.
    fn of_admission_record(record: &ActionDirectoryAdmissionV1) -> Self {
        if *record == ActionDirectoryAdmissionV1::idle() {
            Self::Idle
        } else {
            Self::Preparing
        }
    }

    const fn fact(self) -> &'static str {
        match self {
            Self::Preparing | Self::Idle => "admission scratch",
            Self::Reservation => "resident reservation",
        }
    }

    /// The create / write / flush boundaries this row's durable write crosses.
    #[cfg(test)]
    const fn write_faults(self) -> [CheckedArtifactFaultKeyV1; 3] {
        match self {
            Self::Preparing => [
                CheckedArtifactFaultKeyV1::AdmissionPreparingScratchCreate,
                CheckedArtifactFaultKeyV1::AdmissionPreparingScratchWrite,
                CheckedArtifactFaultKeyV1::AdmissionPreparingScratchFlush,
            ],
            Self::Idle => [
                CheckedArtifactFaultKeyV1::AdmissionIdleScratchCreate,
                CheckedArtifactFaultKeyV1::AdmissionIdleScratchWrite,
                CheckedArtifactFaultKeyV1::AdmissionIdleScratchFlush,
            ],
            Self::Reservation => [
                CheckedArtifactFaultKeyV1::AdmissionReservationCreate,
                CheckedArtifactFaultKeyV1::AdmissionReservationWrite,
                CheckedArtifactFaultKeyV1::AdmissionReservationFlush,
            ],
        }
    }
}

/// The publish / reobserve boundaries of the shared install half of §7 steps 3
/// and 7. `ActionAdmissionEdgeV1::PublishAdmissionRecord` carries no
/// discriminator and the seam is frozen, so the row is read from the scratch
/// bytes the edge is about to publish — the same durable fact the driver
/// resolved the edge from.
#[cfg(test)]
fn install_faults(bytes: &[u8]) -> [CheckedArtifactFaultKeyV1; 2] {
    match decode_action_directory_admission(std::io::Cursor::new(bytes)) {
        Ok(record) if record == ActionDirectoryAdmissionV1::idle() => [
            CheckedArtifactFaultKeyV1::AdmissionIdlePublish,
            CheckedArtifactFaultKeyV1::AdmissionIdleReobserve,
        ],
        _ => [
            CheckedArtifactFaultKeyV1::AdmissionPreparingPublish,
            CheckedArtifactFaultKeyV1::AdmissionPreparingReobserve,
        ],
    }
}

/// Write-through create-or-rewrite plus handle flush plus parent flush — the
/// already-admitted P2 family (interface freeze §4.1 row P2), reusing the
/// catalog owner's own durable open options so the Windows write-through arm
/// is shared rather than restated.
fn write_durable_record(
    parent: &Dir,
    name: &OsStr,
    bytes: &[u8],
    row: AdmissionRecordRowV1,
) -> Result<(), CheckedFsError> {
    let fact = row.fact();
    #[cfg(test)]
    let faults = row.write_faults();
    let mut options = durable_write_options(false);
    options.create(true);
    let mut file = parent
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open admission record", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(faults[0]);
    file.set_len(0)
        .map_err(|source| CheckedFsError::io("truncate admission record", source))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind admission record", source))?;
    file.write_all(bytes)
        .map_err(|source| CheckedFsError::io("write admission record", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(faults[1]);
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush admission record", source))?;
    let identity = encode_identity(&super::HostPlatform.file_identity(&file)?);
    let written = ObservedFileV1 {
        identity: &identity,
        bytes,
    };
    verify_open_file(&mut file, written, fact)?;
    drop(file);
    verify_named_file(parent, name, written, fact)?;
    sync_directory_edge(parent, "flush admission record write")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(faults[2]);
    Ok(())
}

fn observed_record(
    parent: &Dir,
    name: &OsStr,
    fact: &'static str,
) -> Result<(Vec<u8>, Vec<u8>), CheckedFsError> {
    let metadata = parent
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe admission record", source))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Err(CheckedFsError::ambiguous(
            fact,
            "admission record is not a canonical regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = parent
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open admission record", source))?;
    let identity = encode_identity(&super::HostPlatform.file_identity(&file)?);
    let limit = ProtocolRecordKindV1::Admission.max_bytes();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "admission record read allocation failed",
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind admission record", source))?;
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read admission record", source))?;
    if bytes.len() > limit {
        return Err(CheckedFsError::ambiguous(
            fact,
            "admission record exceeds its frozen record bound",
        ));
    }
    Ok((identity, bytes))
}
