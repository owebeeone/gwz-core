use sha2::{Digest, Sha256};

use super::archive::CanonicalArchiveAcquisition;
use crate::workspace_ops::merge::model::archive_projection::{
    ArchiveSourceVersion, ArchivedMergeProjection,
};
use crate::workspace_ops::merge::model::v1::RecordVersion;
use crate::workspace_ops::merge::record_wire::{ArchivedCleanupWorklist, ValidatedArchivedRecord};

pub(super) struct ValidatedArchivedMerge {
    source_version: ArchiveSourceVersion,
    destination_bytes: Vec<u8>,
    destination_sha256: [u8; 32],
    decoded: ValidatedArchivedRecord,
}

impl ValidatedArchivedMerge {
    pub(super) fn from_acquisition(acquisition: CanonicalArchiveAcquisition) -> Self {
        let (destination_bytes, decoded) = acquisition.into_parts();
        let source_version = decoded.projection().source_version;
        let destination_sha256 = Sha256::digest(&destination_bytes).into();
        Self {
            source_version,
            destination_bytes,
            destination_sha256,
            decoded,
        }
    }

    pub(super) fn source_version(&self) -> RecordVersion {
        match self.source_version {
            ArchiveSourceVersion::V0 => RecordVersion::V0,
            ArchiveSourceVersion::V1 => RecordVersion::V1,
        }
    }

    pub(super) fn destination_bytes(&self) -> &[u8] {
        &self.destination_bytes
    }

    pub(super) fn destination_sha256(&self) -> [u8; 32] {
        self.destination_sha256
    }

    pub(super) fn projection(&self) -> &ArchivedMergeProjection {
        self.decoded.projection()
    }

    pub(super) fn cleanup(&self) -> &ArchivedCleanupWorklist {
        self.decoded.cleanup()
    }
}
