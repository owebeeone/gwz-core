//! Contract §2 — version selection and the writer floor.
//!
//! `GwzM5-8I2CompatibilityContract.md` §2 freezes one pure function:
//!
//! ```text
//! max(active_writer_floor, highest_requested_semantic_version)
//! ```
//!
//! The core derives the semantic version from immutable typed request intent;
//! drivers never choose it. Unsupported requested semantics reject *before*
//! record creation, so no record is ever created at a version this binary
//! cannot read back. The chosen version is frozen before the first mutation,
//! existing v1+ records never change version, and open v0 records may use
//! only the explicit A1 migration path (`record_wire::open_v0`).

use super::v1::RecordVersion;
use crate::model::{ErrorCode, ModelError, ModelResult};

/// The version floor this binary writes at.
///
/// **M5d: v1 is the only merge.** Contract §2's creation matrix is `v1` for
/// ordinary, `--ff-only`, custom-message and `--no-ff` starts alike, and
/// `GwzM5-8M5d-Charter.md` §1 makes that the shipped fact: "One merge
/// implementation: **v1**. … The v0 start / continue / abort / migrate /
/// open-body / `adapt` **lifecycle engine** is **not in the binary**."
///
/// Selection is `max(floor, requested)`, so with the floor at `V1` every
/// `RequestedSemantics` that this binary can serve selects `V1` and the
/// version fork in `merge/start.rs` has one live arm. Lowering this constant
/// does not restore a v0 writer — the engine that consumed v0 records is
/// deleted, and an open v0 envelope is refused by `merge/open_v0_refusal.rs`
/// without a body decode (charter §2). `RecordVersion::V0` survives only for
/// the read-only archive projection of pre-0.14 `done/` records (charter §5).
pub(crate) const ACTIVE_WRITER_FLOOR: RecordVersion = RecordVersion::V1;

/// The semantic version a request's typed intent requires.
///
/// A1 installs v0 and v1 body decoders. v2 (branch lifecycle), v3 (snapshot
/// source), and v4 (partial composition) are allocated in the envelope
/// registry but have no body type here, so a request naming them is refused
/// before creation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "The closed enum names every allocated semantic of the frozen envelope registry (compatibility contract §1/§2), including the allocated-but-uninstalled waves. A2-A4 construct these; A1 only rejects them before record creation, so nothing constructs them here."
)]
pub(crate) enum RequestedSemantics {
    /// Ordinary, `--ff-only`, and custom-message starts.
    Ordinary,
    /// `--no-ff`: the v1 record lifecycle's two-parent integration. A1's
    /// public surface, and v1 semantics by construction.
    NoFf,
    /// A2/M6 branch lifecycle.
    BranchLifecycle,
    /// A3/M7 snapshot source.
    SnapshotSource,
    /// A4/M8 partial composition.
    PartialComposition,
}

impl RequestedSemantics {
    /// The request's own semantic version, or the wave that would install it.
    fn semantic_version(self) -> Result<RecordVersion, crate::MergeRecordRequiredWave> {
        match self {
            Self::Ordinary => Ok(RecordVersion::V0),
            Self::NoFf => Ok(RecordVersion::V1),
            Self::BranchLifecycle => Err(crate::MergeRecordRequiredWave::A2),
            Self::SnapshotSource => Err(crate::MergeRecordRequiredWave::A3),
            Self::PartialComposition => Err(crate::MergeRecordRequiredWave::A4),
        }
    }

    /// Derive the requested semantics from one accepted plan's typed intent.
    /// `--ff-only` is an ordinary integration constraint, not a separate
    /// semantic.
    pub(crate) fn from_mode(mode: super::MergeExecutionMode) -> Self {
        match mode {
            super::MergeExecutionMode::NoFf => Self::NoFf,
            super::MergeExecutionMode::Normal | super::MergeExecutionMode::FfOnly => Self::Ordinary,
        }
    }
}

