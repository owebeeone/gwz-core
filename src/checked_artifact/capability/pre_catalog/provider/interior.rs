//! Bounded observation and exact-prefix classification for first-catalog interiors.

use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;

use super::filesystem::PlatformProviderV1;
use super::retained::encode_identity;
use super::{
    RawCatalogBytesV1, RawCatalogInteriorFactV1, RawCatalogInteriorObservationV1,
    RawCatalogInteriorRowV1,
};
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::catalog::{
    CatalogDirectoryFactV1, CatalogNameBudgetV1, native_name_matches_ascii,
};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionSlotV1, BaseActionSlotV1, CatalogBootstrapRecordV1,
    CatalogRootRowCensusV1, CatalogRootRowClassV1, InfrastructureRecordV1, InfrastructureSlotV1,
    MAX_ACTION_SLOTS, MAX_ACTIVE_ACTION_DIRS, MAX_INFRASTRUCTURE_ENTRIES, MAX_RETIRED_ACTION_DIRS,
    MAX_ROOT_ENTRIES, ObservedActionDirectoryV1, OwnershipMarkerV1, ProtocolRecordKindV1,
    RecordObservationV1, ScratchBytesV1, classify_expected_prefix, decode_catalog_bootstrap_record,
    managed_marker_name,
};

const ROAMING_ANCHOR_BYTES: &[u8] = b"GWZ-ROAMING-ANCHOR-V1\n";
const CATALOG_ANCHOR_BYTES: &[u8] = b"GWZ-CATALOG-ANCHOR-V1\n";
/// The catalog root's bound, widened from the ten infrastructure slots to the
/// already-frozen `MAX_ROOT_ENTRIES` (= 74 = 10 infrastructure + 64 active
/// action directories, `protocol/bounds.rs:21-23`) so a published
/// `RootEntryNameV1::ActiveAction` row survives reobservation.
///
/// `GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 2 (C-3) fact 3: "Ten is exactly
/// `|InfrastructureSlotV1::ALL|`, so a fully-populated catalog root has zero
/// headroom: the cap and the grammar have to move together." Both move here,
/// and both move only onto vocabulary R1+C0 already froze. Widening only: the
/// per-family caps below keep every previously-accepted interior accepted.
const MAX_INTERIOR_ENTRIES: usize = MAX_ROOT_ENTRIES;

pub(super) enum StagingPlanV1 {
    CreateRetiredActions,
    WriteRoamingAnchor {
        create_new: bool,
    },
    WriteCatalogAnchorB {
        create_new: bool,
    },
    ExerciseAnchorAndWriteDescriptor(InfrastructureRecordV1),
    WriteDescriptor {
        record: InfrastructureRecordV1,
        create_new: bool,
    },
    WriteFormat {
        record: InfrastructureRecordV1,
        create_new: bool,
    },
    Complete(InfrastructureRecordV1),
    Other,
}

pub(super) fn observe(
    directory: &cap_std::fs::Dir,
    platform: &impl PlatformProviderV1,
) -> Result<RawCatalogInteriorObservationV1, CheckedFsError> {
    let mode = platform.parent_mode(directory)?;
    let mut budget = CatalogNameBudgetV1::new();
    let mut rows = Vec::new();
    let mut action_rows = Vec::new();
    let mut census = CatalogRootRowCensusV1::default();
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate catalog interior", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read catalog interior", source))?;
        let name = entry.file_name();
        budget.charge_os_str(&name)?;
        if rows.len() + action_rows.len() == MAX_INTERIOR_ENTRIES {
            return Err(interior_bound_exceeded());
        }
        let class = exact_row(&name, mode)?;
        census.charge(class);
        if let CatalogRootRowClassV1::ActiveAction(action) = class {
            if action_rows.len() == MAX_ACTIVE_ACTION_DIRS {
                return Err(interior_bound_exceeded());
            }
            reserve_one(&mut action_rows)?;
            action_rows.push(action);
            continue;
        }
        // `exact_row` above refuses every `MalformedRecognized` and `Foreign`
        // child before it can be classified onto this path, so the only classes
        // that reach here are the slot-bearing ones and this cannot panic. Any
        // future widening of that refusal must keep the guarantee or convert
        // this into a typed refusal — the same invariant the driver's
        // `census.has_unowned_row()` stop is deliberately kept for
        // (`admission/driver.rs`).
        let slot = class
            .infrastructure_slot()
            .expect("a classified non-action catalog row owns an infrastructure slot");
        if rows.len() == MAX_INFRASTRUCTURE_ENTRIES {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "catalog interior exceeds the ten-slot bound",
            ));
        }
        let fact = observe_slot(
            directory,
            &name,
            slot == InfrastructureSlotV1::RetiredActions,
            platform,
        )?;
        reserve_one(&mut rows)?;
        rows.push(RawCatalogInteriorRowV1 { slot, fact });
    }
    rows.sort_unstable_by_key(|row| slot_index(row.slot));
    if rows.windows(2).any(|pair| pair[0].slot == pair[1].slot) {
        return Err(CheckedFsError::ambiguous(
            "catalog interior",
            "multiple native entries resolve to one infrastructure slot",
        ));
    }
    action_rows.sort_unstable_by_key(|action| action.bytes());
    if action_rows.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(CheckedFsError::ambiguous(
            "catalog interior",
            "multiple native entries resolve to one action row",
        ));
    }
    Ok(RawCatalogInteriorObservationV1 {
        entry_count: budget.entry_count(),
        encoded_name_bytes: budget.encoded_name_bytes(),
        rows,
        action_rows,
        census,
    })
}

