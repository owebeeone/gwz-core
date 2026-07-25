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

mod abort_recovery;
mod continue_merge;
mod drift;
mod finalization;
mod fixtures;
mod open_operation_gate;
mod root_abort;
mod root_drift;
mod root_recovery;
mod root_stage;
mod root_start;
mod root_status;
mod start;

use fixtures::*;
