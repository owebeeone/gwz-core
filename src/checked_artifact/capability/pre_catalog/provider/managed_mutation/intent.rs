use super::super::directory_mutation::sync_directory_edge;
use super::super::namespace_mutation::RetainedActionNamespaceV1;
use crate::checked_artifact::capability::{AsciiComponent, CheckedFsError};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::ProtocolRecordKindV1;
use crate::filesystem::FsKind;
use std::io::{Seek, SeekFrom, Write};

use super::*;

/// Which generation edge of the managed intent record's durable lifecycle a call
/// belongs to.
///
/// R2-D Phase 3 Step 3.1b (`GwzM5-8R2D-Plan.md` §4 Step 3.1, "durable successor,
/// prior-generation retirement"; `GwzM5-8R2DInterfaceFreeze.md` §4.3 row E17).
/// It selects the stable `managed_bootstrap.*` boundaries and the diagnostic
/// label, exactly as `ActionNamespaceEdgeV1` selects the `namespace.*` ones for
/// the two edges that share one primitive call site.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ManagedIntentEdgeV1 {
    /// The row's first generation: the intent record that exists before any
    /// component is touched.
    Initial,
    /// Every later generation: the successor an evidence transition derives.
    Successor,
    /// The retirement of the generation a successor supersedes.
    PriorGeneration,
    /// The retirement of the last generation, which is the row's own completion
    /// record.
    FinalRetirement,
}

impl ManagedIntentEdgeV1 {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Initial => "publish initial managed intent",
            Self::Successor => "publish managed intent successor",
            Self::PriorGeneration => "retire prior managed intent generation",
            Self::FinalRetirement => "retire final managed intent",
        }
    }

    pub(crate) const fn scratch_label(self) -> &'static str {
        match self {
            Self::Initial => "write initial managed intent scratch",
            Self::Successor => "write managed intent successor scratch",
            Self::PriorGeneration | Self::FinalRetirement => {
                "write managed intent retirement scratch"
            }
        }
    }

    /// The create / write / flush boundaries of this edge's scratch record.
    /// Only the two publishing edges have one; a retirement moves a record that
    /// is already durable.
    #[cfg(test)]
    pub(crate) const fn scratch_faults(self) -> Option<[CheckedArtifactFaultKeyV1; 3]> {
        match self {
            Self::Initial => Some([
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentScratchCreate,
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentScratchWrite,
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentScratchFlush,
            ]),
            Self::Successor => Some([
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorScratchCreate,
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorScratchWrite,
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorScratchFlush,
            ]),
            Self::PriorGeneration | Self::FinalRetirement => None,
        }
    }

    /// The post-edge and post-observation boundaries this edge crosses.
    ///
    /// The first names the state "the namespace rename is durable and nothing
    /// has looked at it yet"; the second names "the published row has been read
    /// back and proved". The rename itself is the already-executed
    /// `namespace.publish_no_replace` / `namespace.retire_exact` boundary of
    /// Step 2.2, because this lifecycle deliberately routes through the
    /// role-validated backend rather than opening the sealed primitive again —
    /// so these two are the boundaries this family owns, in the same shape
    /// Step 2.3 used for `staging_directory_publish` and `marker_retire`.
    #[cfg(test)]
    pub(crate) const fn edge_faults(self) -> [CheckedArtifactFaultKeyV1; 2] {
        match self {
            Self::Initial => [
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentPublish,
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentReobserve,
            ],
            Self::Successor => [
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorPublish,
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorReobserve,
            ],
            Self::PriorGeneration => [
                CheckedArtifactFaultKeyV1::ManagedBootstrapPriorGenerationRetire,
                CheckedArtifactFaultKeyV1::ManagedBootstrapPriorGenerationReobserve,
            ],
            Self::FinalRetirement => [
                CheckedArtifactFaultKeyV1::ManagedBootstrapFinalIntentRetire,
                CheckedArtifactFaultKeyV1::ManagedBootstrapFinalIntentRetiredReobserve,
            ],
        }
    }
}