fn interior_bound_exceeded() -> CheckedFsError {
    CheckedFsError::unsupported(
        PlatformCapability::PrivateNamespaceCollisionScan,
        "catalog interior exceeds the frozen root-entry bound",
    )
}

fn reserve_one<T>(values: &mut Vec<T>) -> Result<(), CheckedFsError> {
    values.try_reserve_exact(1).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog interior row allocation failed",
        )
    })
}

pub(super) fn directory_fact(
    role: CatalogPrivateNameV1,
    directory_identity: &crate::checked_artifact::capability::DurableObjectIdentityV1,
    interior: &RawCatalogInteriorObservationV1,
    expected: Option<&CatalogBootstrapRecordV1>,
) -> CatalogDirectoryFactV1 {
    let Some(expected) = expected else {
        return CatalogDirectoryFactV1::Other;
    };
    match role {
        CatalogPrivateNameV1::BootstrapStaging => {
            match staging_plan(directory_identity, interior, expected) {
                StagingPlanV1::Complete(_) => CatalogDirectoryFactV1::ExactOwned,
                StagingPlanV1::Other => CatalogDirectoryFactV1::Other,
                _ => CatalogDirectoryFactV1::ActiveOwnedPrefix,
            }
        }
        CatalogPrivateNameV1::Final => {
            if completed_record(directory_identity, interior, expected).is_some() {
                CatalogDirectoryFactV1::ExactOwned
            } else {
                CatalogDirectoryFactV1::Other
            }
        }
        _ => CatalogDirectoryFactV1::Other,
    }
}

pub(super) fn staging_plan(
    directory_identity: &crate::checked_artifact::capability::DurableObjectIdentityV1,
    interior: &RawCatalogInteriorObservationV1,
    expected: &CatalogBootstrapRecordV1,
) -> StagingPlanV1 {
    use InfrastructureSlotV1 as Slot;
    // The C-3 widening (interface freeze §4.4 Class 2 fact 2) is deliberately
    // **not** applied here. Admission runs only against a complete catalog's
    // root, never into a bootstrap-staging interior, so no cooperating history
    // ever places an `ActionAdmission*` slot inside a staging directory: the
    // recorded breakage chain (`recover_or_create` -> `execute_owner_complete`
    // -> `retain_completed_catalog`) runs through `completed_record`, which is
    // where the drop is owed and taken. Dropping the triad here instead widened
    // the bootstrap's *adoption* grammar with no flow that needs it — a staging
    // directory planted with a stray-but-valid admission record alongside the
    // six exact roles would classify `Complete`, pass the CatalogStaging
    // source-interior recheck, and publish as a live catalog carrying an
    // unexplained admission row. Beyond the amendment §4.1 trust boundary the
    // R1 posture is to fail closed, so the refusal stays.
    // `CatalogBootstrapRetired` keeps its own refusal: it is the bootstrap
    // owner's pre-retirement discriminator.
    if any_present(
        interior,
        &[
            Slot::CatalogBootstrapRetired,
            Slot::ActionAdmissionActive,
            Slot::ActionAdmissionScratch,
            Slot::ActionAdmissionStaging,
        ],
    ) {
        return StagingPlanV1::Other;
    }
    let retired = match row(interior, Slot::RetiredActions) {
        None => {
            return if only_missing(
                interior,
                &[
                    Slot::RoamingAnchorHome,
                    Slot::CatalogAnchorA,
                    Slot::CatalogAnchorB,
                    Slot::RetiredActionsDescriptor,
                    Slot::CatalogFormat,
                ],
            ) {
                StagingPlanV1::CreateRetiredActions
            } else {
                StagingPlanV1::Other
            };
        }
        Some(RawCatalogInteriorFactV1::EmptyDirectory {
            durable_identity, ..
        }) => durable_identity,
        // T1's widening is deliberately **not** applied here, and the arm is
        // preserved by name rather than left to the catch-all below
        // (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §2.2, "A fourth
        // surface E3.1 must preserve deliberately, not widen"). A catalog
        // being *built* must not find action rows already retired into it: the
        // three widened gates all read a catalog that is already complete,
        // whereas this plan decides whether an incomplete staging interior may
        // be adopted, and adopting one carrying retired action rows would
        // publish a live catalog with an unexplained retirement history. Same
        // posture as the `ActionAdmission*` triad's refusal above.
        Some(RawCatalogInteriorFactV1::RetiredActionRoot { .. }) => {
            return StagingPlanV1::Other;
        }
        _ => return StagingPlanV1::Other,
    };
    let roaming = match file_prefix(interior, Slot::RoamingAnchorHome, ROAMING_ANCHOR_BYTES) {
        FilePrefixV1::Missing => {
            return if later_after_roaming_missing(interior) {
                StagingPlanV1::WriteRoamingAnchor { create_new: true }
            } else {
                StagingPlanV1::Other
            };
        }
        FilePrefixV1::Partial => {
            return if later_after_roaming_missing(interior) {
                StagingPlanV1::WriteRoamingAnchor { create_new: false }
            } else {
                StagingPlanV1::Other
            };
        }
        FilePrefixV1::Exact(identity) => identity,
        FilePrefixV1::Other => return StagingPlanV1::Other,
    };

    let (anchor, needs_exercise) = match (
        file_prefix(interior, Slot::CatalogAnchorA, CATALOG_ANCHOR_BYTES),
        file_prefix(interior, Slot::CatalogAnchorB, CATALOG_ANCHOR_BYTES),
    ) {
        (FilePrefixV1::Missing, FilePrefixV1::Missing)
            if descriptor_and_format_missing(interior) =>
        {
            return StagingPlanV1::WriteCatalogAnchorB { create_new: true };
        }
        (FilePrefixV1::Missing, FilePrefixV1::Partial)
            if descriptor_and_format_missing(interior) =>
        {
            return StagingPlanV1::WriteCatalogAnchorB { create_new: false };
        }
        (FilePrefixV1::Missing, FilePrefixV1::Exact(identity))
            if descriptor_and_format_missing(interior) =>
        {
            (identity, true)
        }
        (FilePrefixV1::Exact(identity), FilePrefixV1::Missing) => (identity, true),
        _ => return StagingPlanV1::Other,
    };
    let infrastructure = match InfrastructureRecordV1::owner_issue_for_catalog(
        expected,
        directory_identity.clone(),
        anchor.clone(),
        roaming.clone(),
        retired.clone(),
    ) {
        Ok(value) => value,
        Err(_) => return StagingPlanV1::Other,
    };
    let descriptor = file_prefix(
        interior,
        Slot::RetiredActionsDescriptor,
        &infrastructure.encode_canonical(),
    );
    if needs_exercise
        && matches!(descriptor, FilePrefixV1::Missing)
        && row(interior, Slot::CatalogFormat).is_none()
    {
        return StagingPlanV1::ExerciseAnchorAndWriteDescriptor(infrastructure);
    }
    match descriptor {
        FilePrefixV1::Missing if row(interior, Slot::CatalogFormat).is_none() => {
            StagingPlanV1::WriteDescriptor {
                record: infrastructure,
                create_new: true,
            }
        }
        FilePrefixV1::Partial if row(interior, Slot::CatalogFormat).is_none() => {
            StagingPlanV1::WriteDescriptor {
                record: infrastructure,
                create_new: false,
            }
        }
        FilePrefixV1::Exact(_) => match file_prefix(
            interior,
            Slot::CatalogFormat,
            &infrastructure.encode_canonical(),
        ) {
            FilePrefixV1::Missing => StagingPlanV1::WriteFormat {
                record: infrastructure,
                create_new: true,
            },
            FilePrefixV1::Partial => StagingPlanV1::WriteFormat {
                record: infrastructure,
                create_new: false,
            },
            FilePrefixV1::Exact(_) => StagingPlanV1::Complete(infrastructure),
            FilePrefixV1::Other => StagingPlanV1::Other,
        },
        _ => StagingPlanV1::Other,
    }
}