/// Contract §2's pure selection function, plus its rejection arm.
pub(crate) fn select_record_version(requested: RequestedSemantics) -> ModelResult<RecordVersion> {
    match requested.semantic_version() {
        Ok(version) => Ok(ACTIVE_WRITER_FLOOR.max(version)),
        Err(wave) => Err(ModelError::new(
            ErrorCode::UnsupportedRecordVersion,
            format!(
                "the requested merge semantics require {}; use a compatible newer GWZ",
                wave_display(wave)
            ),
        )),
    }
}

fn wave_display(wave: crate::MergeRecordRequiredWave) -> &'static str {
    match wave {
        crate::MergeRecordRequiredWave::A1 => "A1 (v1 integration/acceptance/no-ff)",
        crate::MergeRecordRequiredWave::A2 => "A2 (v2 branch lifecycle)",
        crate::MergeRecordRequiredWave::A3 => "A3 (v3 snapshot source)",
        crate::MergeRecordRequiredWave::A4 => "A4 (v4 partial composition)",
    }
}

/// The exact `(schema, record_schema_version)` envelope a created record
/// carries at `version`.
pub(crate) fn creation_envelope(version: RecordVersion) -> (&'static str, u32) {
    match version {
        RecordVersion::V0 => (
            super::common::MERGE_RECORD_SCHEMA,
            super::common::MERGE_RECORD_SCHEMA_VERSION,
        ),
        RecordVersion::V1 => (
            super::v1::MERGE_RECORD_SCHEMA_V1,
            super::v1::MERGE_RECORD_SCHEMA_VERSION_V1,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--no-ff` selects v1 through the `max`, which is what makes the
    /// activated public surface write a v1 record. Ordinary starts follow the
    /// floor, which is `V0` while the ordinary-start v1 owner is absent (see
    /// `ACTIVE_WRITER_FLOOR`'s partial-engagement note).
    #[test]
    fn no_ff_selects_v1_and_ordinary_follows_the_active_floor() {
        assert_eq!(
            select_record_version(RequestedSemantics::NoFf).unwrap(),
            RecordVersion::V1
        );
        assert_eq!(
            select_record_version(RequestedSemantics::Ordinary).unwrap(),
            ACTIVE_WRITER_FLOOR
        );
        assert_eq!(
            RequestedSemantics::from_mode(super::super::MergeExecutionMode::NoFf),
            RequestedSemantics::NoFf
        );
        for mode in [
            super::super::MergeExecutionMode::Normal,
            super::super::MergeExecutionMode::FfOnly,
        ] {
            assert_eq!(
                RequestedSemantics::from_mode(mode),
                RequestedSemantics::Ordinary
            );
        }
    }

    /// The selection is a true `max`: raising the floor to A1's own value
    /// makes every installed semantic v1 without touching the requested side.
    #[test]
    fn raising_the_floor_to_v1_makes_every_installed_semantic_v1() {
        for requested in [RequestedSemantics::Ordinary, RequestedSemantics::NoFf] {
            let version = requested.semantic_version().unwrap();
            assert_eq!(RecordVersion::V1.max(version), RecordVersion::V1);
        }
    }

    /// v2-v4 reject before creation with their own required wave.
    #[test]
    fn unsupported_requested_semantics_reject_before_record_creation() {
        for (requested, wave) in [
            (RequestedSemantics::BranchLifecycle, "A2"),
            (RequestedSemantics::SnapshotSource, "A3"),
            (RequestedSemantics::PartialComposition, "A4"),
        ] {
            let error = select_record_version(requested).unwrap_err();
            assert_eq!(error.code, ErrorCode::UnsupportedRecordVersion);
            assert!(error.message.contains(wave), "{}", error.message);
        }
    }

    #[test]
    fn the_selected_version_names_its_exact_envelope_pair() {
        assert_eq!(
            creation_envelope(RecordVersion::V1),
            ("gwz.merge-operation/v1", 1)
        );
        assert_eq!(
            creation_envelope(RecordVersion::V0),
            ("gwz.merge-operation/v0", 0)
        );
    }
}
