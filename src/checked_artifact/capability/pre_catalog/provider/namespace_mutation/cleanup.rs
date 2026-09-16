use super::super::directory_mutation::sync_directory_edge;
use crate::checked_artifact::capability::{AsciiComponent, CheckedFsError};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, BoundCleanupWorklistV1, CleanupAliasV1, CleanupPhysicalFactV1,
    CleanupResolutionV1, DurableLeafFingerprintV1, ProtocolRecordKindV1,
    read_and_bind_cleanup_worklist,
};
use crate::filesystem::FsKind;
use sha2::{Digest, Sha256};
use std::io::{self, Seek, SeekFrom, Write};

use super::*;

/// Chunk size for the cleanup alias digest stream. A refusal threshold's
/// working buffer, not durable vocabulary: it fixes the memory the digest
/// costs, never what the digest is taken over.
pub(crate) const CLEANUP_ALIAS_STREAM_CHUNK: usize = 8192;

/// R2-E E1.1 — the cleanup worklist's scheduled scratch row (keys
/// `cleanup.worklist_scratch_create` / `_write` / `_flush`).
///
/// DECISION C-3, as simplified at E0.2b §4: the row is the shared
/// `BaseActionSlotV1::RecordScratch` slot (`protocol/slots.rs:106`/`:140`), an
/// entirely unconsumed base slot whose first and only user is this worklist. No
/// slot is minted, and there is no ordering condition — OPEN-C1 was struck when
/// the tree answered it: the authority record's own scratch is
/// `BaseActionSlotV1::AuthorityScratch` (`authority_record_binding.rs:486`), and
/// the `record.scratch_*` keys are named after the record *family*, not a slot.
///
/// **Write-or-rewrite, for the reason `write_managed_intent_scratch` is**
/// (`managed_mutation.rs:1058-1064`, `:1073-1101`). The bytes are re-derived
/// identically on every drive, so a leftover scratch from an interrupted drive
/// is this drive's own residue and must be converged on rather than wedged
/// against. The row carries no authority until the no-replace publish moves it,
/// and the post-write proof below re-reads the named row before any of that
/// happens.
pub(in crate::checked_artifact) fn write_cleanup_worklist_scratch(
    action: &RetainedActionNamespaceV1,
    scratch_leaf: &AsciiComponent,
    bytes: &[u8],
) -> Result<(), CheckedFsError> {
    let directory = &action.handle;
    let name = os_name(scratch_leaf);
    let create_new = match directory.entry_metadata(&name) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => true,
        Err(source) => {
            return Err(CheckedFsError::io(
                "observe cleanup worklist scratch",
                source,
            ));
        }
        Ok(metadata) if metadata.kind == FsKind::File && metadata.kind != FsKind::Symlink => false,
        Ok(_) => {
            return Err(cleanup_error(
                "cleanup worklist scratch row is not a canonical regular file",
            ));
        }
    };
    if bytes.len() > ProtocolRecordKindV1::CleanupWorklist.max_bytes() {
        return Err(cleanup_error(
            "cleanup worklist record exceeds its frozen record bound",
        ));
    }
    let options = super::super::directory_mutation::durable_write_options(create_new);
    let mut file = directory
        .open_file(&name, &options)
        .map_err(|source| CheckedFsError::io("open cleanup worklist scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistScratchCreate);
    if !create_new {
        file.set_len(0)
            .map_err(|source| CheckedFsError::io("truncate cleanup worklist scratch", source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind cleanup worklist scratch", source))?;
    }
    file.write_all(bytes)
        .map_err(|source| CheckedFsError::io("write cleanup worklist scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistScratchWrite);
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush cleanup worklist scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistScratchFlush);
    drop(file);
    sync_directory_edge(directory, "flush cleanup worklist scratch row")?;
    let observed = action.observe_regular_file(
        scratch_leaf,
        ProtocolRecordKindV1::CleanupWorklist,
        "cleanup worklist scratch",
    )?;
    if observed.bytes != bytes {
        return Err(cleanup_error(
            "cleanup worklist scratch is not the record just written",
        ));
    }
    Ok(())
}

/// R2-E E1.1 — the published worklist row and its two boundaries (keys
/// `cleanup.worklist_publish` / `cleanup.worklist_reobserve`).
///
/// The caller has just moved the scratch row onto `BaseActionSlotV1::CleanupWorklist`
/// through the Step-2.2 backend; the first boundary names that rename as durable
/// and unread, and the second names the bounded re-read that proves it — which
/// `read_and_bind_cleanup_worklist` (`protocol/cleanup.rs:356-367`) additionally
/// refuses when the worklist does not match the resident reservation.
pub(in crate::checked_artifact) fn observe_cleanup_worklist_row(
    action: &RetainedActionNamespaceV1,
    worklist_leaf: &AsciiComponent,
    reservation: &ActionCapacityReservationV1,
) -> Result<BoundCleanupWorklistV1, CheckedFsError> {
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistPublish);
    let bound = read_cleanup_worklist(action, worklist_leaf, reservation)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistReobserve);
    Ok(bound)
}

// ---------------------------------------------------------------------------
// R2-E Phase E1 Step E1.1 — the `cleanup.*` family's eleven boundaries.
//
// DECISION C-1 (amendment §2.2): the two renames this family drives — the
// worklist publish and each alias retirement — keep announcing the already
// executed `namespace.publish_no_replace` / `namespace.retire_exact` boundaries
// of Step 2.2, because the family routes through the role-validated backend
// rather than opening the sealed primitive again. `cleanup.worklist_publish`
// and `cleanup.alias_retire` therefore name the *post-edge* states, in the same
// shape `managed_bootstrap.prior_generation_retire` does
// (`managed_mutation.rs:1039-1042`).
// ---------------------------------------------------------------------------

/// R2-E E1.1 — one worklist row's `(source, destination)` physical fact pair
/// (keys `cleanup.source_reobserve` / `cleanup.destination_reobserve`).
///
/// The pair is exactly what `BoundCleanupWorklistV1::classify`
/// (`protocol/cleanup.rs:323-333`) consumes, so a restart resolves each row from
/// durable truth rather than from the caller's own expectation.
pub(in crate::checked_artifact) fn observe_cleanup_row_facts(
    action: &RetainedActionNamespaceV1,
    source_leaf: &AsciiComponent,
    destination_leaf: &AsciiComponent,
) -> Result<(CleanupPhysicalFactV1, CleanupPhysicalFactV1), CheckedFsError> {
    let source = observe_cleanup_alias(action, source_leaf)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupSourceReobserve);
    let destination = observe_cleanup_alias(action, destination_leaf)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupDestinationReobserve);
    Ok((source, destination))
}