pub(super) fn completed_record(
    directory_identity: &crate::checked_artifact::capability::DurableObjectIdentityV1,
    interior: &RawCatalogInteriorObservationV1,
    expected: &CatalogBootstrapRecordV1,
) -> Option<InfrastructureRecordV1> {
    use InfrastructureSlotV1 as Slot;
    // C-3 widening (interface freeze §4.4 Class 2 facts 2 and the "Consequence"
    // paragraph): with an `ActionAdmissionActive` slot resident — the steady
    // state after a successful admission — this predicate used to be `None`,
    // which broke `retain_completed_catalog` (`completed.rs:61`) and therefore
    // ConsumerCheckpoint §7 step 8's reobservation. The admission triad is now
    // admitted; `CatalogAnchorB` keeps its refusal because an unexercised
    // B anchor still means the catalog is mid-bootstrap.
    if any_present(interior, &[Slot::CatalogAnchorB]) {
        return None;
    }
    // T1 widening gate 1 of 3 (E0.2b §2.2). This was
    // `empty_directory_identity(interior, Slot::RetiredActions)?`, which
    // returned `Some` only for an *empty* retired root — so the catalog became
    // unobservable, and therefore unrecoverable, at its own first terminal
    // retirement. `retired_root_identity` accepts the same empty root plus a
    // populated one whose every child is a `RootEntryNameV1::ActiveAction` row
    // and whose count is within `MAX_RETIRED_ACTION_DIRS`. The identity it
    // returns is the retired root's own durable identity in both arms, which a
    // child addition does not change, so `RetiredActionsDescriptor` and
    // `CatalogFormat` stay byte-identical across the widening.
    let retired = retired_root_identity(interior)?;
    let roaming = exact_file_identity(interior, Slot::RoamingAnchorHome, ROAMING_ANCHOR_BYTES)?;
    let anchor = exact_file_identity(interior, Slot::CatalogAnchorA, CATALOG_ANCHOR_BYTES)?;
    let infrastructure = InfrastructureRecordV1::owner_issue_for_catalog(
        expected,
        directory_identity.clone(),
        anchor,
        roaming,
        retired,
    )
    .ok()?;
    let bytes = infrastructure.encode_canonical();
    exact_file_identity(interior, Slot::RetiredActionsDescriptor, &bytes)?;
    exact_file_identity(interior, Slot::CatalogFormat, &bytes)?;
    Some(infrastructure)
}

