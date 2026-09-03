use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::artifact::read_lock;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::runtime::clock::{FixedClock, TimestampMs};
use crate::runtime::ids::SequentialIdProvider;
use crate::workspace_ops::merge::{
    FileMergeStore, MergeDependencies, MergeOperationRecord, MergeStore, OperationState,
    PublicationStep, handle_merge_with_dependencies,
};
use sha2::{Digest, Sha256};

use super::*;

mod a1_activation;
mod abort_recovery;
mod archive_equivalence_v0;
mod atomic_upgrade_v0;
mod characterization_archive_v0;
mod characterization_preservation_v0;
mod characterization_publication_prefix_v0;
mod characterization_publication_v0;
mod characterization_v0;
mod compatibility_residue_v0;
mod compatibility_unbound_v0;
mod compatibility_v0;
mod compatibility_v0_edges;
mod continue_merge;
mod continue_v0_gate;
mod crash_recovery;
mod drift;
mod engine_parity;
mod finalization;
mod fixtures;
mod gc;
mod m4_matrix;
mod open_operation_gate;
mod preserve;
mod root_abort;
mod root_drift;
mod root_recovery;
mod root_stage;
mod root_start;
mod root_status;
mod start;

use fixtures::*;