/// R2-E E1.1 — one alias retirement's three post-edge boundaries (keys
/// `cleanup.alias_retire`, `cleanup.retired_alias_reobserve`,
/// `cleanup.row_complete`).
///
/// The rename itself is Step 2.2's `namespace.retire_exact` (DECISION C-1); the
/// first boundary here names the state it leaves. The flush is the resident twin
/// of the unannounced `sync_directory_edge` at the tail of [`RetainedActionNamespaceV1::execute_edge`]:
/// re-flushing an already-flushed directory is idempotent, and it is what gives
/// this family its own announced completion edge without opening the sealed
/// primitive or minting a key.
pub(in crate::checked_artifact) fn observe_cleanup_retirement(
    action: &RetainedActionNamespaceV1,
    destination_leaf: &AsciiComponent,
    expected: &DurableLeafFingerprintV1,
) -> Result<(), CheckedFsError> {
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupAliasRetire);
    let retired = observe_cleanup_alias(action, destination_leaf)?;
    if retired != CleanupPhysicalFactV1::Exact(expected.clone()) {
        return Err(cleanup_error(
            "retired cleanup alias is not the fingerprint its worklist row named",
        ));
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupRetiredAliasReobserve);
    sync_directory_edge(&action.handle, "flush cleanup row completion")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupRowComplete);
    Ok(())
}