pub(super) fn retired_record(
    interior: &RawCatalogInteriorObservationV1,
) -> crate::checked_artifact::catalog::CatalogRecordFactV1 {
    use crate::checked_artifact::catalog::CatalogRecordFactV1;
    let Some(fact) = row(interior, InfrastructureSlotV1::CatalogBootstrapRetired) else {
        return CatalogRecordFactV1::Missing;
    };
    let RawCatalogInteriorFactV1::RegularFile {
        bytes: RawCatalogBytesV1::Bounded(bytes),
        ..
    } = fact
    else {
        return CatalogRecordFactV1::Other;
    };
    decode_catalog_bootstrap_record(std::io::Cursor::new(bytes))
        .map(CatalogRecordFactV1::exact)
        .unwrap_or(CatalogRecordFactV1::Other)
}

/// Classifies one catalog-root child into the §6 (:199-201) grammar.
///
/// C-3 widening (`GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 2 fact 1): the
/// observer previously walked `InfrastructureSlotV1::ALL` alone and refused
/// every other child, so it "does not yet admit the very row E3 publishes".
/// It now walks the whole frozen `RootEntryNameV1` grammar, whose second arm is
/// the active-action row. Widening only: the platform-alias and unowned-child
/// refusals are byte-identical, so every interior that classified before still
/// classifies the same way, and only `action-<hex>-v1` rows are newly admitted.
fn exact_row(
    name: &OsStr,
    mode: crate::checked_artifact::capability::PathComponentMode,
) -> Result<CatalogRootRowClassV1, CheckedFsError> {
    for slot in InfrastructureSlotV1::ALL.iter().copied() {
        if native_name_matches_ascii(name, slot.name().as_bytes(), mode)? {
            return if name == OsStr::new(slot.name()) {
                Ok(CatalogRootRowClassV1::classify(slot.name().as_bytes()))
            } else {
                Err(CheckedFsError::ambiguous(
                    "catalog interior",
                    "platform-equivalent infrastructure alias is noncanonical",
                ))
            };
        }
    }
    let class = CatalogRootRowClassV1::classify(native_ascii_bytes(name).unwrap_or(&[]));
    if matches!(class, CatalogRootRowClassV1::ActiveAction(_)) {
        return Ok(class);
    }
    Err(CheckedFsError::ambiguous(
        "catalog interior",
        "catalog directory contains an unowned child",
    ))
}

/// The canonical ASCII spelling of a native name, or `None` when the platform
/// name is not ASCII. An action row's grammar is ASCII-only
/// (`protocol/slots.rs:385-400`), so a non-ASCII child can never be one and
/// falls through to the unchanged unowned-child refusal.
fn native_ascii_bytes(name: &OsStr) -> Option<&[u8]> {
    let bytes = name.to_str()?.as_bytes();
    bytes.is_ascii().then_some(bytes)
}

fn observe_slot(
    directory: &cap_std::fs::Dir,
    name: &OsStr,
    probe_empty_directory: bool,
    platform: &impl PlatformProviderV1,
) -> Result<RawCatalogInteriorFactV1, CheckedFsError> {
    let metadata = directory
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe catalog interior slot", source))?;
    if metadata.is_dir() && !metadata.is_symlink() {
        // Share-delete open, not a plain no-follow one. This enumeration runs
        // inside the sealed publication's destination recheck, which holds the
        // retained rename-source handle open across the whole edge — and R2-D
        // Phase 1's `PublishStagingAction` is the first publication whose
        // source is a *directory child of the very root being enumerated here*
        // (`ActionAdmissionStaging`). On Windows that handle carries DELETE
        // access, so any later open of the same object that does not itself
        // grant DELETE sharing fails with a sharing violation (os error 32);
        // cap-std's plain directory open omits `FILE_SHARE_DELETE`, so it is
        // exactly such an open. `platform::open_dir_share_delete` is the
        // established recipe for this collision (`platform.rs`, the
        // `FILE_SHARE_DELETE` arm; freeze §4.1 P3 records it as "so the
        // directory open does not collide with the retained rename-source
        // handle"). Dropping the source handle instead is not available: the
        // primitive renames that exact identity-checked handle, so its lifetime
        // is the seam's guarantee.
        //
        // Non-Windows arm is byte-identical to the previous call — the helper
        // is `open_dir_nofollow` there — so macOS and Linux behaviour is
        // unchanged. The sibling regular-file open below needs no counterpart:
        // it inherits std's default share mode, which already includes
        // `FILE_SHARE_DELETE`, which is why only the directory label appeared
        // in the Windows failures.
        let child = crate::checked_artifact::platform::open_dir_share_delete(directory, name)
            .map_err(|source| CheckedFsError::io("open catalog interior directory", source))?;
        let identity = platform.dir_identity(&child)?;
        if probe_empty_directory {
            // T1 widening (E0.2b §2, AUTHORIZED; the freeze's own Class 2
            // shape, `:1443-1450` — "what Phase 1 must extend is the
            // *provider's reading* of that vocabulary, not the vocabulary").
            // The retired root is read by its own dedicated single-level
            // reader, which is where the empty case is decided too.
            let retired = read_retired_root(&child)?;
            if retired.is_empty() {
                return Ok(RawCatalogInteriorFactV1::EmptyDirectory {
                    identity: encode_identity(&identity),
                    durable_identity: identity.durable().clone(),
                });
            }
            return Ok(RawCatalogInteriorFactV1::RetiredActionRoot {
                identity: encode_identity(&identity),
                durable_identity: identity.durable().clone(),
                unaccepted_rows: retired.unaccepted_rows,
                retired_action_dirs: retired.retired_action_dirs,
            });
        }
    } else if metadata.is_file() && !metadata.is_symlink() {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = directory
            .open_with(name, &options)
            .map_err(|source| CheckedFsError::io("open catalog interior file", source))?;
        let identity = platform.file_identity(&file)?;
        return Ok(RawCatalogInteriorFactV1::RegularFile {
            identity: encode_identity(&identity),
            durable_identity: identity.durable().clone(),
            bytes: read_bounded(&mut file)?,
        });
    }
    let mut value = Vec::new();
    value.push(if metadata.is_symlink() { 3 } else { 4 });
    value.extend_from_slice(&metadata.dev().to_be_bytes());
    value.extend_from_slice(&metadata.ino().to_be_bytes());
    Ok(RawCatalogInteriorFactV1::Other(value))
}