/// R2-D Phase 3 Step 3.1b — the managed intent record written into the action
/// directory's scheduled `BootstrapIntentScratch` row.
///
/// Primitive family P2 only (write-through plus flush); the record reaches its
/// active row through the Step-2.2 backend's `publish_bootstrap_generation`,
/// which is why this step adds no publication call site.
///
/// **Write-or-rewrite, for the same reason `stage_component` is.** The scratch
/// slot is one deterministic base slot shared by every generation of every row
/// of this action, and the bytes are re-derived identically on every drive, so a
/// leftover scratch from an interrupted generation is this drive's own residue
/// and must be converged on rather than wedged against. The row carries no
/// authority until the no-replace publish moves it, and the post-write proof
/// below re-reads the named row before any of that happens.
pub(in crate::checked_artifact) fn write_managed_intent_scratch(
    action: &RetainedActionNamespaceV1,
    scratch_leaf: &AsciiComponent,
    bytes: &[u8],
    edge: ManagedIntentEdgeV1,
) -> Result<(), CheckedFsError> {
    let directory = action.handle();
    let name = os_name(scratch_leaf);
    let create_new = match directory.entry_metadata(&name) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => true,
        Err(source) => return Err(CheckedFsError::io("observe managed intent scratch", source)),
        Ok(metadata) if metadata.kind == FsKind::File && metadata.kind != FsKind::Symlink => false,
        Ok(_) => {
            return Err(managed_error(
                "managed intent scratch row is not a canonical regular file",
            ));
        }
    };
    if bytes.len() > ProtocolRecordKindV1::BootstrapIntent.max_bytes() {
        return Err(managed_error(
            "managed intent record exceeds its frozen record bound",
        ));
    }
    let options = super::super::directory_mutation::durable_write_options(create_new);
    let mut file = directory
        .open_file(&name, &options)
        .map_err(|source| CheckedFsError::io("open managed intent scratch", source))?;
    #[cfg(test)]
    if let Some(faults) = edge.scratch_faults() {
        crate::checked_artifact::fault_v1::hit(faults[0]);
    }
    if !create_new {
        file.set_len(0)
            .map_err(|source| CheckedFsError::io("truncate managed intent scratch", source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind managed intent scratch", source))?;
    }
    file.write_all(bytes)
        .map_err(|source| CheckedFsError::io("write managed intent scratch", source))?;
    #[cfg(test)]
    if let Some(faults) = edge.scratch_faults() {
        crate::checked_artifact::fault_v1::hit(faults[1]);
    }
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush managed intent scratch", source))?;
    #[cfg(test)]
    if let Some(faults) = edge.scratch_faults() {
        crate::checked_artifact::fault_v1::hit(faults[2]);
    }
    drop(file);
    sync_directory_edge(directory, "flush managed intent scratch row")?;
    let observed = observe_regular_file(
        directory,
        &name,
        edge.scratch_label(),
        ProtocolRecordKindV1::BootstrapIntent,
    )?;
    if observed.bytes != bytes {
        return Err(managed_error(
            "managed intent scratch is not the record just written",
        ));
    }
    // Only the successor half of the frozen vocabulary carries a scratch
    // reobservation key; the initial half's post-write proof runs identically and
    // announces nothing, because minting `initial_intent_scratch_reobserve` would
    // move the frozen 165-key census (§3.5, §6).
    #[cfg(test)]
    if edge == ManagedIntentEdgeV1::Successor {
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorScratchReobserve,
        );
    }
    Ok(())
}

/// R2-D Phase 3 Step 3.1b — the post-edge proof of one scheduled managed intent
/// row, and the two boundaries around it.
///
/// The caller has just moved the row through the Step-2.2 backend; this reads it
/// back bounded and returns its bytes, so the record a drive resumes from is
/// durable truth rather than the caller's own expectation.
pub(in crate::checked_artifact) fn observe_managed_intent_row(
    action: &RetainedActionNamespaceV1,
    leaf: &AsciiComponent,
    edge: ManagedIntentEdgeV1,
) -> Result<Vec<u8>, CheckedFsError> {
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(edge.edge_faults()[0]);
    let observed = read_managed_intent_row(action, leaf, edge.label())?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(edge.edge_faults()[1]);
    Ok(observed)
}

/// R2-D Phase 3 Step 3.1b — a bounded read of one resident managed intent row.
///
/// This is the resume's own read. It announces no boundary: it crosses no
/// durable edge, and every row it reads already carries the keyed boundaries of
/// the edge that put it there.
pub(in crate::checked_artifact) fn read_managed_intent_row(
    action: &RetainedActionNamespaceV1,
    leaf: &AsciiComponent,
    label: &'static str,
) -> Result<Vec<u8>, CheckedFsError> {
    Ok(observe_regular_file(
        action.handle(),
        &os_name(leaf),
        label,
        ProtocolRecordKindV1::BootstrapIntent,
    )?
    .bytes)
}