/// R2-E E1.1 — the whole-worklist proof (key `cleanup.completion_reobserve`).
///
/// The worklist is re-read bounded and every row must classify
/// `CleanupResolutionV1::Complete` (`protocol/cleanup.rs:394-398`). This is the
/// state `terminal.cleanup_reobserve` consumes.
///
/// `rows` carries each reserved alias with the two schedule-derived leaves it
/// resolves between, because this file mints no name; the aliases are matched by
/// value rather than by position, so the worklist's own canonical row order is
/// what selects them.
pub(in crate::checked_artifact) fn observe_cleanup_completion(
    action: &RetainedActionNamespaceV1,
    worklist_leaf: &AsciiComponent,
    reservation: &ActionCapacityReservationV1,
    rows: &[(CleanupAliasV1, AsciiComponent, AsciiComponent)],
) -> Result<(), CheckedFsError> {
    let bound = read_cleanup_worklist(action, worklist_leaf, reservation)?;
    for index in 0..bound.len() {
        let row = bound
            .row(index)
            .ok_or_else(|| cleanup_error("cleanup worklist row is not bound"))?;
        let Some((_, source_leaf, destination_leaf)) =
            rows.iter().find(|(alias, _, _)| *alias == row.alias())
        else {
            return Err(cleanup_error(
                "cleanup completion was not given this row's scheduled leaves",
            ));
        };
        let source = observe_cleanup_alias(action, source_leaf)?;
        let destination = observe_cleanup_alias(action, destination_leaf)?;
        if bound.classify(index, &source, &destination) != Some(CleanupResolutionV1::Complete) {
            return Err(cleanup_error(
                "a cleanup worklist row is not durably complete",
            ));
        }
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupCompletionReobserve);
    Ok(())
}

/// A bounded read of the resident worklist row, bound to the resident
/// reservation. It announces no boundary of its own: the announced pair is
/// [`observe_cleanup_worklist_row`]'s, and the completion proof's re-read is the
/// resume's own read — exactly as `read_managed_intent_row` is
/// (`managed_mutation.rs:1159-1163`).
pub(crate) fn read_cleanup_worklist(
    action: &RetainedActionNamespaceV1,
    worklist_leaf: &AsciiComponent,
    reservation: &ActionCapacityReservationV1,
) -> Result<BoundCleanupWorklistV1, CheckedFsError> {
    let observed = action.observe_regular_file(
        worklist_leaf,
        ProtocolRecordKindV1::CleanupWorklist,
        "cleanup worklist row",
    )?;
    read_and_bind_cleanup_worklist(observed.bytes.as_slice(), reservation).map_err(|_| {
        cleanup_error("cleanup worklist row does not bind to the resident reservation")
    })
}

/// One cleanup alias row's physical fact — absent, exactly fingerprinted, or
/// anything else — in the three-way shape `classify_cleanup_row`
/// (`protocol/cleanup.rs:383-401`) resolves.
///
/// The row is read under the **same** stated bound its retirement is read under
/// — this family's own `CleanupWorklist` record bound, whose statement of record
/// is `CleanupRetirementDestination::source_bound` (`namespace/roles.rs`). Using
/// one bound for both is what keeps the two halves coherent: a row this family
/// could not retire through `execute_edge` is refused when it is observed rather
/// than fingerprinted as though it could be.
///
/// It announces no boundary: the announced observations are
/// [`observe_cleanup_row_facts`]'s, and the completion proof's re-reads are the
/// resume's own reads.
pub(crate) fn observe_cleanup_alias(
    action: &RetainedActionNamespaceV1,
    leaf: &AsciiComponent,
) -> Result<CleanupPhysicalFactV1, CheckedFsError> {
    match action.handle.entry_metadata(os_name(leaf)) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(CleanupPhysicalFactV1::Missing);
        }
        Err(source) => return Err(CheckedFsError::io("observe cleanup alias", source)),
        Ok(metadata) if metadata.kind == FsKind::File && metadata.kind != FsKind::Symlink => {}
        Ok(_) => return Ok(CleanupPhysicalFactV1::Other),
    }
    let observed = action.observe_regular_file(
        leaf,
        ProtocolRecordKindV1::CleanupWorklist,
        "cleanup alias",
    )?;
    Ok(CleanupPhysicalFactV1::Exact(DurableLeafFingerprintV1::new(
        observed.identity,
        observed.bytes.len() as u64,
        Sha256::digest(&observed.bytes).into(),
    )))
}