/// The T1 widening's bounded reading of the `RetiredActions` root, one level
/// deep.
struct RetiredRootReadingV1 {
    /// Children that classify `RootEntryNameV1::ActiveAction`.
    retired_action_dirs: usize,
    /// Children that do **not**. Infrastructure-slot names, scheduled-scratch
    /// and retired names, malformed-recognized names, non-ASCII names and
    /// foreign names all land here: the reading accepts *only* action rows, so
    /// one counter of everything else is all the predicate needs, and it is
    /// what makes an infrastructure-slot name planted in the retired root a
    /// refusal rather than a classified row.
    unaccepted_rows: usize,
}

impl RetiredRootReadingV1 {
    const fn is_empty(&self) -> bool {
        self.retired_action_dirs == 0 && self.unaccepted_rows == 0
    }
}

/// Reads the `RetiredActions` root's own children **exactly once, exactly one
/// level deep**, and classifies each name through the frozen
/// [`RootEntryNameV1`] grammar.
///
/// **It deliberately calls neither [`observe`] nor [`observe_slot`], and that
/// is a structural property, not a check.** The first shape of this widening
/// re-entered `observe` on the retired root; `exact_row` is parent-independent,
/// so a `retired-actions-v1` child of the retired root classified as a
/// perfectly good infrastructure row and the pair became mutually recursive
/// with no depth counter. A nested chain then aborted the process on a stack
/// overflow — `SIGABRT`, reproduced at depth 700 — instead of returning the
/// typed refusal this owner's whole discipline promises, and it did so on the
/// path of *every* catalog consumer, since `completed_record` runs in every
/// recovery and every publication acquisition window. There is no self-call
/// here to exceed, so a nested chain of **any** depth is one directory read and
/// a refusal.
///
/// **The bound is checked explicitly and is not inherited.** The entry cap
/// below is `MAX_RETIRED_ACTION_DIRS` itself (`protocol/bounds.rs:2`), not
/// `interior::observe`'s own effective caps — `MAX_INTERIOR_ENTRIES`
/// (= `MAX_ROOT_ENTRIES` = 74) and `MAX_ACTIVE_ACTION_DIRS` (= 64) — and not
/// the name budget's `MAX_CATALOG_PARENT_ENTRIES_V1`. The reused reader was
/// numerically safe only because `bounds.rs:1` and `:2` are both 64, which
/// silently coupled the retired-root bound to the active one; naming the
/// retired constant here is what makes a future edit to either fail closed
/// (E0.2b §3.2 ground 3, Code round-2 [P3-R1]).
fn read_retired_root(directory: &cap_std::fs::Dir) -> Result<RetiredRootReadingV1, CheckedFsError> {
    let mut budget = CatalogNameBudgetV1::new();
    let mut actions: Vec<crate::checked_artifact::protocol::ActionDigestV1> = Vec::new();
    let mut unaccepted_rows = 0_usize;
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate retired-action root", source))?
    {
        let entry =
            entry.map_err(|source| CheckedFsError::io("read retired-action root", source))?;
        let name = entry.file_name();
        budget.charge_os_str(&name)?;
        if budget.entry_count() > MAX_RETIRED_ACTION_DIRS {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "retired-action root exceeds the frozen retired-action bound",
            ));
        }
        match CatalogRootRowClassV1::classify(native_ascii_bytes(&name).unwrap_or(&[])) {
            CatalogRootRowClassV1::ActiveAction(action) => {
                reserve_one(&mut actions)?;
                actions.push(action);
            }
            _ => unaccepted_rows += 1,
        }
    }
    actions.sort_unstable_by_key(|action| action.bytes());
    if actions.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(CheckedFsError::ambiguous(
            "retired-action root",
            "multiple native entries resolve to one retired action row",
        ));
    }
    Ok(RetiredRootReadingV1 {
        retired_action_dirs: actions.len(),
        unaccepted_rows,
    })
}

/// Bounded observation of one action directory through the frozen
/// [`ActionSlotV1`] grammar.
///
/// It lives in this file because `GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 1
/// records that the verification a recheck arm drives "lives in a different
/// file of the same owner" — `publication.rs` decides, `interior.rs` verifies.
/// It returns the frozen R1 observation type, so neither the admission driver
/// nor the sealed primitive receives a handle or a raw row.
pub(super) fn observe_action_directory(
    parent: &cap_std::fs::Dir,
    name: &OsStr,
    expected: &ActionCapacityReservationV1,
    platform: &impl PlatformProviderV1,
) -> Result<ObservedActionDirectoryV1, CheckedFsError> {
    let metadata = match parent.symlink_metadata(name) {
        Ok(value) => value,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ObservedActionDirectoryV1::Missing);
        }
        Err(source) => return Err(CheckedFsError::io("observe action directory", source)),
    };
    if !metadata.is_dir() || metadata.is_symlink() {
        return Ok(ObservedActionDirectoryV1::Other);
    }
    let directory = parent
        .open_dir_nofollow(name)
        .map_err(|source| CheckedFsError::io("open action directory", source))?;
    let identity = platform.dir_identity(&directory)?;
    let observed = observe_action_interior(&directory, expected)?;
    Ok(ObservedActionDirectoryV1::exact(
        identity.durable().clone(),
        observed.reservation,
        observed.extra_children,
    ))
}

/// Bounded interior of an already-open action directory, so the sealed
/// publication primitive can re-verify its retained source handle without
/// reopening the name it is about to consume.
pub(super) struct ActionInteriorObservationV1 {
    pub(super) reservation: RecordObservationV1<ActionCapacityReservationV1>,
    pub(super) extra_children: usize,
}

impl ActionInteriorObservationV1 {
    /// The §7 (:220-221) exactness predicate: the deterministic resident
    /// reservation and no extra children.
    pub(super) fn is_exact(&self, expected: &ActionCapacityReservationV1) -> bool {
        self.extra_children == 0
            && matches!(&self.reservation, RecordObservationV1::Exact(value) if value == expected)
    }

    /// R2-E E3.1's terminal source-interior predicate: the resident reservation
    /// is still this exact one.
    ///
    /// It is [`Self::is_exact`] without the `extra_children == 0` clause,
    /// which holds of a freshly staged action directory and never of one that
    /// has run an action — the directory a terminal retirement moves carries
    /// its authority, payload, worklist and retired-alias rows by construction.
    /// The bounded enumeration and the frozen `MAX_ACTION_SLOTS` refusal are
    /// the same for both; only this clause differs. "`publication.rs` decides,
    /// `interior.rs` verifies" (freeze §4.4 Class 1).
    pub(super) fn is_reservation_exact(&self, expected: &ActionCapacityReservationV1) -> bool {
        matches!(&self.reservation, RecordObservationV1::Exact(value) if value == expected)
    }
}

pub(super) fn observe_action_interior(
    directory: &cap_std::fs::Dir,
    expected: &ActionCapacityReservationV1,
) -> Result<ActionInteriorObservationV1, CheckedFsError> {
    let reservation_name =
        ActionSlotV1::Base(BaseActionSlotV1::Reservation).name(expected.action_digest());
    let reservation_name = OsStr::new(reservation_name.as_str());
    let expected_bytes = expected.encode_canonical().map_err(|_| {
        CheckedFsError::ambiguous(
            "action capacity reservation",
            "expected capacity record is not canonically encodable",
        )
    })?;
    let mut budget = CatalogNameBudgetV1::new();
    let mut reservation = RecordObservationV1::Missing;
    let mut extra_children = 0_usize;
    let mut seen = 0_usize;
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate action directory", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read action directory", source))?;
        let child = entry.file_name();
        budget.charge_os_str(&child)?;
        seen += 1;
        if seen > MAX_ACTION_SLOTS {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "action directory exceeds the frozen action-slot bound",
            ));
        }
        if child == reservation_name {
            reservation = observe_reservation(directory, &child, &expected_bytes, expected)?;
            continue;
        }
        extra_children += 1;
    }
    Ok(ActionInteriorObservationV1 {
        reservation,
        extra_children,
    })
}

/// The scan bound for a staged or installed managed component interior.
///
/// A staged component holds exactly the ownership marker; an installed one
/// holds the marker until it retires and nothing after. Two is therefore one
/// row of headroom over every shape this owner accepts, and it is a *refusal
/// threshold* for the bounded enumeration (§4.1 family P4), not durable
/// vocabulary: no record, slot, purpose or phase is minted by it.
const MAX_MANAGED_COMPONENT_ENTRIES: usize = 2;

/// Bounded interior of an already-open staged managed component directory, so
/// the sealed publication primitive can re-verify its retained source handle
/// inside the acquisition window without reopening the name it is about to
/// consume.
///
/// This is the verification half of `GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 1's
/// **managed source-interior** arm (edge E15) — the row whose definition is "a
/// staged managed component's interior is neither record type". It lives here
/// rather than in `publication.rs` for the reason §4.4 records: "`publication.rs`
/// decides, `interior.rs` verifies".
pub(super) struct ManagedComponentInteriorObservationV1 {
    marker: RecordObservationV1<()>,
    extra_children: usize,
}

impl ManagedComponentInteriorObservationV1 {
    /// The exactness predicate: the deterministic resident ownership marker,
    /// byte-exact against the marker the intent issued, and no extra children.
    pub(super) const fn is_exact(&self) -> bool {
        self.extra_children == 0 && matches!(self.marker, RecordObservationV1::Exact(()))
    }
}

pub(super) fn observe_managed_component_interior(
    directory: &cap_std::fs::Dir,
    expected: &OwnershipMarkerV1,
) -> Result<ManagedComponentInteriorObservationV1, CheckedFsError> {
    let marker_name = managed_marker_name();
    let marker_name = OsStr::new(
        std::str::from_utf8(marker_name.as_bytes())
            .expect("the frozen managed marker name is ASCII"),
    );
    let expected_bytes = expected.encode_canonical();
    let mut budget = CatalogNameBudgetV1::new();
    let mut marker = RecordObservationV1::Missing;
    let mut extra_children = 0_usize;
    let mut seen = 0_usize;
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate managed component", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read managed component", source))?;
        let child = entry.file_name();
        budget.charge_os_str(&child)?;
        seen += 1;
        if seen > MAX_MANAGED_COMPONENT_ENTRIES {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "managed component exceeds its frozen interior bound",
            ));
        }
        if child == marker_name {
            marker = observe_managed_marker(directory, &child, &expected_bytes)?;
            continue;
        }
        extra_children += 1;
    }
    Ok(ManagedComponentInteriorObservationV1 {
        marker,
        extra_children,
    })
}

/// The resident marker, read bounded against the frozen `Marker` record bound
/// and compared byte-exact. The comparison is in-memory only: no handle, no row
/// and no path leaves this owner.
fn observe_managed_marker(
    directory: &cap_std::fs::Dir,
    name: &OsStr,
    expected_bytes: &[u8],
) -> Result<RecordObservationV1<()>, CheckedFsError> {
    let metadata = directory
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe ownership marker", source))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Ok(RecordObservationV1::Other);
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open ownership marker", source))?;
    let RawCatalogBytesV1::Bounded(bytes) =
        read_bounded_with(&mut file, ProtocolRecordKindV1::Marker.max_bytes())?
    else {
        return Ok(RecordObservationV1::Other);
    };
    Ok(match classify_expected_prefix(&bytes, expected_bytes) {
        ScratchBytesV1::Exact => RecordObservationV1::Exact(()),
        ScratchBytesV1::PartialExpectedPrefix => RecordObservationV1::PartialExpectedPrefix,
        ScratchBytesV1::Missing | ScratchBytesV1::Other => RecordObservationV1::Other,
    })
}

fn observe_reservation(
    directory: &cap_std::fs::Dir,
    name: &OsStr,
    expected_bytes: &[u8],
    expected: &ActionCapacityReservationV1,
) -> Result<RecordObservationV1<ActionCapacityReservationV1>, CheckedFsError> {
    let metadata = directory
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe resident reservation", source))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Ok(RecordObservationV1::Other);
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open resident reservation", source))?;
    let RawCatalogBytesV1::Bounded(bytes) =
        read_bounded_with(&mut file, ProtocolRecordKindV1::Capacity.max_bytes())?
    else {
        return Ok(RecordObservationV1::Other);
    };
    Ok(match classify_expected_prefix(&bytes, expected_bytes) {
        ScratchBytesV1::Exact => RecordObservationV1::Exact(expected.clone()),
        ScratchBytesV1::PartialExpectedPrefix => RecordObservationV1::PartialExpectedPrefix,
        ScratchBytesV1::Missing | ScratchBytesV1::Other => RecordObservationV1::Other,
    })
}

fn read_bounded(file: &mut cap_std::fs::File) -> Result<RawCatalogBytesV1, CheckedFsError> {
    read_bounded_with(
        file,
        ProtocolRecordKindV1::Infrastructure
            .max_bytes()
            .max(ProtocolRecordKindV1::CatalogBootstrap.max_bytes()),
    )
}

fn read_bounded_with(
    file: &mut cap_std::fs::File,
    limit: usize,
) -> Result<RawCatalogBytesV1, CheckedFsError> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog interior read allocation failed",
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind catalog interior file", source))?;
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read catalog interior file", source))?;
    Ok(if bytes.len() > limit {
        RawCatalogBytesV1::Oversize
    } else {
        RawCatalogBytesV1::Bounded(bytes)
    })
}

enum FilePrefixV1<'a> {
    Missing,
    Partial,
    Exact(&'a crate::checked_artifact::capability::DurableObjectIdentityV1),
    Other,
}

fn file_prefix<'a>(
    interior: &'a RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    expected: &[u8],
) -> FilePrefixV1<'a> {
    let Some(fact) = row(interior, slot) else {
        return FilePrefixV1::Missing;
    };
    let RawCatalogInteriorFactV1::RegularFile {
        durable_identity,
        bytes: RawCatalogBytesV1::Bounded(bytes),
        ..
    } = fact
    else {
        return FilePrefixV1::Other;
    };
    match classify_expected_prefix(bytes, expected) {
        ScratchBytesV1::PartialExpectedPrefix => FilePrefixV1::Partial,
        ScratchBytesV1::Exact => FilePrefixV1::Exact(durable_identity),
        ScratchBytesV1::Missing | ScratchBytesV1::Other => FilePrefixV1::Other,
    }
}

pub(super) fn row(
    interior: &RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
) -> Option<&RawCatalogInteriorFactV1> {
    interior
        .rows
        .iter()
        .find(|candidate| candidate.slot == slot)
        .map(|candidate| &candidate.fact)
}

fn exact_file_identity(
    interior: &RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    expected: &[u8],
) -> Option<crate::checked_artifact::capability::DurableObjectIdentityV1> {
    match file_prefix(interior, slot, expected) {
        FilePrefixV1::Exact(identity) => Some(identity.clone()),
        _ => None,
    }
}

/// The `RetiredActions` root's own durable identity, under T1's widened
/// reading (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §2, both axes
/// concurring; the freeze's Class 2 sanction `:1443-1450`).
///
/// Two arms, and only two: the empty root every catalog is created with
/// (`directory_mutation.rs`'s single `CreateRetiredActions` arm), and a
/// populated one whose children are **exclusively**
/// `RootEntryNameV1::ActiveAction` rows.
///
/// **This bound is checked explicitly and is not inherited.** The reused
/// reader's own caps are `MAX_INTERIOR_ENTRIES` (= `MAX_ROOT_ENTRIES` = 74)
/// and `MAX_ACTIVE_ACTION_DIRS` (= 64) — neither of them
/// `MAX_RETIRED_ACTION_DIRS`. They are numerically safe today only because
/// `bounds.rs:1` and `:2` are both 64, which silently couples the retired-root
/// bound to the active one; the explicit comparison below is what makes a
/// future edit to either constant fail closed here instead of decoupling them
/// unnoticed (E0.2b §3.2 ground 3, Code round-2 [P3-R1]).
fn retired_root_identity(
    interior: &RawCatalogInteriorObservationV1,
) -> Option<crate::checked_artifact::capability::DurableObjectIdentityV1> {
    match row(interior, InfrastructureSlotV1::RetiredActions)? {
        RawCatalogInteriorFactV1::EmptyDirectory {
            durable_identity, ..
        } => Some(durable_identity.clone()),
        RawCatalogInteriorFactV1::RetiredActionRoot {
            durable_identity,
            unaccepted_rows,
            retired_action_dirs,
            ..
        } if *unaccepted_rows == 0 && *retired_action_dirs <= MAX_RETIRED_ACTION_DIRS => {
            Some(durable_identity.clone())
        }
        _ => None,
    }
}

/// The bounded count of retired action directories resident under the catalog's
/// `RetiredActions` root, for the callers that charge the frozen retirement
/// credit against it (`protocol/bounds.rs` `CatalogOccupancyV1`). `None` means
/// the retired root is not readable as a retired root at all, which is the same
/// fact [`completed_record`] refuses on.
pub(super) fn retired_action_dirs(interior: &RawCatalogInteriorObservationV1) -> Option<usize> {
    match row(interior, InfrastructureSlotV1::RetiredActions)? {
        RawCatalogInteriorFactV1::EmptyDirectory { .. } => Some(0),
        RawCatalogInteriorFactV1::RetiredActionRoot {
            unaccepted_rows,
            retired_action_dirs,
            ..
        } if *unaccepted_rows == 0 && *retired_action_dirs <= MAX_RETIRED_ACTION_DIRS => {
            Some(*retired_action_dirs)
        }
        _ => None,
    }
}

fn any_present(interior: &RawCatalogInteriorObservationV1, slots: &[InfrastructureSlotV1]) -> bool {
    slots.iter().any(|slot| row(interior, *slot).is_some())
}

fn only_missing(
    interior: &RawCatalogInteriorObservationV1,
    slots: &[InfrastructureSlotV1],
) -> bool {
    !any_present(interior, slots)
}

fn later_after_roaming_missing(interior: &RawCatalogInteriorObservationV1) -> bool {
    only_missing(
        interior,
        &[
            InfrastructureSlotV1::CatalogAnchorA,
            InfrastructureSlotV1::CatalogAnchorB,
            InfrastructureSlotV1::RetiredActionsDescriptor,
            InfrastructureSlotV1::CatalogFormat,
        ],
    )
}

fn descriptor_and_format_missing(interior: &RawCatalogInteriorObservationV1) -> bool {
    only_missing(
        interior,
        &[
            InfrastructureSlotV1::RetiredActionsDescriptor,
            InfrastructureSlotV1::CatalogFormat,
        ],
    )
}

fn slot_index(slot: InfrastructureSlotV1) -> usize {
    InfrastructureSlotV1::ALL
        .iter()
        .position(|candidate| *candidate == slot)
        .expect("slot belongs to the closed infrastructure grammar")
}

pub(super) const fn roaming_anchor_bytes() -> &'static [u8] {
    ROAMING_ANCHOR_BYTES
}

pub(super) const fn catalog_anchor_bytes() -> &'static [u8] {
    CATALOG_ANCHOR_BYTES
}
